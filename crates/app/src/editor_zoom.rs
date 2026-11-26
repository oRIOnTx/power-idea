//! Zooming the code editors: ⌘ + mouse wheel, trackpad pinch and the font size actions.
//! The zoom lasts until quit and leaves the configured font size alone.

use gpui_kit::*;

use crate::config::AppConfig;

actions!(
    ide,
    [
        IncreaseEditorFontSize,
        DecreaseEditorFontSize,
        ResetEditorFontSize
    ]
);

const MIN_SIZE: f32 = 6.;
const MAX_SIZE: f32 = 72.;
/// One action or one wheel notch, in points.
const STEP: f32 = 1.;
/// Trackpad scrolling reports pixels; this many make one point.
const PIXELS_PER_POINT: f32 = 12.;

/// The zoomed editor font size, if it differs from the configured one.
#[derive(Default)]
struct EditorZoom(Option<f32>);

impl Global for EditorZoom {}

/// The font size of code editors.
pub fn font_size(cx: &App) -> Pixels {
    px(current(cx))
}

fn current(cx: &App) -> f32 {
    cx.try_global::<EditorZoom>()
        .and_then(|zoom| zoom.0)
        .unwrap_or(cx.global::<AppConfig>().0.editor_font_size)
}

fn clamp(size: f32) -> f32 {
    size.clamp(MIN_SIZE, MAX_SIZE)
}

/// The size after a ⌘-scroll by `delta` (positive scrolls up, which zooms in).
fn scrolled(size: f32, delta: &ScrollDelta) -> f32 {
    let points = match delta {
        ScrollDelta::Lines(lines) => lines.y.signum() * STEP,
        ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / PIXELS_PER_POINT,
    };
    clamp(size + points)
}

/// The size after a pinch by `delta` (0.1 is 10% larger).
fn pinched(size: f32, delta: f32) -> f32 {
    clamp(size * (1. + delta))
}

fn set(size: f32, cx: &mut App) {
    let configured = cx.global::<AppConfig>().0.editor_font_size;
    let zoom = ((size - configured).abs() > f32::EPSILON).then_some(size);
    cx.set_global(EditorZoom(zoom));
    cx.refresh_windows();
}

/// Registers the font size actions; call once at startup.
pub fn register(cx: &mut App) {
    cx.on_action(|_: &IncreaseEditorFontSize, cx| set(clamp(current(cx) + STEP), cx));
    cx.on_action(|_: &DecreaseEditorFontSize, cx| set(clamp(current(cx) - STEP), cx));
    cx.on_action(|_: &ResetEditorFontSize, cx| {
        cx.set_global(EditorZoom(None));
        cx.refresh_windows();
    });
}

/// An overlay for an editor's area that turns ⌘ + wheel and pinch over it into zoom. It
/// listens in the capture phase, so the editor does not scroll while zooming.
pub fn gesture_layer() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, (), window, _| {
            window.on_mouse_event(move |event: &ScrollEvent, phase, _, cx| {
                let zooming = event.modifiers.platform && bounds.contains(&event.position);
                if phase == DispatchPhase::Capture && zooming {
                    set(scrolled(current(cx), &event.delta), cx);
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |event: &PinchEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && bounds.contains(&event.position) {
                    set(pinched(current(cx), event.delta), cx);
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_full()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px};

    use super::{MAX_SIZE, MIN_SIZE, ScrollDelta, pinched, scrolled};

    #[test]
    fn wheel_and_pinch_change_the_size_within_limits() {
        assert_eq!(scrolled(13., &ScrollDelta::Lines(point(0., 3.))), 14.);
        assert_eq!(scrolled(13., &ScrollDelta::Lines(point(0., -1.))), 12.);
        assert_eq!(scrolled(13., &ScrollDelta::Pixels(point(px(0.), px(24.)))), 15.);
        assert_eq!(scrolled(MAX_SIZE, &ScrollDelta::Lines(point(0., 1.))), MAX_SIZE);
        assert_eq!(pinched(10., 0.5), 15.);
        assert_eq!(pinched(MIN_SIZE, -0.5), MIN_SIZE);
    }
}
