//! Ref chips: classifying the ref names `ocelli-git` attaches to a node.
//!
//! `ocelli-git` decorates names for display — `🏷 ` before a tag, `📦 ` before a
//! stash. Those emoji are double-width in most terminals, so the prefix is read
//! as the ref kind and stripped from the label (ADR-0028, Consequences).

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// The checked-out local branch.
    Head,
    Local,
    Remote,
    Tag,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub kind: RefKind,
    pub label: String,
}

/// What a chip needs to know about the repository beyond the ref name.
#[derive(Debug, Default)]
pub struct RefContext {
    pub locals: HashSet<String>,
    pub head: Option<String>,
}

fn classify(name: &str, ctx: &RefContext) -> Chip {
    let chip = |kind, label: &str| Chip {
        kind,
        label: label.to_string(),
    };
    if let Some(tag) = name.strip_prefix("🏷") {
        return chip(RefKind::Tag, tag.trim_start());
    }
    if let Some(stash) = name.strip_prefix("📦") {
        return chip(RefKind::Stash, stash.trim_start());
    }
    // The local-branch tests run before the slash test: `feat/x` is a local
    // branch, not a remote one.
    if ctx.head.as_deref() == Some(name) {
        return chip(RefKind::Head, name);
    }
    if ctx.locals.contains(name) || !name.contains('/') {
        return chip(RefKind::Local, name);
    }
    chip(RefKind::Remote, name)
}

/// The chips for one node, with the checked-out branch moved to the front so it
/// is never the one cut off by a narrow terminal.
pub fn chips(refs: &[String], ctx: &RefContext) -> Vec<Chip> {
    let mut chips: Vec<Chip> = refs.iter().map(|r| classify(r, ctx)).collect();
    chips.sort_by_key(|c| c.kind != RefKind::Head);
    chips
}

/// A chip as it will be drawn in the refs column. `kind` is `None` for the
/// `+N` badge standing in for chips that did not fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub kind: Option<RefKind>,
    pub label: String,
}

/// Cells a chip takes: its label padded by one cell on each side.
pub fn chip_width(label: &str) -> usize {
    label.chars().count() + 2
}

/// Cells between two chips.
const GAP: usize = 1;

/// The text drawn in a chip. The checked-out branch carries a tick, so it says
/// what it is even where its colour is hard to judge.
fn label(chip: &Chip) -> String {
    match chip.kind {
        RefKind::Head => format!("✓ {}", chip.label),
        _ => chip.label.clone(),
    }
}

fn truncate(text: &str, cells: usize) -> String {
    if text.chars().count() <= cells {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cells.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Decides which of a row's chips fit into `avail` cells — the desktop's
/// `planRefChips`. Chips stay in order and those that do not fit collapse into
/// one `+N` badge, so a commit with many refs still shows that they exist. The
/// leading chip (the checked-out branch, when present) is shortened before it
/// is dropped: a row showing nothing but a count says no more than an empty one.
pub fn plan(chips: &[Chip], avail: usize) -> Vec<Planned> {
    let decorated: Vec<Planned> = chips
        .iter()
        .map(|c| Planned {
            kind: Some(c.kind),
            label: label(c),
        })
        .collect();
    let mut plan: Vec<Planned> = Vec::new();
    let mut used = 0;
    for chip in &decorated {
        let gap = if plan.is_empty() { 0 } else { GAP };
        let width = chip_width(&chip.label);
        if used + gap + width > avail {
            break;
        }
        plan.push(chip.clone());
        used += gap + width;
    }

    let mut hidden = decorated.len() - plan.len();
    // Make room for the badge: trailing chips go first; once only the leading
    // one is left it is shortened instead, so the row keeps a name on it.
    loop {
        if hidden == 0 {
            return plan;
        }
        let badge = format!("+{hidden}");
        let badge_width = chip_width(&badge);
        if !plan.is_empty() && used + GAP + badge_width <= avail {
            plan.push(Planned {
                kind: None,
                label: badge,
            });
            return plan;
        }
        if plan.len() > 1 {
            let dropped = plan.pop().unwrap();
            used -= chip_width(&dropped.label) + GAP;
            hidden += 1;
            continue;
        }
        let room = avail.saturating_sub(badge_width + GAP + 2);
        if room < 4 {
            // Not even a shortened chip and a badge fit: the count is all there is.
            return vec![Planned {
                kind: None,
                label: format!("+{}", decorated.len()),
            }];
        }
        let first = &decorated[0];
        let shortened = truncate(&first.label, room);
        used = chip_width(&shortened);
        plan = vec![Planned {
            kind: first.kind,
            label: shortened,
        }];
        hidden = decorated.len() - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RefContext {
        RefContext {
            locals: ["main", "feat/x"].iter().map(|s| s.to_string()).collect(),
            head: Some("feat/x".to_string()),
        }
    }

    fn kinds(names: &[&str]) -> Vec<(RefKind, String)> {
        let refs: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        chips(&refs, &ctx())
            .into_iter()
            .map(|c| (c.kind, c.label))
            .collect()
    }

    #[test]
    fn decorated_names_become_kinds_without_their_emoji() {
        assert_eq!(
            kinds(&["🏷 v1.0", "📦 stash@{0}"]),
            [
                (RefKind::Tag, "v1.0".to_string()),
                (RefKind::Stash, "stash@{0}".to_string())
            ]
        );
    }

    #[test]
    fn a_local_branch_with_a_slash_is_not_a_remote() {
        assert_eq!(
            kinds(&["origin/main", "main"]),
            [
                (RefKind::Remote, "origin/main".to_string()),
                (RefKind::Local, "main".to_string())
            ]
        );
    }

    #[test]
    // Verifies: G-11
    fn the_checked_out_branch_comes_first() {
        assert_eq!(
            kinds(&["origin/feat/x", "feat/x"]),
            [
                (RefKind::Head, "feat/x".to_string()),
                (RefKind::Remote, "origin/feat/x".to_string())
            ]
        );
    }

    fn planned(names: &[&str], avail: usize) -> Vec<String> {
        let refs: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        plan(&chips(&refs, &ctx()), avail)
            .into_iter()
            .map(|p| p.label)
            .collect()
    }

    #[test]
    fn chips_that_fit_are_shown_in_full() {
        assert_eq!(planned(&["main", "🏷 v1"], 20), ["main", "v1"]);
    }

    #[test]
    // Verifies: G-11
    fn surplus_chips_collapse_into_a_count() {
        // main(6) v1(4) v2(4) v3(4) with gaps: main + v1 + "+2"(4) = 6+1+4+1+4 = 16.
        assert_eq!(
            planned(&["main", "🏷 v1", "🏷 v2", "🏷 v3"], 16),
            ["main", "v1", "+2"]
        );
    }

    #[test]
    fn the_leading_chip_is_shortened_before_it_is_dropped() {
        assert_eq!(
            planned(&["feat/x", "origin/feat/x"], 12),
            ["✓ fe…", "+1"],
            "the checked-out branch leads and keeps a name"
        );
    }

    #[test]
    fn with_no_room_at_all_only_the_count_remains() {
        assert_eq!(planned(&["main", "🏷 v1"], 6), ["+2"]);
    }
}
