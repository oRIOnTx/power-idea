//! The tab's non-ready states: checking, no repository or remote, not GitHub, and the
//! "Connect to GitHub" choices (gh CLI or a personal access token).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::CiView;
use super::fetch::Setup;
use crate::style;

impl CiView {
    /// A centered message with an optional hint and buttons: every non-ready state.
    pub(super) fn render_message(
        &self,
        icon: &str,
        title: &str,
        hint: impl Into<SharedString>,
        actions: Vec<AnyElement>,
        cx: &App,
    ) -> Div {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(style::lucide(icon).large().text_color(theme.muted_foreground))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .max_w(px(520.))
                    .text_xs()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(hint.into()),
            )
            .child(h_flex().gap_2().pt_2().children(actions))
    }

    pub(super) fn render_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.setup {
            Setup::Checking => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(Spinner::new().small())
                .into_any_element(),
            Setup::NoRepo => self
                .render_message(
                    "git-branch",
                    "Not a git repository",
                    "GitHub Actions show up here once the project is a git repository with a GitHub remote.",
                    Vec::new(),
                    cx,
                )
                .into_any_element(),
            Setup::NoRemote => self
                .render_message(
                    "git-branch",
                    "No remote",
                    "Add a GitHub remote (git remote add origin git@github.com:owner/repo.git) to see its workflow runs.",
                    vec![self.retry_button(cx)],
                    cx,
                )
                .into_any_element(),
            Setup::NotGitHub(url) => self
                .render_message(
                    "git-branch",
                    "Not a GitHub repository",
                    format!("The remote points to {url}. Only GitHub Actions are supported for now."),
                    Vec::new(),
                    cx,
                )
                .into_any_element(),
            Setup::NotConnected { gh_installed, error } => {
                self.render_connect(*gh_installed, error.clone(), cx).into_any_element()
            }
            Setup::Ready => div().into_any_element(),
        }
    }

    pub(super) fn retry_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("ci-retry")
            .small()
            .label("Check again")
            .on_click(cx.listener(|view, _, _, cx| view.retry_setup(cx)))
            .into_any_element()
    }

    pub(super) fn render_connect(
        &self,
        gh_installed: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let card = |title: &'static str| {
            v_flex()
                .w_full()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
        };
        let muted = |text: &'static str| div().text_xs().text_color(theme.muted_foreground).child(text);
        let gh = card("1. GitHub CLI (recommended)")
            .child(muted(if gh_installed {
                "Log in once with gh; the IDE asks gh for the token and never stores it."
            } else {
                "Install the GitHub CLI (brew install gh), then log in; the IDE asks gh for the token."
            }))
            .child(
                h_flex()
                    .gap_2()
                    .when(gh_installed, |el| {
                        el.child(
                            Button::new("ci-gh-login")
                                .small()
                                .primary()
                                .label("Log in with gh in Terminal")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.run_in_terminal(
                                        "gh auth login --hostname github.com --web",
                                        window,
                                        cx,
                                    );
                                })),
                        )
                    })
                    .when(!gh_installed, |el| {
                        el.child(
                            Button::new("ci-gh-install")
                                .small()
                                .label("Install in Terminal")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.run_in_terminal(
                                        "brew install gh && gh auth login --web",
                                        window,
                                        cx,
                                    );
                                })),
                        )
                    })
                    .child(self.retry_button(cx)),
            );
        let pat = card("2. Personal access token")
            .child(muted(
                "A fine-grained token with Actions: read & write, Contents: read and Metadata: read. \
                 It is kept in the macOS Keychain.",
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.pat_input).small()))
                    .child(
                        Button::new("ci-save-pat")
                            .small()
                            .label("Save")
                            .loading(self.saving_pat)
                            .on_click(cx.listener(|view, _, _, cx| view.save_pat(cx))),
                    ),
            )
            .child(
                Button::new("ci-create-pat")
                    .xsmall()
                    .ghost()
                    .icon(style::lucide("external-link"))
                    .label("Create a token on GitHub")
                    .on_click(|_, _, cx| {
                        cx.open_url("https://github.com/settings/personal-access-tokens/new");
                    }),
            );
        v_flex().size_full().items_center().justify_center().p_6().child(
            v_flex()
                .w(px(520.))
                .max_w_full()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(style::lucide("github").text_color(theme.muted_foreground))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Connect to GitHub")),
                )
                .children(error.map(|e| div().text_xs().text_color(theme.danger).child(e)))
                .child(gh)
                .child(pat),
        )
    }
}
