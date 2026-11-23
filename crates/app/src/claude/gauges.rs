//! Live gauges: the bridge socket server (hook and status-line messages from `claude` sessions
//! the IDE started), per-project gauge state, and the ring widgets in the status bar, the
//! Claude tab header and the sidebar. When no status line arrives, tokens and cost come from
//! the session transcript, parsed by the Insights parser.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use claude_integration::bridge::{Kind, Message};
use claude_integration::gauges::{Band, GaugeState, band};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use super::state;
use crate::inbox::Inbox;

/// Gauge state per project root (the session tag), and the values currently drawn.
#[derive(Default)]
pub struct ClaudeGauges {
    pub sessions: HashMap<String, GaugeState>,
    /// Animated ring values (tag, gauge) → shown value, eased toward the real one.
    shown: HashMap<(String, &'static str), f64>,
}

impl Global for ClaudeGauges {}

/// Starts the socket server; messages are applied on the UI thread in batches (one redraw each).
pub fn start(cx: &mut App) {
    cx.set_global(ClaudeGauges::default());
    let socket = super::bridge_socket();
    // A socket file left by a previous run would make bind fail.
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(err) => return eprintln!("Claude bridge socket {}: {err}", socket.display()),
    };
    let inbox: Inbox<Message> = Inbox::default();
    let sender = inbox.clone();
    let _ = std::thread::Builder::new()
        .name("claude-bridge".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let reader = std::io::BufReader::new(stream);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(message) = Message::decode(&line) {
                        sender.push(message);
                    }
                }
            }
        });
    cx.spawn(async move |cx| {
        while let Some(batch) = inbox.next_batch().await {
            cx.update(|cx| {
                for message in batch {
                    apply(message, cx);
                }
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

fn apply(message: Message, cx: &mut App) {
    let root = PathBuf::from(&message.tag);
    let gauges = cx.global_mut::<ClaudeGauges>();
    let session = gauges.sessions.entry(message.tag.clone()).or_default();
    match message.kind {
        Kind::Status => session.apply_status(&message.payload),
        Kind::Hook => {
            let edited = session.apply_hook(&message.payload, &root);
            let event = message
                .payload
                .get("hook_event_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let transcript = message
                .payload
                .get("transcript_path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);
            if event == "Stop"
                && !session.has_status
                && let Some(path) = transcript
            {
                fill_from_transcript(message.tag.clone(), path, cx);
            }
            if edited {
                super::quality::on_claude_edit(&root, cx);
            }
        }
    }
}

/// Tokens and cost from the transcript when the status line is not available (Insights parser).
fn fill_from_transcript(tag: String, path: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        let session = cx
            .background_spawn(async move {
                let text = std::fs::read_to_string(&path).ok()?;
                insights::parse::parse_session(text.lines().map(str::to_string))
            })
            .await;
        let Some(session) = session else { return };
        cx.update(|cx| {
            if let Some(state) = cx.global_mut::<ClaudeGauges>().sessions.get_mut(&tag) {
                state.input_tokens =
                    session.input_tokens + session.cache_read_tokens + session.cache_write_tokens;
                state.output_tokens = session.output_tokens;
                if session.cost_usd > 0.0 {
                    state.cost_usd = Some(session.cost_usd);
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Where the gauges are drawn: sizes and how much detail.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    StatusBar,
    TabHeader,
    Sidebar,
}

/// The gauges for `project_root` at `place`, or `None` when switched off or no session ran.
pub fn render(project_root: &Path, place: Place, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let settings = state(cx).settings.clone();
    let gauges = &settings.gauges;
    let wanted = match place {
        Place::StatusBar => gauges.in_status_bar,
        Place::TabHeader => gauges.in_tab_header,
        Place::Sidebar => gauges.in_sidebar,
    };
    if !settings.enabled || !gauges.enabled || !wanted {
        return None;
    }
    let tag = project_root.to_string_lossy().into_owned();
    let session = cx.try_global::<ClaudeGauges>()?.sessions.get(&tag)?.clone();
    let theme = cx.theme().clone();
    let band_color = |b: Band| match b {
        Band::Good => theme.green,
        Band::Warn => theme.warning,
        Band::Bad => theme.danger,
    };
    let ring_size = if place == Place::Sidebar { px(28.) } else { px(14.) };
    let mut items: Vec<AnyElement> = Vec::new();

    if gauges.context
        && let Some(percent) = session.context_percent
    {
        let shown = ease(&tag, "context", percent, window, cx);
        let color = band_color(band(percent, gauges.context_amber, gauges.context_red));
        items.push(labeled(
            ring(shown, color, theme.border, ring_size),
            format!("ctx {percent:.0}%"),
            "Context window used",
        ));
    }
    if gauges.cost && place != Place::TabHeader && (session.total_tokens() > 0 || session.cost_usd.is_some())
    {
        let cost = session.cost_usd.map_or_default(|c| format!(" · ${c:.2}"));
        items.push(text(
            format!("{}{cost}", tokens(session.total_tokens())),
            "Tokens and cost of the session",
        ));
    }
    if gauges.activity && place != Place::StatusBar {
        items.push(text(session.activity.label(), "What Claude is doing"));
    }
    if gauges.files && !session.files_changed.is_empty() {
        let count = session.files_changed.len();
        items.push(text(
            format!("{count} file{}", if count == 1 { "" } else { "s" }),
            "Files changed in this session",
        ));
    }
    if gauges.quality
        && settings.quality.enabled
        && let Some(quality) = session.quality
    {
        let shown = ease(&tag, "quality", quality.score, window, cx);
        // Score is good when high: reuse the bands on "100 − score".
        let color = band_color(band(100.0 - quality.score, 25, 50));
        let delta = session
            .quality_delta()
            .map_or_default(|(e, w)| format!(" ({e:+}E {w:+}W)"));
        items.push(labeled(
            ring(shown, color, theme.border, ring_size),
            format!("Q {:.0}{delta}", quality.score),
            "Code quality score",
        ));
    }
    if items.is_empty() {
        return None;
    }
    let row = h_flex().gap_2().items_center().children(items);
    Some(match place {
        Place::Sidebar => v_flex()
            .gap_1()
            .p_2()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("Claude · {}", session.activity.label())),
            )
            .child(row.flex_wrap())
            .into_any_element(),
        _ => row.into_any_element(),
    })
}

/// Eases the drawn value toward `target`, asking for another frame until it gets there.
fn ease(tag: &str, gauge: &'static str, target: f64, window: &mut Window, cx: &mut App) -> f64 {
    let shown = cx
        .global_mut::<ClaudeGauges>()
        .shown
        .entry((tag.to_string(), gauge))
        .or_insert(target);
    let next = *shown + (target - *shown) * 0.25;
    *shown = if (target - next).abs() < 0.5 { target } else { next };
    if (*shown - target).abs() > f64::EPSILON {
        window.request_animation_frame();
    }
    *shown
}

fn tokens(count: u64) -> String {
    match count {
        0..1_000 => format!("{count} tok"),
        1_000..1_000_000 => format!("{:.1}k tok", count as f64 / 1_000.0),
        _ => format!("{:.2}M tok", count as f64 / 1_000_000.0),
    }
}

fn text(value: String, tooltip: &'static str) -> AnyElement {
    div()
        .id(SharedString::from(format!("gauge-{tooltip}")))
        .whitespace_nowrap()
        .child(value)
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .into_any_element()
}

/// A ring with its value as text next to it (never color alone).
fn labeled(ring: impl IntoElement, value: String, tooltip: &'static str) -> AnyElement {
    h_flex()
        .id(SharedString::from(format!("gauge-{tooltip}")))
        .gap_1()
        .items_center()
        .child(ring)
        .child(div().whitespace_nowrap().child(value))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .into_any_element()
}

/// A circular progress ring: `percent` of the circle in `color` over a `track` circle.
fn ring(percent: f64, color: Hsla, track: Hsla, size: Pixels) -> impl IntoElement {
    let fraction = (percent / 100.0).clamp(0.0, 1.0) as f32;
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let stroke = (f32::from(size) / 7.0).max(2.0);
            let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - stroke / 2.0;
            let center = bounds.center();
            if let Some(path) = arc(center, radius, stroke, 1.0) {
                window.paint_path(path, track);
            }
            if fraction > 0.0
                && let Some(path) = arc(center, radius, stroke, fraction)
            {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
}

/// A stroked arc from 12 o'clock, clockwise, over `fraction` of the circle (line segments).
fn arc(center: Point<Pixels>, radius: f32, stroke: f32, fraction: f32) -> Option<gpui_kit::Path<Pixels>> {
    let steps = ((48.0 * fraction).ceil() as usize).max(2);
    let mut builder = PathBuilder::stroke(px(stroke));
    for step in 0..=steps {
        let angle =
            std::f32::consts::TAU * fraction * step as f32 / steps as f32 - std::f32::consts::FRAC_PI_2;
        let point = point(
            center.x + px(radius * angle.cos()),
            center.y + px(radius * angle.sin()),
        );
        if step == 0 {
            builder.move_to(point);
        } else {
            builder.line_to(point);
        }
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn formats_token_counts() {
        assert_eq!(tokens(950), "950 tok");
        assert_eq!(tokens(12_345), "12.3k tok");
        assert_eq!(tokens(2_500_000), "2.50M tok");
    }
}
