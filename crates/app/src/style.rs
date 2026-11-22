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
        .ok_text(ok_text)
        .ok_variant(ButtonVariant::Danger)
        .cancel_text("Cancel")
}
