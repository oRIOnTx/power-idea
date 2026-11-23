//! Commit graph layout: assigns each commit a lane (column) and describes the
//! line segments to draw in its row, like the graph in JetBrains' Git log.
//!
//! Each row is split vertically into a top half (lines arriving from the row
//! above) and a bottom half (lines leaving towards the row below); the commit
//! node sits at the middle of its lane.

/// Vertical anchor of a segment end inside a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub from: (usize, Anchor),
    pub to: (usize, Anchor),
    /// Lane whose color the segment uses.
    pub lane: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// Lane of the commit node.
    pub column: usize,
    pub segments: Vec<Segment>,
    /// Lanes in use in this row (for sizing the graph column).
    pub width: usize,
}

/// Lays out `commits` (hash, parent hashes), newest first.
pub fn layout<'a>(commits: impl IntoIterator<Item = (&'a str, &'a [String])>) -> Vec<GraphRow> {
    // Each lane holds the hash of the commit it is heading to.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::new();

    for (hash, parents) in commits {
        let before = lanes.clone();
        let column = match lanes.iter().position(|l| l.as_deref() == Some(hash)) {
            Some(col) => col,
            None => free_slot(&mut lanes),
        };
        let mut segments = Vec::new();

        // Lines arriving from above: either into this node or passing by.
        for (j, lane) in before.iter().enumerate() {
            let Some(target) = lane else { continue };
            if target == hash {
                segments.push(Segment {
                    from: (j, Anchor::Top),
                    to: (column, Anchor::Middle),
                    lane: j,
                });
                if j != column {
                    lanes[j] = None; // another branch converges into this commit
                }
            }
        }

        // Continue towards the parents.
        lanes[column] = parents.first().cloned();
        for parent in parents.iter().skip(1) {
            if !lanes.iter().any(|l| l.as_deref() == Some(parent.as_str())) {
                let slot = free_slot(&mut lanes);
                lanes[slot] = Some(parent.clone());
            }
        }
        for parent in parents {
            if let Some(k) = lanes.iter().position(|l| l.as_deref() == Some(parent.as_str())) {
                segments.push(Segment {
                    from: (column, Anchor::Middle),
                    to: (k, Anchor::Bottom),
                    lane: k,
                });
            }
        }

        // Lanes that just pass through this row.
        for (j, lane) in before.iter().enumerate() {
            if let Some(target) = lane
                && target != hash
                && lanes.get(j) == Some(lane)
            {
                segments.push(Segment {
                    from: (j, Anchor::Top),
                    to: (j, Anchor::Bottom),
                    lane: j,
                });
            }
        }

        while lanes.last() == Some(&None) {
            lanes.pop();
        }
        let width = before.len().max(lanes.len()).max(column + 1);
        rows.push(GraphRow {
            column,
            segments,
            width,
        });
    }
    rows
}

fn free_slot(lanes: &mut Vec<Option<String>>) -> usize {
    if let Some(slot) = lanes.iter().position(Option::is_none) {
        slot
    } else {
        lanes.push(None);
        lanes.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commits(spec: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        spec.iter()
            .map(|(h, ps)| {
                (
                    h.to_string(),
                    ps.iter().map(std::string::ToString::to_string).collect(),
                )
            })
            .collect()
    }

    fn run(spec: &[(&str, &[&str])]) -> Vec<GraphRow> {
        let commits = commits(spec);
        layout(commits.iter().map(|(h, ps)| (h.as_str(), ps.as_slice())))
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = run(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert!(rows.iter().all(|r| r.column == 0 && r.width == 1));
        // Middle row: arrives from the top, leaves to the bottom.
        assert!(rows[1].segments.contains(&Segment {
            from: (0, Anchor::Top),
            to: (0, Anchor::Middle),
            lane: 0
        }));
        assert!(rows[1].segments.contains(&Segment {
            from: (0, Anchor::Middle),
            to: (0, Anchor::Bottom),
            lane: 0
        }));
        // Root commit has nothing below.
        assert!(rows[2].segments.iter().all(|s| s.to.1 != Anchor::Bottom));
    }

    #[test]
    fn merge_opens_and_closes_a_side_lane() {
        // m merges feature (f) into main (b); both come from a.
        let rows = run(&[("m", &["b", "f"]), ("f", &["a"]), ("b", &["a"]), ("a", &[])]);
        assert_eq!(rows[0].column, 0);
        assert!(rows[0].segments.contains(&Segment {
            from: (0, Anchor::Middle),
            to: (1, Anchor::Bottom),
            lane: 1
        }));
        assert_eq!(rows[1].column, 1); // feature commit on the side lane
        assert_eq!(rows[2].column, 0); // main continues, side lane passes by
        assert!(rows[2].segments.contains(&Segment {
            from: (1, Anchor::Top),
            to: (1, Anchor::Bottom),
            lane: 1
        }));
        // Both lanes converge into the root.
        assert_eq!(rows[3].column, 0);
        assert!(rows[3].segments.contains(&Segment {
            from: (1, Anchor::Top),
            to: (0, Anchor::Middle),
            lane: 1
        }));
        assert_eq!(rows[3].width, 2);
    }

    #[test]
    fn independent_tips_get_their_own_lanes() {
        let rows = run(&[("x", &["a"]), ("y", &["a"]), ("a", &[])]);
        assert_eq!((rows[0].column, rows[1].column, rows[2].column), (0, 1, 0));
    }
}
