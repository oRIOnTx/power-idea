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
    CheckoutRevision,
}

impl GitCommand {
    const ALL: [Self; 5] = [
        Self::Update,
        Self::Commit,
        Self::Push,
        Self::NewBranch,
        Self::CheckoutRevision,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Update Project…",
            Self::Commit => "Commit…",
            Self::Push => "Push…",
            Self::NewBranch => "New Branch…",
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
            .map(|ix| (ix, branches[ix].name.as_str()))
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

/// What a branch offers (its submenu); `None` is a separator.
pub fn branch_ops(branch: &Branch) -> Vec<Option<(&'static str, GitLogOp)>> {
    let mut ops = Vec::new();
    if !branch.is_current {
        ops.push(Some(("Checkout", GitLogOp::CheckoutBranch)));
    }
    ops.push(Some(("New Branch from Here…", GitLogOp::NewBranchAt)));
    if !branch.is_current {
        ops.push(None);
        ops.push(Some(("Merge into Current", GitLogOp::MergeIntoCurrent)));
        ops.push(Some(("Rebase Current onto This", GitLogOp::RebaseCurrentOnto)));
    }
    ops.push(None);
    if branch.is_remote {
        ops.push(Some(("Delete…", GitLogOp::DeleteBranch)));
        return ops;
    }
    ops.push(Some(("Push", GitLogOp::PushBranch)));
    ops.push(Some(("Rename…", GitLogOp::RenameBranch)));
    if !branch.is_current {
        ops.push(Some(("Delete…", GitLogOp::DeleteBranch)));
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use vcs::Branch;

    use super::{GitCommand, Group, Row, aims_at_submenu, branch_ops, rows};
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
        let tail: Vec<Row> = rows[6..].to_vec();
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
        let ops =
            |b: &Branch| -> Vec<GitLogOp> { branch_ops(b).into_iter().flatten().map(|(_, op)| op).collect() };
        let current = ops(&branch("main", false, true));
        assert!(!current.contains(&GitLogOp::CheckoutBranch));
        assert!(!current.contains(&GitLogOp::DeleteBranch));
        let remote = ops(&branch("origin/x", true, false));
        assert!(!remote.contains(&GitLogOp::RenameBranch));
        assert!(remote.contains(&GitLogOp::DeleteBranch));
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
}
