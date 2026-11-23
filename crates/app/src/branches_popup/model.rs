//! Rows of the Branches popup: git actions, then Recent / Local / Remote branch groups,
//! filtered by the search query; and the actions offered for one branch.

use std::collections::HashSet;

use search::fuzzy_filter;
use vcs::Branch;

use crate::actions::GitLogOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Recent,
    Local,
    Remote,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Local => "Local",
            Self::Remote => "Remote",
        }
    }
}

/// Actions at the top of the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCommand {
    Update,
    Commit,
    Push,
    NewBranch,
    NewWorktree,
    CheckoutRevision,
}

impl GitCommand {
    const ALL: [Self; 6] = [
        Self::Update,
        Self::Commit,
        Self::Push,
        Self::NewBranch,
        Self::NewWorktree,
        Self::CheckoutRevision,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Update Project…",
            Self::Commit => "Commit…",
            Self::Push => "Push…",
            Self::NewBranch => "New Branch…",
            Self::NewWorktree => "New Worktree…",
            Self::CheckoutRevision => "Checkout Tag or Revision…",
        }
    }

    /// A separator goes above the branch-creating commands, as in JetBrains.
    fn starts_group(self) -> bool {
        self == Self::NewBranch
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Command(GitCommand),
    Separator,
    Group {
        group: Group,
        expanded: bool,
    },
    /// Index into the branch list.
    Branch {
        ix: usize,
        group: Group,
    },
    /// "Create branch '<query>'" when the query names no branch.
    Create(String),
}

impl Row {
    pub fn selectable(&self) -> bool {
        !matches!(self, Self::Separator)
    }
}

/// Groups are only listed while they have branches. While searching every group is
/// expanded, so a match is never hidden in a folded group.
pub fn rows(branches: &[Branch], recent: &[String], query: &str, collapsed: &HashSet<Group>) -> Vec<Row> {
    let query = query.trim();
    let searching = !query.is_empty();
    let mut rows = Vec::new();

    for command in fuzzy_filter(query, GitCommand::ALL.to_vec(), |c| c.label()) {
        if command.starts_group() && !searching {
            rows.push(Row::Separator);
        }
        rows.push(Row::Command(command));
    }
    if searching && valid_branch_name(query) && !branches.iter().any(|b| b.name == query) {
        rows.push(Row::Create(query.to_string()));
    }

    let local = |b: &&Branch| !b.is_remote;
    let recent_ixs: Vec<usize> = recent
        .iter()
        .filter_map(|name| branches.iter().position(|b| !b.is_remote && &b.name == name))
        .collect();
    let groups: [(Group, Vec<usize>); 3] = [
        (Group::Recent, recent_ixs),
        (Group::Local, indexes(branches, local)),
        (Group::Remote, indexes(branches, |b| b.is_remote)),
    ];
    for (group, ixs) in groups {
        let named: Vec<(usize, &str)> = ixs
            .into_iter()
            .filter_map(|ix| Some((ix, branches.get(ix)?.name.as_str())))
            .collect();
        let ixs: Vec<usize> = fuzzy_filter(query, named, |(_, name)| name)
            .into_iter()
            .map(|(ix, _)| ix)
            .collect();
        if ixs.is_empty() {
            continue;
        }
        let expanded = searching || !collapsed.contains(&group);
        if rows
            .last()
            .is_some_and(|row| !matches!(row, Row::Branch { .. } | Row::Group { .. }))
        {
            rows.push(Row::Separator);
        }
        rows.push(Row::Group { group, expanded });
        if expanded {
            rows.extend(ixs.into_iter().map(|ix| Row::Branch { ix, group }));
        }
    }
    rows
}

fn indexes(branches: &[Branch], keep: impl Fn(&&Branch) -> bool) -> Vec<usize> {
    branches
        .iter()
        .enumerate()
        .filter(|(_, b)| keep(b))
        .map(|(ix, _)| ix)
        .collect()
}

/// Cheap check for the "Create branch" row; git validates the name for real.
fn valid_branch_name(name: &str) -> bool {
    !name.contains(|c: char| c.is_whitespace() || "~^:?*[\\".contains(c))
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
}

/// What a branch offers (its submenu), worded as in JetBrains with the branch names
/// spelled out; `None` is a separator. `current` is the checked-out branch, if any.
pub fn branch_ops(branch: &Branch, current: Option<&str>) -> Vec<Option<(String, GitLogOp)>> {
    let name = &branch.name;
    let mut ops = Vec::new();
    let mut op = |label: String, op: GitLogOp| ops.push(Some((label, op)));
    if !branch.is_current {
        op("Checkout".into(), GitLogOp::CheckoutBranch);
    }
    op(format!("New Branch from '{name}'…"), GitLogOp::NewBranchAt);
    op(format!("New Worktree from '{name}'…"), GitLogOp::NewWorktreeFrom);
    ops.push(None);
    ops.push(Some((
        "Show Diff with Working Tree".into(),
        GitLogOp::DiffWithWorkingTree,
    )));
    if let (false, Some(current)) = (branch.is_current, current) {
        ops.push(None);
        ops.push(Some((
            format!("Rebase '{current}' onto '{name}'"),
            GitLogOp::RebaseCurrentOnto,
        )));
        ops.push(Some((
            format!("Merge '{name}' into '{current}'"),
            GitLogOp::MergeIntoCurrent,
        )));
    }
    ops.push(None);
    if branch.is_remote {
        ops.push(Some(("Delete…".into(), GitLogOp::DeleteBranch)));
        return ops;
    }
    if branch.is_current {
        ops.push(Some(("Update".into(), GitLogOp::UpdateBranch)));
    }
    ops.push(Some(("Push…".into(), GitLogOp::PushBranch)));
    let tracked = match &branch.upstream {
        Some(upstream) => format!("Tracked Branch '{upstream}'…"),
        None => "Set Tracked Branch…".into(),
    };
    ops.push(Some((tracked, GitLogOp::SetTrackedBranch)));
    ops.push(None);
    ops.push(Some(("Rename…".into(), GitLogOp::RenameBranch)));
    if !branch.is_current {
        ops.push(Some(("Delete…".into(), GitLogOp::DeleteBranch)));
    }
    ops
}

/// Menu "safe triangle": whether the pointer, moving from `from` to `to`, heads into a
/// submenu whose left edge spans `top..bottom` at `left`. Rows crossed on the way must
/// not steal the submenu.
pub fn aims_at_submenu(from: (f32, f32), to: (f32, f32), left: f32, top: f32, bottom: f32) -> bool {
    // Side of `p` relative to the line a→b; `to` is inside when all signs agree.
    let side =
        |a: (f32, f32), b: (f32, f32), p: (f32, f32)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let (top, bottom) = ((left, top), (left, bottom));
    let sides = [side(from, top, to), side(top, bottom, to), side(bottom, from, to)];
    sides.iter().all(|s| *s >= 0.) || sides.iter().all(|s| *s <= 0.)
}

/// Menu-aim prediction: the pointer, moving from `from` (a few events back) to `to`, travels
/// right and into the submenu's triangle, widened by some slack because hands don't move
/// in straight lines.
pub fn heads_for_submenu(from: (f32, f32), to: (f32, f32), left: f32, top: f32, bottom: f32) -> bool {
    const SLACK: f32 = 100.;
    to.0 > from.0 && aims_at_submenu(from, to, left, top - SLACK, bottom + SLACK)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use vcs::Branch;

    use super::{GitCommand, Group, Row, aims_at_submenu, branch_ops, heads_for_submenu, rows};
    use crate::actions::GitLogOp;

    fn branch(name: &str, is_remote: bool, is_current: bool) -> Branch {
        Branch {
            name: name.into(),
            is_remote,
            is_current,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
        }
    }

    fn sample() -> Vec<Branch> {
        vec![
            branch("main", false, true),
            branch("feature/login", false, false),
            branch("origin/main", true, false),
        ]
    }

    #[test]
    fn lists_commands_then_groups() {
        let collapsed = HashSet::from([Group::Remote]);
        let rows = rows(&sample(), &["main".into()], "", &collapsed);
        assert_eq!(rows[0], Row::Command(GitCommand::Update));
        assert_eq!(rows[3], Row::Separator);
        assert_eq!(rows[4], Row::Command(GitCommand::NewBranch));
        assert_eq!(rows[5], Row::Command(GitCommand::NewWorktree));
        let tail: Vec<Row> = rows[7..].to_vec();
        assert_eq!(
            tail,
            vec![
                Row::Separator,
                Row::Group {
                    group: Group::Recent,
                    expanded: true
                },
                Row::Branch {
                    ix: 0,
                    group: Group::Recent
                },
                Row::Group {
                    group: Group::Local,
                    expanded: true
                },
                Row::Branch {
                    ix: 0,
                    group: Group::Local
                },
                Row::Branch {
                    ix: 1,
                    group: Group::Local
                },
                Row::Group {
                    group: Group::Remote,
                    expanded: false
                },
            ]
        );
    }

    #[test]
    fn search_expands_groups_and_offers_to_create() {
        let collapsed = HashSet::from([Group::Remote]);
        let found = rows(&sample(), &[], "orig", &collapsed);
        assert!(found.contains(&Row::Branch {
            ix: 2,
            group: Group::Remote
        }));
        assert!(found.contains(&Row::Create("orig".into())));
        let exact = rows(&sample(), &[], "main", &collapsed);
        assert!(!exact.iter().any(|row| matches!(row, Row::Create(_))));
        let spaced = rows(&sample(), &[], "two words", &collapsed);
        assert!(!spaced.iter().any(|row| matches!(row, Row::Create(_))));
    }

    #[test]
    fn current_and_remote_branches_offer_fewer_ops() {
        let ops = |b: &Branch| -> Vec<GitLogOp> {
            branch_ops(b, Some("main"))
                .into_iter()
                .flatten()
                .map(|(_, op)| op)
                .collect()
        };
        let current = ops(&branch("main", false, true));
        assert!(!current.contains(&GitLogOp::CheckoutBranch));
        assert!(!current.contains(&GitLogOp::DeleteBranch));
        assert!(!current.contains(&GitLogOp::MergeIntoCurrent));
        assert!(current.contains(&GitLogOp::UpdateBranch));
        let remote = ops(&branch("origin/x", true, false));
        assert!(!remote.contains(&GitLogOp::RenameBranch));
        assert!(!remote.contains(&GitLogOp::PushBranch));
        assert!(remote.contains(&GitLogOp::DeleteBranch));
    }

    #[test]
    fn labels_name_both_branches() {
        let labels: Vec<String> = branch_ops(&branch("feature", false, false), Some("main"))
            .into_iter()
            .flatten()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(
            labels,
            [
                "Checkout",
                "New Branch from 'feature'…",
                "New Worktree from 'feature'…",
                "Show Diff with Working Tree",
                "Rebase 'main' onto 'feature'",
                "Merge 'feature' into 'main'",
                "Push…",
                "Set Tracked Branch…",
                "Rename…",
                "Delete…",
            ]
        );
    }

    #[test]
    fn diagonal_moves_toward_the_submenu_keep_it() {
        // Submenu to the right: left edge x=400, from y=0 to y=200; pointer on a row at y=300.
        let aims = |to| aims_at_submenu((300., 300.), to, 400., 0., 200.);
        assert!(aims((320., 280.)), "up and to the right, across the rows above");
        assert!(!aims((340., 300.)), "straight right, below the short submenu");
        assert!(
            aims_at_submenu((300., 300.), (340., 300.), 400., 0., 400.),
            "right, into a tall one"
        );
        assert!(!aims((300., 280.)), "straight up: choosing another row");
        assert!(!aims((290., 290.)), "away from the submenu");
        assert!(!aims((320., 320.)), "down, below the submenu");
    }

    #[test]
    fn predicts_wobbly_diagonal_moves() {
        // Pointer at (200, 245) under a submenu at x=400, y=30..130 (as in the popup).
        let heads = |to| heads_for_submenu((200., 245.), to, 400., 30., 130.);
        assert!(
            heads((230., 200.)),
            "steeper than the exact triangle, within the slack"
        );
        assert!(heads((240., 240.)), "mostly right");
        assert!(!heads((200., 200.)), "straight up: choosing another row");
        assert!(!heads((190., 230.)), "moving left");
        assert!(!heads((230., 290.)), "down, away from the submenu");
    }
}
