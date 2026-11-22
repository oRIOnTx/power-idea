//! JetBrains-style Branches popup under the title bar's branch widget: search, git actions,
//! Recent / Local / Remote branches, and a submenu with what a branch offers.

mod model;

use std::collections::HashSet;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::Branch;

use crate::actions::{self, BRANCHES_CONTEXT, GitLogOp, SelectNext, SelectPrev, WORKSPACE_CONTEXT};
use crate::style;
pub use model::branch_ops;
use model::{GitCommand, Group, Row};

const LIST_WIDTH: f32 = 400.;
const SUBMENU_WIDTH: f32 = 240.;
const MAX_LIST_HEIGHT: f32 = 520.;
const ROW_HEIGHT: f32 = 26.;
/// How long the pointer may rest on another row, while heading for the submenu, before
/// that row takes over.
const AIM_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

pub enum BranchesPopupEvent {
    Dismiss,
    /// A workspace action (Update Project, Commit, Push, Fetch, New Branch, …).
    Run(Box<dyn Action>),
    /// A branch operation from the submenu.
    Branch(GitLogOp, Branch),
    /// "Create branch" for the typed name.
    Create(String),
}

impl EventEmitter<BranchesPopupEvent> for BranchesPopup {}

pub struct BranchesPopup {
    input: Entity<InputState>,
    branches: Vec<Branch>,
    recent: Vec<String>,
    collapsed: HashSet<Group>,
    rows: Vec<Row>,
    selected: usize,
    /// The branch row (index into `rows`) whose submenu is open, and its selected item.
    submenu: Option<(usize, usize)>,
    /// Where the open submenu is drawn, for the safe-triangle check.
    submenu_bounds: Option<Bounds<Pixels>>,
    /// Recent pointer positions over the popup, oldest first: the direction of travel.
    trail: std::collections::VecDeque<Point<Pixels>>,
    /// A row hovered on the way to the submenu; selected only if the pointer stays.
    _aim_task: Option<Task<()>>,
    scroll: ScrollHandle,
    focused_once: bool,
    _subscription: Subscription,
}

impl BranchesPopup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search for branches and actions"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.refresh(cx),
                    InputEvent::PressEnter { .. } => this.confirm(window, cx),
                    _ => {}
                },
            );
        let mut popup = Self {
            input,
            branches: Vec::new(),
            recent: Vec::new(),
            collapsed: HashSet::from([Group::Remote]),
            rows: Vec::new(),
            selected: 0,
            submenu: None,
            submenu_bounds: None,
            trail: Default::default(),
            _aim_task: None,
            scroll: ScrollHandle::new(),
            focused_once: false,
            _subscription: subscription,
        };
        popup.refresh(cx);
        popup
    }

    /// Fills in the branches once git has listed them.
    pub fn set_branches(&mut self, branches: Vec<Branch>, recent: Vec<String>, cx: &mut Context<Self>) {
        self.branches = branches;
        self.recent = recent;
        self.refresh(cx);
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.rows = model::rows(&self.branches, &self.recent, &query, &self.collapsed);
        self.submenu = None;
        // While searching, the best match goes first: skip the commands nobody asked for.
        let first_branch = self.rows.iter().position(|r| matches!(r, Row::Branch { .. }));
        self.selected = match (query.is_empty(), first_branch) {
            (false, Some(ix)) if !self.rows.iter().any(|r| matches!(r, Row::Command(_))) => ix,
            _ => self.rows.iter().position(Row::selectable).unwrap_or(0),
        };
        cx.notify();
    }

    fn branch_at(&self, row: usize) -> Option<&Branch> {
        match self.rows.get(row)? {
            Row::Branch { ix, .. } => self.branches.get(*ix),
            _ => None,
        }
    }

    fn submenu_ops(&self) -> Vec<(&'static str, GitLogOp)> {
        self.submenu
            .and_then(|(row, _)| self.branch_at(row))
            .map_or_default(|branch| branch_ops(branch).into_iter().flatten().collect())
    }

    fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.selected != row {
            self.selected = row;
            // Hovering another branch moves an open submenu along, as in JetBrains.
            if self.submenu.is_some() {
                self.submenu = self.branch_at(row).map(|_| (row, 0));
            }
            cx.notify();
        }
    }

    /// The pointer moved over row `ix`. While it heads for the open submenu (predicted from
    /// its recent trail), crossing other rows doesn't move or close the submenu; a row takes
    /// over only once the pointer rests on it.
    fn hover_row(&mut self, ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let heading_to_submenu = match (self.trail.front(), self.submenu, self.submenu_bounds) {
            (Some(from), Some((row, _)), Some(target)) if row != ix => model::heads_for_submenu(
                (f32::from(from.x), f32::from(from.y)),
                (f32::from(position.x), f32::from(position.y)),
                f32::from(target.origin.x),
                f32::from(target.origin.y),
                f32::from(target.bottom()),
            ),
            _ => false,
        };
        if !heading_to_submenu {
            self._aim_task = None;
            return self.select(ix, cx);
        }
        // Each move restarts the wait, so the switch happens only when the pointer stops.
        self._aim_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AIM_DELAY).await;
            this.update(cx, |this, cx| this.select(ix, cx)).ok();
        }));
    }

    /// Runs after the row handlers (bubble phase), so they compare against earlier positions.
    fn track_pointer(&mut self, position: Point<Pixels>) {
        const TRAIL: usize = 5;
        if self.trail.len() == TRAIL {
            self.trail.pop_front();
        }
        self.trail.push_back(position);
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some((row, item)) = self.submenu {
            let len = self.submenu_ops().len().max(1);
            let item = if forward {
                (item + 1) % len
            } else {
                (item + len - 1) % len
            };
            self.submenu = Some((row, item));
            return cx.notify();
        }
        let len = self.rows.len();
        let mut ix = self.selected;
        for _ in 0..len {
            ix = if forward {
                (ix + 1) % len
            } else {
                (ix + len - 1) % len
            };
            if self.rows[ix].selectable() {
                break;
            }
        }
        self.selected = ix;
        self.scroll.scroll_to_item(ix);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn open_submenu(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.branch_at(row).is_some() {
            self.selected = row;
            self.submenu = Some((row, 0));
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((row, item)) = self.submenu {
            let op = self.submenu_ops().get(item).map(|(_, op)| *op);
            if let (Some(op), Some(branch)) = (op, self.branch_at(row).cloned()) {
                cx.emit(BranchesPopupEvent::Branch(op, branch));
            }
            return;
        }
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        match row {
            Row::Command(command) => cx.emit(BranchesPopupEvent::Run(command_action(command))),
            Row::Group { group, .. } => self.toggle_group(group, cx),
            Row::Branch { .. } => self.open_submenu(self.selected, cx),
            Row::Create(name) => cx.emit(BranchesPopupEvent::Create(name)),
            Row::Separator => {}
        }
    }

    fn toggle_group(&mut self, group: Group, cx: &mut Context<Self>) {
        if !self.query(cx).is_empty() {
            return;
        }
        if !self.collapsed.remove(&group) {
            self.collapsed.insert(group);
        }
        let selected = self.selected;
        self.refresh(cx);
        self.selected = selected.min(self.rows.len().saturating_sub(1));
    }

    /// ←/→ fold the submenu and Escape closes it before closing the popup.
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" if self.submenu.is_some() => self.submenu = None,
            "escape" => cx.emit(BranchesPopupEvent::Dismiss),
            "left" if self.submenu.is_some() => self.submenu = None,
            "right" if self.submenu.is_none() && self.branch_at(self.selected).is_some() => {
                self.open_submenu(self.selected, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn current_ahead(&self) -> u32 {
        self.branches.iter().find(|b| b.is_current).map_or(0, |b| b.ahead)
    }

    fn render_row(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[ix];
        if *row == Row::Separator {
            return div().mx_2().my_1().h(px(1.)).bg(theme.border).into_any_element();
        }
        let selected = ix == self.selected;
        let base = h_flex()
            .id(("branch-row", ix))
            .h(px(ROW_HEIGHT))
            .mx_1()
            .px_2()
            .gap_2()
            .rounded_sm()
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.list_active))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.hover_row(ix, event.position, cx);
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = ix;
                this.confirm(window, cx);
            }));
        let muted = theme.muted_foreground;
        let icon_slot = |icon: Option<Icon>| {
            div()
                .flex_none()
                .w(px(16.))
                .flex()
                .justify_center()
                .children(icon.map(|i| i.small().text_color(muted)))
        };
        match row {
            Row::Command(command) => {
                let icon = match command {
                    GitCommand::Update => Some(style::lucide("arrow-down-left")),
                    GitCommand::Commit => Some(style::lucide("git-commit-horizontal")),
                    GitCommand::Push => Some(style::lucide("arrow-up-right")),
                    GitCommand::NewBranch => Some(style::lucide("plus")),
                    GitCommand::CheckoutRevision => None,
                };
                let action = command_action(*command);
                base.child(icon_slot(icon))
                    .child(command.label())
                    .when(*command == GitCommand::Push, |el| {
                        el.children(style::tracking_arrows(self.current_ahead(), 0, cx))
                    })
                    .child(div().flex_1())
                    .children(Kbd::binding_for_action(
                        action.as_ref(),
                        Some(WORKSPACE_CONTEXT),
                        window,
                    ))
                    .into_any_element()
            }
            Row::Group { group, expanded } => base
                .child(
                    Icon::new(if *expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(style::TREE_CHEVRON)
                    .text_color(muted),
                )
                .child(group.title())
                .into_any_element(),
            Row::Branch { ix: branch_ix, .. } => {
                let branch = &self.branches[*branch_ix];
                let tint = if branch.is_current { theme.primary } else { muted };
                let open = self.submenu.is_some_and(|(row, _)| row == ix);
                base.pl(px(26.))
                    .when(open, |el| el.bg(theme.list_active))
                    .child(style::lucide("git-branch").small().flex_none().text_color(tint))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .when(branch.is_current, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(branch.name.clone()),
                    )
                    .children(style::tracking_arrows(branch.ahead, branch.behind, cx))
                    .child(div().flex_1())
                    .children(branch.upstream.clone().map(|upstream| {
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if branch.upstream_gone { theme.danger } else { muted })
                            .child(upstream)
                    }))
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .xsmall()
                            .flex_none()
                            .text_color(muted),
                    )
                    .into_any_element()
            }
            Row::Create(name) => base
                .child(icon_slot(Some(style::lucide("plus"))))
                .child(div().text_color(theme.primary).child("Create branch"))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(format!("'{name}'")),
                )
                .into_any_element(),
            Row::Separator => unreachable!("handled above"),
        }
    }

    fn render_submenu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (row, selected) = self.submenu?;
        let branch = self.branch_at(row)?.clone();
        let theme = cx.theme();
        let mut items: Vec<AnyElement> = Vec::new();
        let mut item_ix = 0;
        for op in branch_ops(&branch) {
            let Some((label, op)) = op else {
                items.push(div().mx_2().my_1().h(px(1.)).bg(theme.border).into_any_element());
                continue;
            };
            let ix = item_ix;
            item_ix += 1;
            let branch = branch.clone();
            items.push(
                h_flex()
                    .id(("branch-op", ix))
                    .h(px(ROW_HEIGHT))
                    .mx_1()
                    .px_2()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .when(ix == selected, |el| el.bg(theme.list_active))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && let Some((row, _)) = this.submenu {
                            this.submenu = Some((row, ix));
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(BranchesPopupEvent::Branch(op, branch.clone()));
                    }))
                    .child(label)
                    .into_any_element(),
            );
        }
        let this = cx.entity().downgrade();
        Some(
            v_flex()
                .relative()
                .w(px(SUBMENU_WIDTH))
                .flex_none()
                // Reaching the submenu ends the aim: no row it crossed takes over.
                .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| this._aim_task = None))
                .child(
                    canvas(
                        move |bounds, _, cx| {
                            this.update(cx, |this, _| this.submenu_bounds = Some(bounds)).ok();
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .py_1()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(branch.name),
                )
                .children(items),
        )
    }
}

impl Render for BranchesPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme();
        let border = theme.border;
        let fetch = style::icon_button("branches-fetch", IconName::ArrowDown, "Fetch")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(BranchesPopupEvent::Run(Box::new(actions::Fetch)))));
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|ix| self.render_row(ix, window, cx))
            .collect();
        let list = v_flex()
            .w(px(LIST_WIDTH))
            .flex_none()
            .child(
                h_flex()
                    .p_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div().flex_1().child(
                            Input::new(&self.input)
                                .prefix(Icon::new(IconName::Search).small())
                                .small()
                                .cleanable(false),
                        ),
                    )
                    .child(fetch),
            )
            .child(
                div()
                    .id("branches-list")
                    .max_h(px(MAX_LIST_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .py_1()
                    .children(rows),
            );

        h_flex()
            .key_context(BRANCHES_CONTEXT)
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, _| this.track_pointer(event.position)),
            )
            .capture_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .items_start()
            .child(list)
            .children(self.render_submenu(cx))
    }
}

fn command_action(command: GitCommand) -> Box<dyn Action> {
    match command {
        GitCommand::Update => Box::new(actions::UpdateProject),
        GitCommand::Commit => Box::new(actions::CommitChanges),
        GitCommand::Push => Box::new(actions::Push),
        GitCommand::NewBranch => Box::new(actions::NewBranch),
        GitCommand::CheckoutRevision => Box::new(actions::CheckoutRevision),
    }
}
