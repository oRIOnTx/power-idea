//! Merge conflicts: parsing conflict markers into chunks, building the resolved
//! text from per-chunk choices, and resolving whole files (accept a side, mark resolved).

use anyhow::{Context as _, Result};

use super::Repo;

/// One conflict chunk between `<<<<<<<` and `>>>>>>>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictChunk {
    pub ours: String,
    /// Common ancestor text (diff3 / zdiff3 conflict style).
    pub base: Option<String>,
    pub theirs: String,
    /// Marker lines, kept to write unresolved chunks back unchanged.
    markers: [String; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictSegment {
    Common(String),
    Conflict(ConflictChunk),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// `marker` followed by a space or the line end, e.g. `<<<<<<< HEAD`.
fn is_marker(line: &str, marker: &str) -> bool {
    line.strip_prefix(marker)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\n', '\r']))
}

/// Splits `text` into common text and conflict chunks. Unterminated markers stay common text.
pub fn parse_conflicts(text: &str) -> Vec<ConflictSegment> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut segments = Vec::new();
    let mut common = String::new();
    let mut ix = 0;
    while ix < lines.len() {
        if is_marker(lines[ix], "<<<<<<<")
            && let Some((chunk, next)) = parse_chunk(&lines, ix)
        {
            if !common.is_empty() {
                segments.push(ConflictSegment::Common(std::mem::take(&mut common)));
            }
            segments.push(ConflictSegment::Conflict(chunk));
            ix = next;
        } else {
            common.push_str(lines[ix]);
            ix += 1;
        }
    }
    if !common.is_empty() {
        segments.push(ConflictSegment::Common(common));
    }
    segments
}

/// The chunk starting at `start` and the line after it, if it is complete.
fn parse_chunk(lines: &[&str], start: usize) -> Option<(ConflictChunk, usize)> {
    let (mut ours, mut base, mut theirs) = (String::new(), None::<String>, String::new());
    let mut base_marker = String::new();
    let mut middle = None;
    let mut ix = start + 1;
    while ix < lines.len() {
        let line = lines[ix];
        if middle.is_none() && base.is_none() && is_marker(line, "|||||||") {
            base = Some(String::new());
            base_marker = line.to_string();
        } else if middle.is_none() && is_marker(line, "=======") {
            middle = Some(line.to_string());
        } else if middle.is_some() && is_marker(line, ">>>>>>>") {
            let chunk = ConflictChunk {
                ours,
                base,
                theirs,
                markers: [lines[start].to_string(), base_marker, middle?, line.to_string()],
            };
            return Some((chunk, ix + 1));
        } else if is_marker(line, "<<<<<<<") {
            return None;
        } else if middle.is_some() {
            theirs.push_str(line);
        } else if let Some(base) = base.as_mut() {
            base.push_str(line);
        } else {
            ours.push_str(line);
        }
        ix += 1;
    }
    None
}

impl ConflictChunk {
    /// Branch or commit names from the markers: (ours, theirs).
    pub fn labels(&self) -> (&str, &str) {
        fn label(marker: &str) -> &str {
            marker.get(7..).unwrap_or_default().trim()
        }
        (label(&self.markers[0]), label(&self.markers[3]))
    }

    fn with_markers(&self) -> String {
        let [start, base_marker, middle, end] = &self.markers;
        let mut text = format!("{start}{}", self.ours);
        if let Some(base) = &self.base {
            text.push_str(base_marker);
            text.push_str(base);
        }
        text.push_str(middle);
        text.push_str(&self.theirs);
        text.push_str(end);
        text
    }
}

/// The file text with each chunk resolved as chosen; unresolved chunks keep their markers.
pub fn resolve(segments: &[ConflictSegment], choices: &[Option<Resolution>]) -> String {
    let mut text = String::new();
    let mut chunk = 0;
    for segment in segments {
        match segment {
            ConflictSegment::Common(common) => text.push_str(common),
            ConflictSegment::Conflict(c) => {
                match choices.get(chunk).copied().flatten() {
                    Some(Resolution::Ours) => text.push_str(&c.ours),
                    Some(Resolution::Theirs) => text.push_str(&c.theirs),
                    Some(Resolution::OursThenTheirs) => {
                        text.push_str(&c.ours);
                        text.push_str(&c.theirs);
                    }
                    Some(Resolution::TheirsThenOurs) => {
                        text.push_str(&c.theirs);
                        text.push_str(&c.ours);
                    }
                    None => text.push_str(&c.with_markers()),
                }
                chunk += 1;
            }
        }
    }
    text
}

pub fn conflict_count(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Conflict(_)))
        .count()
}

impl Repo {
    /// Resolves `path` (repo-relative) with one side's whole version; a side that
    /// deleted the file deletes it.
    pub fn accept_side(&self, path: &str, side: Side) -> Result<()> {
        let stage = match side {
            Side::Ours => "2",
            Side::Theirs => "3",
        };
        let stages = self.run(&["ls-files", "-u", "--", path])?;
        let has_stage = stages
            .lines()
            .any(|line| line.split_whitespace().nth(2) == Some(stage));
        if has_stage {
            let flag = if side == Side::Ours { "--ours" } else { "--theirs" };
            self.run(&["checkout", flag, "--", path])?;
            self.mark_resolved(path)
        } else {
            self.run(&["rm", "-q", "--", path]).map(drop)
        }
    }

    /// Stages `path` as resolved (after editing out the markers).
    pub fn mark_resolved(&self, path: &str) -> Result<()> {
        self.run(&["add", "--", path])
            .map(drop)
            .with_context(|| format!("marking {path} as resolved"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;
    use crate::{ChangeKind, Operation};

    const TEXT: &str = "top\n<<<<<<< HEAD\nmine\n=======\ntheirs\nmore\n>>>>>>> feature\nmiddle\n\
                        <<<<<<< HEAD\na\n||||||| base\nb\n=======\nc\n>>>>>>> feature\nend";

    #[test]
    fn parses_chunks_with_and_without_base() {
        let segments = parse_conflicts(TEXT);
        assert_eq!(segments.len(), 5);
        assert_eq!(conflict_count(&segments), 2);
        let ConflictSegment::Conflict(first) = &segments[1] else {
            panic!()
        };
        assert_eq!(
            (first.ours.as_str(), first.theirs.as_str()),
            ("mine\n", "theirs\nmore\n")
        );
        assert_eq!(first.base, None);
        assert_eq!(first.labels(), ("HEAD", "feature"));
        let ConflictSegment::Conflict(second) = &segments[3] else {
            panic!()
        };
        assert_eq!(second.base.as_deref(), Some("b\n"));
        assert_eq!(segments[4], ConflictSegment::Common("end".into()));
    }

    #[test]
    fn resolves_and_keeps_unresolved_markers() {
        let segments = parse_conflicts(TEXT);
        assert_eq!(resolve(&segments, &[None, None]), TEXT);
        assert_eq!(
            resolve(
                &segments,
                &[Some(Resolution::Theirs), Some(Resolution::OursThenTheirs)]
            ),
            "top\ntheirs\nmore\nmiddle\na\nc\nend"
        );
        assert_eq!(
            resolve(
                &segments,
                &[Some(Resolution::TheirsThenOurs), Some(Resolution::Ours)]
            ),
            "top\ntheirs\nmore\nmine\nmiddle\na\nend"
        );
    }

    #[test]
    fn leaves_broken_markers_as_text() {
        let text = "<<<<<<< HEAD\nx\n=======\ny\n";
        assert_eq!(parse_conflicts(text), [ConflictSegment::Common(text.into())]);
        assert_eq!(conflict_count(&parse_conflicts("<<<<<<<< not a marker\n")), 0);
    }

    fn conflicted_repo() -> TempRepo {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "base\n", "base");
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.commit_file("a.txt", "feature\n", "feature");
        repo.git(&["checkout", "-q", "main"]);
        repo.commit_file("a.txt", "main\n", "main");
        assert!(repo.0.merge("feature").is_err());
        repo
    }

    #[test]
    fn accepts_a_side_and_marks_resolved() {
        let repo = conflicted_repo();
        assert_eq!(repo.0.operation_in_progress(), Some(Operation::Merge));
        assert_eq!(conflict_count(&parse_conflicts(&repo.read("a.txt"))), 1);
        repo.0.accept_side("a.txt", Side::Theirs).unwrap();
        assert_eq!(repo.read("a.txt"), "feature\n");
        let status = repo.0.status().unwrap();
        assert!(status.changes.iter().all(|c| c.kind != ChangeKind::Conflicted));

        let repo = conflicted_repo();
        repo.write("a.txt", "both\n");
        repo.0.mark_resolved("a.txt").unwrap();
        assert!(
            repo.0
                .status()
                .unwrap()
                .changes
                .iter()
                .all(|c| c.kind != ChangeKind::Conflicted)
        );
    }
}
