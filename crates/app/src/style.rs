//! Shared visual accents of the "Neo" look: restrained glow and hairlines.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Soft neon halo around an accent element.
pub fn glow(color: Hsla, strength: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.opacity(0.45 * strength),
        offset: point(px(0.), px(0.)),
        blur_radius: px(10. * strength),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// A short glowing dot, used as the brand mark and status indicator.
pub fn signal_dot(color: Hsla) -> Div {
    div()
        .size(px(6.))
        .rounded_full()
        .bg(color)
        .shadow(glow(color, 0.8))
}

/// `↑2 ↓1` next to a branch: commits to push (green) and to pull (yellow); `None` when in sync.
pub fn tracking_arrows(ahead: u32, behind: u32, cx: &App) -> Option<Div> {
    if ahead == 0 && behind == 0 {
        return None;
    }
    let theme = gpui_kit::component::ActiveTheme::theme(cx);
    let arrow = |text: String, color: Hsla| div().text_color(color).child(text);
    Some(
        div()
            .flex()
            .flex_none()
            .gap_1()
            .text_xs()
            .when(ahead > 0, |el| el.child(arrow(format!("↑{ahead}"), theme.green)))
            .when(behind > 0, |el| {
                el.child(arrow(format!("↓{behind}"), theme.yellow))
            }),
    )
}

/// Horizontal hairline that fades out at both ends.
pub fn fade_line(color: Hsla, width: Pixels) -> Div {
    let half = |from: Hsla, to: Hsla| {
        div().flex_1().h_full().bg(linear_gradient(
            90.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 1.),
        ))
    };
    div()
        .flex()
        .w(width)
        .h(px(1.))
        .child(half(color.opacity(0.), color))
        .child(half(color, color.opacity(0.)))
}

/// Button sizes; gpui-kit draws the icon at 75% of the button size.
pub const BUTTON_SMALL: Pixels = px(22.);
pub const BUTTON_TITLE_BAR: Pixels = px(26.);
pub const BUTTON_ACTIVITY_BAR: Pixels = px(34.);
/// Icon sizes in the project tree.
pub const TREE_ICON: Pixels = px(18.);
pub const TREE_CHEVRON: Pixels = px(14.);

/// An icon from the bundled Lucide set, by file name (e.g. `"git-branch"`).
pub fn lucide(name: &str) -> Icon {
    Icon::empty().path(format!("icons/{name}.svg"))
}

/// One of the app's own icons (`crates/app/assets/icons`), e.g. the Docker whale.
pub fn app_icon(name: &str) -> Icon {
    Icon::empty().path(format!("app-icons/{name}.svg"))
}

/// Borderless icon button; `size` is one of the `BUTTON_*` constants.
pub fn icon_button_sized(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    tooltip: &'static str,
    size: Pixels,
) -> Button {
    Button::new(id)
        .ghost()
        .with_size(size)
        .icon(icon)
        .tooltip(tooltip)
}

/// Small borderless icon button used in tab strips and panel headers.
pub fn icon_button(id: impl Into<ElementId>, icon: IconName, tooltip: &'static str) -> Button {
    icon_button_sized(id, icon, tooltip, BUTTON_SMALL)
}

/// A destructive confirmation: a red `icon`, Cancel (also Esc) and a red `ok_text` button.
pub fn danger_confirm(dialog: AlertDialog, icon: &str, ok_text: &'static str, cx: &App) -> AlertDialog {
    dialog
        .confirm()
        .width(px(440.))
        .icon(lucide(icon).text_color(cx.theme().danger))
        .footer(confirm_buttons(ok_text, ButtonVariant::Danger))
}

/// Dialog buttons are 1.25× gpui-kit's sizes: bigger targets in confirmations.
const DIALOG_BUTTON_SCALE: f32 = 1.25;

/// A button for a hand-built dialog footer, in place of `.small()` (24 → 30 px high).
pub fn dialog_button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .small()
        .h(px(24. * DIALOG_BUTTON_SCALE))
        .px(px(8. * DIALOG_BUTTON_SCALE))
}

/// Cancel + `ok_text` for an alert dialog's `.footer(..)`, replacing its built-in buttons
/// (32 → 40 px high). They dispatch the dialog's own Confirm / Cancel actions, so the
/// dialog's `on_ok` / `on_cancel`, Enter and Esc work as with the built-in ones.
pub fn confirm_buttons(ok_text: &'static str, ok_variant: ButtonVariant) -> ConfirmButtons {
    ConfirmButtons { ok_text, ok_variant }
}

#[derive(IntoElement)]
pub struct ConfirmButtons {
    ok_text: &'static str,
    ok_variant: ButtonVariant,
}

impl RenderOnce for ConfirmButtons {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::dialog::{Cancel, Confirm, DialogFooter};
        let button = |id: &'static str,
                      label: &'static str,
                      action: Box<dyn Action>,
                      window: &mut Window,
                      cx: &mut App| {
            // Dispatched from a focus node inside the button, so the action reaches the dialog
            // the button sits in whatever holds focus (as gpui-kit's own dialog buttons do).
            let anchor = window
                .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone();
            Button::new(id)
                .label(label)
                .h(px(32. * DIALOG_BUTTON_SCALE))
                .px(px(10. * DIALOG_BUTTON_SCALE))
                .child(div().absolute().size_0().track_focus(&anchor))
                .on_click(move |_, window, cx| anchor.dispatch_action(&*action, window, cx))
        };
        let cancel = button("confirm-cancel", "Cancel", Box::new(Cancel), window, cx);
        let ok = button(
            "confirm-ok",
            self.ok_text,
            Box::new(Confirm { secondary: false }),
            window,
            cx,
        )
        .with_variant(self.ok_variant);
        DialogFooter::new().child(cancel).child(ok)
    }
}

/// Enter in a dialog's `input` runs `submit`, like clicking the dialog's primary button.
///
/// Subscribes through the window, not an entity's `Context`, so `submit` may update the
/// workspace (or any other entity) without re-entering it; the dialog's input owns the
/// subscription's lifetime, so it is detached.
pub fn submit_on_enter(
    input: &Entity<gpui_kit::component::input::InputState>,
    window: &mut Window,
    cx: &mut App,
    submit: impl Fn(&mut Window, &mut App) + 'static,
) {
    use gpui_kit::component::input::InputEvent;
    window
        .subscribe(input, cx, move |_, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: false, .. } = event {
                submit(window, cx);
            }
        })
        .detach();
}
