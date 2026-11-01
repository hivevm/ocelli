//! Drawing. The layout follows the desktop window: a title bar with the open,
//! refresh and search controls; the repository sidebar, the graph and the
//! commit detail with its changed files side by side; a status bar below. Only
//! the visible window of the graph is rasterised each frame (ADR-0020);
//! everything else is read from the [`App`] state.

use crate::app::{App, Focus, Modal, StashPrompt, TagPrompt};
use crate::chips::{Planned, RefKind, chip_width, chips, plan};
use crate::diffsplit::{CellKind, SplitRow, split_rows};
use crate::files::{FileRow, FileView};
use crate::merge::{LineKind, MergeEditor};
use crate::raster::{self, Glyphs, rasterize};
use crate::sidebar::{self, Look, RailRow, RowKind};
use crate::theme::{Palette, lane, palette};
use ocelli_core::model::{CommitKind, DiffLine, DiffLineKind, FileStatus, Staged};
use ocelli_core::port::{ResetMode, Write};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// Width of the branch/tag column left of the graph lanes.
const REFS_WIDTH: usize = 22;
const SIDEBAR_WIDTH: u16 = 32;
const RAIL_WIDTH: u16 = 7;
/// The narrowest terminal the layout is designed for, which the status bar's hints are held to.
/// At 80 columns the sidebar and the branch/tag column leave the graph pane five columns for its
/// lanes, short SHA, and message; at 100 they leave it eighteen.
#[cfg(test)]
const DESIGN_WIDTH: u16 = 100;
/// Columns the status bar keeps for the status text left of the hint, enough for a count of
/// commits or the name of a write running.
const STATUS_ROOM: u16 = 20;
/// Search results beyond this are reachable by cycling, not listed.
const MAX_PANEL_ROWS: usize = 200;

fn pane<'a>(p: &Palette, title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
        .border_style(Style::new().fg(if focused { p.accent } else { p.border }))
        .style(Style::new().bg(p.bg).fg(p.text))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let p = palette(app.theme);
    frame.render_widget(
        Block::new().style(Style::new().bg(p.bg).fg(p.text)),
        frame.area(),
    );

    let [title, main, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    if let Some(editor) = app.merge.as_mut() {
        draw_merge(frame, editor, p, title.union(main));
        draw_status(frame, app, p, status);
        return;
    }
    let side_width = if app.side.minimised {
        RAIL_WIDTH
    } else {
        SIDEBAR_WIDTH
    };
    let [side, rest] =
        Layout::horizontal([Constraint::Length(side_width), Constraint::Fill(1)]).areas(main);
    let [graph, right] =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Fill(1)]).areas(rest);
    let [detail, files] =
        Layout::vertical([Constraint::Length(10), Constraint::Fill(1)]).areas(right);

    draw_title(frame, app, p, title);
    draw_sidebar(frame, app, p, side);
    draw_graph(frame, app, p, graph);
    draw_detail(frame, app, p, detail);
    draw_files(frame, app, p, files);
    if app.diff.is_some() {
        draw_diff(frame, app, p, graph);
    }
    if app.search.editing {
        draw_search_panel(frame, app, p, graph);
    }
    match &app.modal {
        Some(Modal::Open(path)) => draw_prompt(frame, path, p, main),
        Some(Modal::Tag(tag)) => draw_tag_prompt(frame, tag, p, main),
        Some(Modal::Stash(stash)) => draw_stash_prompt(frame, stash, p, main),
        Some(Modal::Confirm(write)) => draw_confirm(frame, write, p, main),
        Some(Modal::Reset { commit, mode }) => {
            let short = &commit[..commit.len().min(7)];
            let mut spans = vec![Span::styled("Mode  ", Style::new().fg(p.text_muted))];
            for (each, name) in [
                (ResetMode::Soft, "soft"),
                (ResetMode::Mixed, "mixed"),
                (ResetMode::Hard, "hard"),
            ] {
                let style = if each == *mode {
                    Style::new().fg(p.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(p.text_dim)
                };
                spans.push(Span::styled(format!("[{name}] "), style));
            }
            let lines = vec![
                Line::from(spans),
                Line::styled(
                    "Tab mode · Enter reset, a hard one asked first · Esc cancel",
                    Style::new().fg(p.text_dim),
                ),
            ];
            draw_dialog(
                frame,
                p,
                main,
                format!(" Reset the branch to {short} "),
                lines,
            );
        }
        Some(Modal::Conflicts { selected }) => draw_conflicts(frame, app, *selected, p, main),
        Some(Modal::Branch {
            commit,
            name,
            switch,
        }) => {
            let short = &commit[..commit.len().min(7)];
            let switch = if *switch { "[x]" } else { "[ ]" };
            let lines = vec![
                input_line(p, "Name    ", name, true),
                Line::from(vec![
                    Span::styled("Switch  ", Style::new().fg(p.text_muted)),
                    Span::styled(
                        format!("{switch} switch to it"),
                        Style::new().fg(p.text_bright),
                    ),
                ]),
                Line::styled(
                    "Enter create · Tab switch · Esc cancel",
                    Style::new().fg(p.text_dim),
                ),
            ];
            draw_dialog(frame, p, main, format!(" Branch at {short} "), lines);
        }
        Some(Modal::Commit { head, message }) => {
            let short = &head[..head.len().min(7)];
            let lines = vec![
                input_line(p, "Message  ", message, true),
                Line::styled(
                    "Enter commit the staged files · Esc cancel",
                    Style::new().fg(p.text_dim),
                ),
            ];
            draw_dialog(frame, p, main, format!(" Commit on {short} "), lines);
        }
        None => {}
    }
    draw_status(frame, app, p, status);
}

// ── Title bar ───────────────────────────────────────────────────────────────

fn draw_title(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let key = |k: &'static str| Span::styled(k, Style::new().fg(p.accent));
    let dim = |t: &'static str| Span::styled(t, Style::new().fg(p.text_muted));
    let left = Line::from(vec![
        Span::styled(
            " ◉ Ocelli ",
            Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
        ),
        key(" o"),
        dim(" Open  "),
        key("r"),
        dim(" Refresh   "),
        Span::styled("⎇ ", Style::new().fg(p.accent)),
        Span::styled(
            app.repo.display().to_string(),
            Style::new().fg(p.text_bright),
        ),
    ]);

    let s = &app.search;
    let search = if s.editing || !s.query.is_empty() {
        let count = match (s.pos, s.matches.len()) {
            (_, 0) if !s.query.trim().is_empty() => " no matches".to_string(),
            (Some(i), n) => format!(" {}/{n}", i + 1),
            (None, n) if n > 0 => format!(" {n}"),
            _ => String::new(),
        };
        let cursor = if s.editing { "▏" } else { "" };
        Line::from(vec![
            Span::styled("⌕ ", Style::new().fg(p.text_muted)),
            Span::styled(
                format!("{}{cursor}", s.query),
                Style::new().fg(p.text_bright),
            ),
            Span::styled(count, Style::new().fg(p.text_muted)),
            Span::raw(" "),
        ])
    } else {
        Line::from(vec![
            Span::styled("⌕ ", Style::new().fg(p.text_muted)),
            dim("Ctrl+F search "),
        ])
    };
    let width = search.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    let bar = Style::new().bg(p.bg_chrome);
    frame.render_widget(Paragraph::new(left).style(bar), l);
    frame.render_widget(Paragraph::new(search).style(bar), r);
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

fn look_style(p: &Palette, look: Look) -> Style {
    match look {
        Look::Parent | Look::Submodule => Style::new().fg(p.teal),
        Look::Head => Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
        Look::Local => Style::new().fg(p.accent),
        Look::Remote => Style::new().fg(p.blue),
        Look::Tag => Style::new().fg(p.amber),
        Look::Worktree => Style::new().fg(p.purple),
    }
}

fn draw_sidebar(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let focused = app.focus == Focus::Sidebar && app.diff.is_none();
    let Some(data) = &app.data else {
        frame.render_widget(pane(p, "", focused), area);
        return;
    };
    let highlight = if focused {
        Style::new().bg(p.bg_sel).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };

    if app.side.minimised {
        // The rail: glyphs and counts, the way back in.
        let items: Vec<ListItem> = sidebar::rail(&data.sidebar)
            .iter()
            .map(|r| match r {
                RailRow::Parent(_) => ListItem::new(Line::styled(" ↑", Style::new().fg(p.teal))),
                RailRow::Section { section, count } => ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", section.glyph()), Style::new().fg(p.accent)),
                    Span::styled(count.to_string(), Style::new().fg(p.text_muted)),
                ])),
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(app.side.rail_selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(pane(p, "", focused))
                .highlight_style(highlight),
            area,
            &mut state,
        );
        return;
    }

    let block = pane(p, " Repository ", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [filter, list] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let filter_line = if app.side.editing || !app.side.filter.is_empty() {
        let cursor = if app.side.editing { "▏" } else { "" };
        Line::from(vec![
            Span::styled(" ⌕ ", Style::new().fg(p.text_muted)),
            Span::styled(
                format!("{}{cursor}", app.side.filter),
                Style::new().fg(p.text_bright),
            ),
        ])
    } else {
        Line::styled(" ⌕ / filter", Style::new().fg(p.text_dim))
    };
    frame.render_widget(Paragraph::new(filter_line), filter);

    let items: Vec<ListItem> = sidebar::rows(&data.sidebar, &app.side)
        .into_iter()
        .map(|row| {
            let indent = " ".repeat(1 + 2 * row.depth);
            let line = match row.kind {
                RowKind::Section { open, count, .. } => Line::from(vec![
                    Span::raw(indent),
                    Span::styled(
                        format!("{} {}", if open { "▾" } else { "▸" }, row.label),
                        Style::new().fg(p.text_muted).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {count}"), Style::new().fg(p.text_dim)),
                ]),
                RowKind::Group {
                    open,
                    count,
                    has_head,
                    ..
                } => {
                    let color = if has_head { p.accent } else { p.text };
                    Line::from(vec![
                        Span::raw(indent),
                        Span::styled(
                            format!("{} {}", if open { "▾" } else { "▸" }, row.label),
                            Style::new().fg(color),
                        ),
                        Span::styled(format!(" {count}"), Style::new().fg(p.text_dim)),
                    ])
                }
                RowKind::Leaf(item) => {
                    let style = if data.can_act(item) {
                        look_style(p, item.look)
                    } else {
                        // Rows that cannot act stay, greyed; Enter says why.
                        Style::new().fg(p.text_dim)
                    };
                    let tick = if item.look == Look::Head { "✓ " } else { "" };
                    let mut spans = vec![
                        Span::raw(indent),
                        Span::styled(format!("{tick}{}", row.label), style),
                    ];
                    if !item.badge.is_empty() {
                        spans.push(Span::styled(
                            format!(" {}", item.badge),
                            Style::new().fg(p.text_muted),
                        ));
                    }
                    if let Some(mark) = item.mark {
                        spans.push(Span::styled(
                            format!(" {mark}"),
                            Style::new().fg(p.accent).add_modifier(Modifier::ITALIC),
                        ));
                    }
                    Line::from(spans)
                }
            };
            ListItem::new(line)
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.side.selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(highlight),
        list,
        &mut state,
    );
}

// ── Graph ───────────────────────────────────────────────────────────────────

fn chip_spans(p: &Palette, planned: Vec<Planned>, dim: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, chip) in planned.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let c = match chip.kind {
            Some(RefKind::Head) => p.chip_head,
            Some(RefKind::Local) => p.chip_local,
            Some(RefKind::Remote) => p.chip_remote,
            Some(RefKind::Tag) => p.chip_tag,
            Some(RefKind::Stash) => p.chip_stash,
            None => p.chip_more,
        };
        let mut style = Style::new()
            .fg(if dim { p.text_dim } else { c.fg })
            .bg(c.bg);
        if chip.kind == Some(RefKind::Head) {
            style = style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(format!(" {} ", chip.label), style));
    }
    spans
}

fn draw_graph(frame: &mut Frame, app: &mut App, p: &Palette, area: Rect) {
    let focused = app.focus == Focus::Graph && app.diff.is_none();
    let block = pane(p, "", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [header, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    app.scroll_to_selection(body.height as usize);

    let Some(data) = app.data.as_mut() else {
        return;
    };
    // The graph column is as wide as the most lanes shown, so older history arriving with more
    // lanes moves nothing until the developer scrolls to it.
    let window = data.history().window(app.offset, body.height as usize);
    data.shown_lanes = data.shown_lanes.max(window.lanes());
    let width = raster::width(data.shown_lanes);
    let data = &*data;
    let graph_width = width.max(5);
    let head = Style::new().fg(p.text_dim).add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{:>w$} ", "Branch / Tag", w = REFS_WIDTH), head),
            Span::styled(format!("{:<w$} ", "Graph", w = graph_width), head),
            Span::styled("SHA     ", head),
            Span::styled("Description", head),
        ])),
        header,
    );

    let lineage = app.lineage.as_ref().filter(|_| app.active);
    let cells = rasterize(&window, width, app.glyphs, lineage);
    let lines: Vec<Line> = cells
        .iter()
        .enumerate()
        .map(|(i, row_cells)| {
            let row = app.offset + i;
            let node = data.node(row);
            let selected = app.active && row == app.selected;
            let dim = lineage.is_some_and(|l| !l.contains(row));

            // Refs column, right-aligned so the chips end where the lanes begin.
            let planned = plan(&chips(&node.refs, &data.refs), REFS_WIDTH);
            let used: usize = planned.iter().map(|c| chip_width(&c.label)).sum::<usize>()
                + planned.len().saturating_sub(1);
            let mut spans = vec![Span::raw(" ".repeat(REFS_WIDTH.saturating_sub(used)))];
            spans.extend(chip_spans(p, planned, dim));
            spans.push(Span::raw(" "));

            for c in row_cells {
                spans.push(match c.lane {
                    Some(l) => Span::styled(
                        c.ch.to_string(),
                        Style::new().fg(if c.dim { p.faint } else { lane(l) }),
                    ),
                    None => Span::raw(" "),
                });
            }
            spans.push(Span::raw(" ".repeat(graph_width - row_cells.len() + 1)));

            let sha_color = if selected {
                p.blue
            } else if dim {
                p.faint
            } else {
                p.sha
            };
            spans.push(Span::styled(
                format!("{:<7} ", node.short_sha),
                Style::new().fg(sha_color),
            ));
            let msg_color = if selected {
                p.text_bright
            } else if dim {
                p.text_dim
            } else {
                p.text
            };
            let mut msg = Style::new().fg(msg_color);
            if node.kind == CommitKind::Working {
                msg = msg.add_modifier(Modifier::ITALIC);
            }
            spans.push(Span::styled(node.message.clone(), msg));

            let line = Line::from(spans);
            if selected {
                line.style(Style::new().bg(p.bg_sel))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

// ── Commit detail ───────────────────────────────────────────────────────────

fn draw_detail(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let focused = app.focus == Focus::Detail && app.diff.is_none();
    let copyable = app
        .selected_node()
        .is_some_and(|n| n.kind != CommitKind::Working);
    let title = Line::from(vec![
        Span::raw(" Commit "),
        Span::styled(
            if copyable { "· y copy SHA " } else { "" },
            Style::new().fg(p.text_dim),
        ),
    ]);
    let block = pane(p, title, focused);
    let (Some(data), Some(node)) = (&app.data, app.selected_node()) else {
        frame.render_widget(
            Paragraph::new(Line::styled(
                "No commit selected",
                Style::new().fg(p.text_dim),
            ))
            .block(block),
            area,
        );
        return;
    };
    let label = |s: &'static str| Span::styled(format!("{s:<10}"), Style::new().fg(p.text_muted));
    let working = node.kind == CommitKind::Working;

    // Parent and child links; the focused pane highlights the selected one.
    let mut link_index = 0;
    let mut link_line = |name: &'static str, shas: &[String]| {
        let mut spans = vec![label(name)];
        if shas.is_empty() {
            spans.push(Span::styled("—", Style::new().fg(p.text_dim)));
        }
        for sha in shas {
            let loaded = data.row_of(sha).is_some();
            let mut style = Style::new().fg(if loaded { p.blue } else { p.text_dim });
            if focused && link_index == app.link {
                style = style
                    .bg(p.bg_sel)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
            spans.push(Span::styled(data.link_label(sha), style));
            spans.push(Span::raw(" "));
            link_index += 1;
        }
        Line::from(spans)
    };
    let parents = link_line("Parents", &node.parents);
    let children = link_line("Children", &node.children);

    let sha = if working {
        Span::styled("uncommitted changes", Style::new().fg(p.text_dim))
    } else {
        Span::styled(node.sha.clone(), Style::new().fg(p.blue))
    };
    let sha_line = vec![label("SHA"), sha];
    let mut lines = vec![
        Line::styled(
            node.message.clone(),
            Style::new().fg(p.text_bright).add_modifier(Modifier::BOLD),
        ),
        Line::from(sha_line),
        Line::from(vec![
            label("Author"),
            Span::raw(format!("{}  ", node.author)),
            Span::styled(node.author_date.clone(), Style::new().fg(p.text_muted)),
        ]),
    ];
    if !node.committer.is_empty() {
        lines.push(Line::from(vec![
            label("Committer"),
            Span::raw(format!("{}  ", node.committer)),
            Span::styled(node.date.clone(), Style::new().fg(p.text_muted)),
        ]));
    }
    lines.push(parents);
    lines.push(children);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── Changed files ───────────────────────────────────────────────────────────

fn status_letter(p: &Palette, status: &FileStatus) -> Span<'static> {
    let (letter, color) = match status {
        FileStatus::Added => ("A", p.accent),
        FileStatus::Modified => ("M", p.blue),
        FileStatus::Deleted => ("D", p.red),
        FileStatus::Renamed => ("R", p.amber),
        FileStatus::Copied => ("C", p.teal),
        FileStatus::Unknown => ("?", p.text_dim),
    };
    Span::styled(
        format!("{letter} "),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Whether the files shown are the working-directory node's, which stage.
fn working(app: &App) -> bool {
    app.files
        .as_ref()
        .is_some_and(|f| f.sha == ocelli_core::model::WORKDIR_SHA)
}

/// How much of a file's change is staged: all, part, or none (G-12).
fn staged_mark(p: &Palette, glyphs: Glyphs, staged: Staged) -> Span<'static> {
    let (unicode, ascii, colour) = match staged {
        Staged::Yes => ("● ", "[x] ", p.accent),
        Staged::Partly => ("◐ ", "[~] ", p.amber),
        Staged::No => ("○ ", "[ ] ", p.text_dim),
    };
    let mark = if glyphs == Glyphs::Ascii {
        ascii
    } else {
        unicode
    };
    Span::styled(mark, Style::new().fg(colour))
}

fn draw_files(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let focused = app.focus == Focus::Files && app.diff.is_none();
    let view = match app.file_view {
        FileView::Tree => "tree",
        FileView::Flat => "flat",
    };
    let count = match app.files.as_ref().map(|f| &f.files) {
        Some(Ok(list)) => format!(" ({})", list.len()),
        _ => String::new(),
    };
    let title = Line::from(vec![
        Span::raw(format!(" Changed files{count} ")),
        Span::styled(format!("· {view} (t) "), Style::new().fg(p.text_dim)),
    ]);
    let block = pane(p, title, focused);
    let message = |text: String, color| Paragraph::new(Line::styled(text, Style::new().fg(color)));
    let Some(files) = &app.files else {
        let text = if app.active { "Loading…" } else { "" };
        frame.render_widget(message(text.into(), p.text_dim).block(block), area);
        return;
    };
    let list = match &files.files {
        Err(e) => {
            frame.render_widget(message(format!("Error: {e}"), p.red).block(block), area);
            return;
        }
        Ok(list) if list.is_empty() => {
            frame.render_widget(message("No changes".into(), p.text_dim).block(block), area);
            return;
        }
        Ok(list) => list,
    };
    let open = app.diff.as_ref().map(|d| d.path.as_str());
    let items: Vec<ListItem> = app
        .file_rows()
        .into_iter()
        .map(|row| match row {
            FileRow::Dir {
                depth,
                name,
                file_count,
                open: expanded,
                ..
            } => ListItem::new(Line::from(vec![
                Span::raw("  ".repeat(depth)),
                Span::styled(
                    format!("{} {name}/", if expanded { "▾" } else { "▸" }),
                    Style::new().fg(p.text_muted),
                ),
                Span::styled(format!(" {file_count}"), Style::new().fg(p.text_dim)),
            ])),
            FileRow::File { depth, name, index } => {
                let f = &list[index];
                let mut name_style = Style::new().fg(p.text);
                if open == Some(f.path.as_str()) {
                    name_style = name_style.fg(p.accent).add_modifier(Modifier::BOLD);
                }
                let mut spans = vec![Span::raw("  ".repeat(depth))];
                if let Some(staging) = &f.staging {
                    spans.push(staged_mark(p, app.glyphs, staging.staged));
                }
                spans.extend([status_letter(p, &f.status), Span::styled(name, name_style)]);
                if f.additions > 0 {
                    spans.push(Span::styled(
                        format!(" +{}", f.additions),
                        Style::new().fg(p.accent),
                    ));
                }
                if f.deletions > 0 {
                    spans.push(Span::styled(
                        format!(" -{}", f.deletions),
                        Style::new().fg(p.red),
                    ));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();
    let highlight = if focused {
        Style::new().bg(p.bg_sel).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let mut state = ListState::default().with_selected(Some(files.selected));
    frame.render_stateful_widget(
        List::new(items).block(block).highlight_style(highlight),
        area,
        &mut state,
    );
}

// ── Diff overlay ────────────────────────────────────────────────────────────

fn lineno(n: Option<u32>) -> String {
    n.map_or("    ".to_string(), |n| format!("{n:>4}"))
}

fn clean(content: &str) -> String {
    content.trim_end_matches(['\n', '\r']).replace('\t', "    ")
}

fn unified_line<'a>(p: &Palette, l: &DiffLine) -> Line<'a> {
    let gutter = Span::styled(
        format!("{} {} │ ", lineno(l.old_lineno), lineno(l.new_lineno)),
        Style::new().fg(p.text_dim),
    );
    let style = match l.kind {
        DiffLineKind::Add => Style::new().fg(p.accent).bg(p.add_bg),
        DiffLineKind::Delete => Style::new().fg(p.red).bg(p.del_bg),
        DiffLineKind::Hunk => Style::new().fg(p.teal),
        DiffLineKind::Meta => Style::new().fg(p.text_dim),
        DiffLineKind::Context => Style::new().fg(p.text),
    };
    Line::from(vec![gutter, Span::styled(clean(&l.content), style)])
}

/// One half of a side-by-side row, padded or clipped to `width` cells.
fn split_cell(p: &Palette, cell: &crate::diffsplit::Cell, width: usize) -> Vec<Span<'static>> {
    let style = match cell.kind {
        CellKind::Add => Style::new().fg(p.accent).bg(p.add_bg),
        CellKind::Delete => Style::new().fg(p.red).bg(p.del_bg),
        CellKind::Context => Style::new().fg(p.text),
        CellKind::Empty => Style::new().bg(p.bg_chrome),
    };
    let text_width = width.saturating_sub(7);
    let mut text: String = clean(&cell.content).chars().take(text_width).collect();
    let pad = text_width.saturating_sub(text.chars().count());
    text.push_str(&" ".repeat(pad));
    vec![
        Span::styled(
            format!("{} │ ", lineno(cell.lineno)),
            Style::new().fg(p.text_dim),
        ),
        Span::styled(text, style),
    ]
}

fn draw_diff(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let Some(diff) = &app.diff else { return };
    let mode = if app.diff_split {
        "side-by-side"
    } else {
        "unified"
    };
    let title = Line::from(vec![
        Span::styled(
            format!(" {} ", diff.path),
            Style::new().fg(p.text_bright).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("· {mode} (s) "), Style::new().fg(p.text_dim)),
    ]);
    let block = pane(p, title, true);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    let lines: Vec<Line> = match &diff.lines {
        None => vec![Line::styled("Loading diff…", Style::new().fg(p.text_dim))],
        Some(Err(e)) => vec![Line::styled(format!("Error: {e}"), Style::new().fg(p.red))],
        Some(Ok(lines)) if app.diff_split => {
            let half = (inner.width as usize).saturating_sub(1) / 2;
            split_rows(lines)
                .iter()
                .skip(diff.scroll)
                .take(height)
                .map(|row| match row {
                    SplitRow::Banner { kind, content } => {
                        let color = if *kind == DiffLineKind::Hunk {
                            p.teal
                        } else {
                            p.text_dim
                        };
                        Line::styled(clean(content), Style::new().fg(color))
                    }
                    SplitRow::Line { left, right } => {
                        let mut spans = split_cell(p, left, half);
                        spans.push(Span::styled("│", Style::new().fg(p.border)));
                        spans.extend(split_cell(p, right, half));
                        Line::from(spans)
                    }
                })
                .collect()
        }
        Some(Ok(lines)) => lines
            .iter()
            .skip(diff.scroll)
            .take(height)
            .map(|l| unified_line(p, l))
            .collect(),
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Search results, open prompt ─────────────────────────────────────────────

/// The results panel: open while the field has focus and a query has matches.
fn draw_search_panel(frame: &mut Frame, app: &App, p: &Palette, graph: Rect) {
    let Some(data) = &app.data else { return };
    let s = &app.search;
    if s.query.trim().is_empty() || s.matches.is_empty() {
        return;
    }
    let shown = s.matches.len().min(MAX_PANEL_ROWS);
    let more = s.matches.len() - shown;
    let height = (shown + usize::from(more > 0) + 2).min(14) as u16;
    let width = graph.width.min(70);
    let area = Rect {
        x: graph.x + graph.width - width,
        y: graph.y,
        width,
        height: height.min(graph.height),
    };
    let mut items: Vec<ListItem> = s.matches[..shown]
        .iter()
        .map(|&row| {
            let n = data.node(row);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<7} ", n.short_sha), Style::new().fg(p.sha)),
                Span::raw(n.message.clone()),
                Span::styled(format!("  {}", n.author), Style::new().fg(p.text_muted)),
            ]))
        })
        .collect();
    if more > 0 {
        items.push(ListItem::new(Line::styled(
            format!("+{more} more — Enter cycles through all"),
            Style::new().fg(p.text_dim),
        )));
    }
    let mut state = ListState::default().with_selected(s.pos.filter(|&i| i < shown));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(pane(p, " Matches ", true))
            .highlight_style(Style::new().bg(p.bg_sel).add_modifier(Modifier::BOLD)),
        area,
        &mut state,
    );
}

fn draw_prompt(frame: &mut Frame, text: &str, p: &Palette, main: Rect) {
    let lines = vec![
        input_line(p, "", text, true),
        Line::styled("Enter open · Esc cancel", Style::new().fg(p.text_dim)),
    ];
    draw_dialog(frame, p, main, " Open repository ".to_string(), lines);
}

/// The tag prompt (G-16): the name, then the message, the line being typed
/// carrying the cursor.
fn draw_tag_prompt(frame: &mut Frame, tag: &TagPrompt, p: &Palette, main: Rect) {
    let typing_name = tag.message.is_none();
    let mut lines = vec![input_line(p, "Name     ", &tag.name, typing_name)];
    if let Some(message) = &tag.message {
        lines.push(input_line(p, "Message  ", message, true));
    }
    lines.push(Line::styled(
        if typing_name {
            "Enter next · Esc cancel"
        } else {
            "Enter tag, an empty message makes a lightweight tag · Esc cancel"
        },
        Style::new().fg(p.text_dim),
    ));
    let short = &tag.sha[..tag.sha.len().min(7)];
    draw_dialog(frame, p, main, format!(" Tag {short} "), lines);
}

/// The question before a write that discards work, naming what is lost
/// (ADR-0033).
fn draw_confirm(frame: &mut Frame, write: &Write, p: &Palette, main: Rect) {
    let Some(lost) = write.discards() else {
        return;
    };
    let lines = vec![
        Line::styled(
            format!("This discards {lost}, and nothing brings it back."),
            Style::new().fg(p.text_bright),
        ),
        Line::from(vec![
            Span::styled("y", Style::new().fg(p.accent)),
            Span::styled(
                " go ahead · any other key keeps it",
                Style::new().fg(p.text_dim),
            ),
        ]),
    ];
    draw_dialog(frame, p, main, " Discard? ".to_string(), lines);
}

/// The stash prompt (G-16): the message, and whether the untracked files go
/// with the changes.
fn draw_stash_prompt(frame: &mut Frame, stash: &StashPrompt, p: &Palette, main: Rect) {
    let untracked = if stash.untracked { "[x]" } else { "[ ]" };
    let lines = vec![
        input_line(p, "Message    ", &stash.message, true),
        Line::from(vec![
            Span::styled("Untracked  ", Style::new().fg(p.text_muted)),
            Span::styled(
                format!("{untracked} include untracked files"),
                Style::new().fg(p.text_bright),
            ),
        ]),
        Line::styled(
            "Enter stash · Tab untracked · Esc cancel",
            Style::new().fg(p.text_dim),
        ),
    ];
    draw_dialog(frame, p, main, " Stash changes ".to_string(), lines);
}

fn input_line<'a>(p: &Palette, label: &'a str, text: &str, cursor: bool) -> Line<'a> {
    Line::from(vec![
        Span::styled(label, Style::new().fg(p.text_muted)),
        Span::styled(text.to_string(), Style::new().fg(p.text_bright)),
        Span::styled(if cursor { "▏" } else { "" }, Style::new().fg(p.accent)),
    ])
}

/// A box over the panes, a third of the way down.
fn draw_dialog(frame: &mut Frame, p: &Palette, main: Rect, title: String, lines: Vec<Line>) {
    let width = main.width.min(80);
    let area = Rect {
        x: main.x + (main.width - width) / 2,
        y: main.y + main.height / 3,
        width,
        height: (lines.len() as u16 + 2).min(main.height),
    };
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(pane(p, title, true)), area);
}

// ── Status bar ──────────────────────────────────────────────────────────────

/// The keys of the state the application is in. Each fits beside the theme and quit keys and
/// [`STATUS_ROOM`] in the width the layout is designed for, so a hint names the keys that matter
/// most there and groups the rest; search is on the title bar and is not repeated here.
fn hints(app: &App) -> &'static str {
    if app.merge.is_some() {
        return "M-o/t/b · M-n/p · M-a base · ^S save · ^U undo · Esc leave";
    }
    match &app.modal {
        Some(Modal::Open(_)) => return "type a path",
        Some(Modal::Confirm(_)) => return "y goes ahead · any other key keeps it",
        Some(Modal::Tag(tag)) if tag.message.is_none() => return "type the tag's name",
        Some(Modal::Tag(_) | Modal::Stash(_)) => return "type a message, or none",
        Some(Modal::Commit { .. }) => return "type the commit's message",
        Some(Modal::Branch { .. }) => return "type the branch's name",
        Some(Modal::Reset { .. }) => return "Tab soft/mixed/hard · Enter reset",
        Some(Modal::Conflicts { .. }) => {
            return "j/k move · o/t/e resolve · m edit · c continue · A abort";
        }
        None => {}
    }
    if app.search.editing {
        return "Enter/⇧Enter next/prev · ↑/↓ · Tab keep · Esc clear";
    }
    if app.side.editing {
        return "type to filter · Enter done · Esc clear";
    }
    if app.diff.is_some() {
        return "j/k scroll · s split · Esc close";
    }
    match app.focus {
        Focus::Sidebar if app.side.minimised => "j/k move · Enter expand · 1 expand",
        Focus::Sidebar => "Enter open · w switch · d delete · / filter · 1 minimise",
        Focus::Graph => match app.selected_node().map(|n| n.kind) {
            Some(CommitKind::Working) => "j/k move · c commit · s stash · Esc deselect",
            Some(CommitKind::Stash) => "j/k move · a/p/d apply/pop/drop · Esc deselect",
            _ => "t/b/w tag/branch/switch · m/R/C/V/X history · y copy",
        },
        Focus::Detail => "j/k link · Enter jump · y copy · t/b/w tag/branch/switch",
        Focus::Files if working(app) => "Enter diff · t tree/flat · a/u stage/unstage · c commit",
        Focus::Files => "j/k move · Enter diff/fold · t tree/flat",
    }
}

/// What the status bar says first while an operation is stopped: which, how many files still
/// conflict, and the key to resolve them (ADR-0031).
fn stopped_note(app: &App) -> String {
    match app.data.as_ref().and_then(|d| d.in_progress.as_ref()) {
        Some(stopped) => format!(
            " ⚠ {} in progress, {} conflicted · ! resolve ",
            stopped.operation.name(),
            stopped.conflicts.len()
        ),
        None => String::new(),
    }
}

/// The merge editor full screen, in one column at any width (ADR-0031): the path and what the
/// editor says, the text with each block's lines set apart, ours above theirs, and the base of
/// the block under the cursor beneath it while open.
fn draw_merge(frame: &mut Frame, editor: &mut MergeEditor, p: &Palette, area: Rect) {
    let base = editor
        .base_open
        .then(|| editor.base_here().map(str::to_string))
        .flatten();
    let base_rows = base.as_ref().map_or(0, |b| {
        let rows = u16::try_from(b.lines().count()).unwrap_or(u16::MAX);
        rows.saturating_add(2).min(area.height / 2)
    });
    let [head, text, below] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(base_rows),
    ])
    .areas(area);

    let said = if editor.leaving {
        Span::styled(
            " Leave and discard the result not saved? y leaves · any other key stays ",
            Style::new().fg(p.amber).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            editor.said.clone().unwrap_or_default(),
            Style::new().fg(p.red),
        )
    };
    let [path, note] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(said.width() as u16)])
            .areas(head);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Merge ",
                Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(editor.path.clone(), Style::new().fg(p.text_bright)),
        ]))
        .style(Style::new().bg(p.bg_chrome)),
        path,
    );
    frame.render_widget(
        Paragraph::new(Line::from(said)).style(Style::new().bg(p.bg_chrome)),
        note,
    );

    let kinds = editor.line_kinds();
    let area_text = &mut editor.area;
    area_text.clear_custom_highlight();
    for (row, kind) in kinds {
        let style = match kind {
            LineKind::Marker => Style::new().fg(p.text_muted).bg(p.bg_chrome),
            LineKind::Ours => Style::new().bg(p.add_bg),
            LineKind::Theirs => Style::new().bg(p.theirs_bg),
        };
        let end = area_text.lines()[row].chars().count();
        area_text.custom_highlight(((row, 0), (row, end)), style, 1);
    }
    area_text.set_style(Style::new().bg(p.bg).fg(p.text));
    area_text.set_cursor_line_style(Style::new());
    area_text.set_line_number_style(Style::new().fg(p.text_dim).bg(p.bg_chrome));
    frame.render_widget(&*area_text, text);

    if let Some(base) = base {
        frame.render_widget(
            Paragraph::new(base)
                .wrap(Wrap { trim: false })
                .block(
                    Block::bordered()
                        .title(" Base ")
                        .border_style(Style::new().fg(p.amber)),
                )
                .style(Style::new().bg(p.base_bg).fg(p.text)),
            below,
        );
    }
}

/// The conflicted files of the operation in progress, the cursor on `selected` (ADR-0031).
fn draw_conflicts(frame: &mut Frame, app: &App, selected: usize, p: &Palette, main: Rect) {
    let Some(stopped) = app.data.as_ref().and_then(|d| d.in_progress.as_ref()) else {
        return;
    };
    // Why the last key did nothing, here rather than in the status bar, which an 80-column
    // terminal gives no room beside the stopped operation's note.
    let said: Vec<String> = app.notice.as_deref().map_or_else(Vec::new, |said| {
        wrapped(
            said,
            usize::from(main.width.min(80).saturating_sub(2)).max(1),
        )
    });
    // The dialog opens a third down; the rows below it hold the list, a window that follows the
    // selection where the list is longer, and the reason beneath it.
    let room = usize::from((main.height - main.height / 3).saturating_sub(3))
        .saturating_sub(said.len())
        .max(1);
    let first = selected.saturating_sub(room - 1);
    let mut lines: Vec<Line> = stopped
        .conflicts
        .iter()
        .enumerate()
        .skip(first)
        .take(room)
        .map(|(i, c)| {
            let style = if i == selected {
                Style::new().fg(p.text_bright).bg(p.bg_sel)
            } else {
                Style::new().fg(p.text)
            };
            Line::styled(format!("  {}", c.path), style)
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::styled(
            "  Nothing conflicts any more: c continues the operation",
            Style::new().fg(p.text_muted),
        ));
    }
    for line in said {
        lines.push(Line::styled(line, Style::new().fg(p.red)));
    }
    lines.push(Line::styled(
        "o ours · t theirs · e as edited · m edit · c continue · A abort · Esc close",
        Style::new().fg(p.text_dim),
    ));
    let title = format!(" {} in progress ", stopped.operation.name());
    draw_dialog(frame, p, main, title, lines);
}

/// `text` broken at its whitespace, line breaks included, into lines at most `width` cells wide
/// on the terminal; a word wider than that stands on a line of its own and is cut there.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let cells = |s: &str| Span::raw(s).width();
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if cells(line) + 1 + cells(word) <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The right of the status bar: the hint, then the theme and quit keys, which the merge editor
/// takes as text.
fn status_keys(app: &App, p: &Palette) -> Line<'static> {
    if app.merge.is_some() {
        return Line::styled(format!("{} ", hints(app)), Style::new().fg(p.text_dim));
    }
    Line::from(vec![
        Span::styled(hints(app), Style::new().fg(p.text_dim)),
        Span::styled("  T ", Style::new().fg(p.accent)),
        Span::styled(
            format!("{} theme  ", app.theme.name()),
            Style::new().fg(p.text_dim),
        ),
        Span::styled("q", Style::new().fg(p.accent)),
        Span::styled(" quit ", Style::new().fg(p.text_dim)),
    ])
}

fn draw_status(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let right = status_keys(app, p);
    let width = right.width() as u16;
    // Where the terminal is narrower than the hint needs, the status text keeps its room.
    let [l, r] =
        Layout::horizontal([Constraint::Min(STATUS_ROOM), Constraint::Length(width)]).areas(area);
    let bar = Style::new().bg(p.bg_chrome);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(stopped_note(app), Style::new().fg(p.amber)),
            Span::raw(format!(" {}", app.notice.as_ref().unwrap_or(&app.status))),
        ]))
        .style(bar.fg(p.text_muted)),
        l,
    );
    frame.render_widget(Paragraph::new(right).style(bar), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{loaded_app, press};
    use crate::loader::Msg;
    use ocelli_core::model::{DiffLine, FileChange};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    /// The merge editor on a file of two blocks, the second's base found in memory.
    fn merge_editor() -> MergeEditor {
        let text = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> side\nz\n";
        let merged =
            "a\n<<<<<<< ours\nours\n||||||| base\nthe base\n=======\ntheirs\n>>>>>>> t\nz\n";
        let conflict = ocelli_core::model::Conflict {
            path: "src/f.rs".into(),
            ancestor: Some("a".into()),
            ours: Some("o".into()),
            theirs: Some("t".into()),
        };
        MergeEditor::open(
            conflict,
            ocelli_core::merge::MergeFile {
                text: text.into(),
                blob: "b".into(),
                blocks: ocelli_core::merge::blocks_with_bases(text, merged),
            },
        )
    }

    #[test]
    // Verifies: ADR-0031, ADR-0024, G-15, G-18
    fn the_merge_editor_fills_an_80_column_terminal_in_one_column_its_sides_in_the_theme() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        for theme in [crate::theme::Theme::Dark, crate::theme::Theme::Light] {
            let mut app = loaded_app();
            app.theme = theme;
            app.merge = Some(merge_editor());
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            let buf = terminal.backend().buffer().clone();
            let rows: Vec<String> = (0..24)
                .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect())
                .collect();
            assert!(rows[0].contains("Merge src/f.rs"), "{}", rows[0]);
            let row_of = |text: &str| rows.iter().position(|r| r.contains(text)).unwrap();
            let (ours, theirs) = (row_of("ours"), row_of("theirs"));
            assert!(ours < theirs, "ours above theirs");
            // Each side's line on its own background, as the theme names it; no graph beside.
            let p = palette(theme);
            let bg_at = |row: usize, text: &str| {
                let x = rows[row].find(text).unwrap();
                let x = rows[row][..x].chars().count() as u16;
                buf[(x, row as u16)].bg
            };
            assert_eq!(bg_at(ours, "ours"), p.add_bg);
            assert_eq!(bg_at(theirs, "theirs"), p.theirs_bg);
            assert!(
                !rows.iter().any(|r| r.contains("main")),
                "the panes are covered"
            );
            assert!(
                !rows.iter().any(|r| r.contains("the base")),
                "the base is folded"
            );
            app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            let buf = terminal.backend().buffer().clone();
            let rows: Vec<String> = (0..24)
                .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect())
                .collect();
            let base = rows.iter().position(|r| r.contains("the base")).unwrap();
            assert!(base > theirs, "the base opens beneath the text");
            let x = rows[base].find("the base").unwrap();
            let x = rows[base][..x].chars().count() as u16;
            assert_eq!(buf[(x, base as u16)].bg, p.base_bg);
        }
    }

    #[test]
    // Verifies: ADR-0031, G-18
    fn the_merge_editor_s_keys_fit_an_80_column_terminal_beside_the_status_text() {
        let mut app = loaded_app();
        app.merge = Some(merge_editor());
        let keys = status_keys(&app, palette(app.theme));
        assert!(
            keys.width() + usize::from(STATUS_ROOM) <= 80,
            "{} columns: {}",
            keys.width(),
            hints(&app)
        );
        let rows = screen(&mut app, 80, 24);
        assert!(rows[23].contains("Esc leave"), "{}", rows[23]);
    }

    #[test]
    // Verifies: ADR-0031, G-15, G-18
    fn why_m_opens_nothing_shows_in_the_conflicts_on_an_80_column_terminal() {
        let mut app = loaded_app();
        let conflict = ocelli_core::model::Conflict {
            path: "blob.bin".into(),
            ancestor: Some("a".into()),
            ours: Some("o".into()),
            theirs: Some("t".into()),
        };
        app.data.as_mut().unwrap().in_progress = Some(ocelli_core::model::InProgress {
            operation: ocelli_core::model::Operation::Merge,
            conflicts: vec![conflict],
            editable: Vec::new(),
            head: "d".into(),
        });
        press(&mut app, KeyCode::Char('!'));
        press(&mut app, KeyCode::Char('m'));
        let shown = screen(&mut app, 80, 24).join("\n");
        // Wrapped within the dialog at 80 columns.
        assert!(shown.contains("resolve blob.bin by"), "{shown}");
        assert!(shown.contains("o, t, or e"), "{shown}");
        press(&mut app, KeyCode::Char('j'));
        let shown = screen(&mut app, 80, 24).join("\n");
        assert!(
            !shown.contains("resolve blob.bin"),
            "until the next key: {shown}"
        );
    }

    #[test]
    fn a_reason_wraps_at_whitespace_within_the_cells_it_takes() {
        assert_eq!(wrapped("ab cd ef", 5), ["ab cd", "ef"]);
        assert_eq!(wrapped("abcdefg h", 3), ["abcdefg", "h"]);
        assert_eq!(wrapped("aaaa  bb\ncc", 4), ["aaaa", "bb", "cc"]);
        // A wide character takes two cells.
        assert_eq!(wrapped("日本 語", 4), ["日本", "語"]);
        assert!(wrapped("", 4).is_empty());
    }

    #[test]
    // Verifies: ADR-0031, G-18
    fn a_reason_beneath_a_long_list_of_conflicts_keeps_the_keys_and_the_border_on_screen() {
        let mut app = loaded_app();
        let conflicts = (0..40)
            .map(|i| ocelli_core::model::Conflict {
                path: format!("file{i:02}"),
                ancestor: Some("a".into()),
                ours: Some("o".into()),
                theirs: Some("t".into()),
            })
            .collect();
        app.data.as_mut().unwrap().in_progress = Some(ocelli_core::model::InProgress {
            operation: ocelli_core::model::Operation::Merge,
            conflicts,
            editable: Vec::new(),
            head: "d".into(),
        });
        press(&mut app, KeyCode::Char('!'));
        press(&mut app, KeyCode::Char('m'));
        let rows = screen(&mut app, 80, 24);
        let shown = rows.join("\n");
        assert!(shown.contains("resolve file00 by"), "{shown}");
        assert!(shown.contains("A abort · Esc close"), "{shown}");
        assert!(
            rows[22].starts_with('╰'),
            "the border above the status bar: {shown}"
        );
    }

    #[test]
    fn a_long_list_of_conflicts_scrolls_with_the_selection() {
        let mut app = loaded_app();
        let conflicts = (0..40)
            .map(|i| ocelli_core::model::Conflict {
                path: format!("file{i:02}"),
                ancestor: None,
                ours: None,
                theirs: None,
            })
            .collect();
        app.data.as_mut().unwrap().in_progress = Some(ocelli_core::model::InProgress {
            operation: ocelli_core::model::Operation::Merge,
            conflicts,
            editable: Vec::new(),
            head: "d".into(),
        });
        press(&mut app, KeyCode::Char('!'));
        for _ in 0..39 {
            press(&mut app, KeyCode::Char('j'));
        }
        let rows = screen(&mut app, 100, 30).join("\n");
        assert!(rows.contains("file39"), "{rows}");
        assert!(!rows.contains("file00"), "{rows}");
        assert!(rows.contains("A abort · Esc close"), "{rows}");
    }

    /// The graph pane's content lines, right of the sidebar and its border.
    fn graph_lines(rows: &[String], from: usize, count: usize) -> Vec<String> {
        rows[from..from + count]
            .iter()
            .map(|r| {
                r.chars()
                    .skip(SIDEBAR_WIDTH as usize + 1)
                    .take(REFS_WIDTH + 27)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    // Verifies: ADR-0029
    fn the_graph_pane_shows_ref_chips_lanes_shas_and_messages() {
        let mut app = loaded_app();
        let rows = screen(&mut app, 160, 14);
        let lines = graph_lines(&rows, 3, 4);
        // Chips right-aligned against the lanes; the merge carries ◉.
        assert!(
            lines[0].ends_with(" ✓ main  ◉─╮   d       commit d"),
            "{lines:#?}"
        );
        assert!(
            lines[1].ends_with("     v1  │ ●   c       commit c"),
            "{lines:#?}"
        );
        assert!(
            lines[2].ends_with("         ● │   b       commit b"),
            "{lines:#?}"
        );
        assert!(
            lines[3].ends_with("         ●─╯   a       commit a"),
            "{lines:#?}"
        );
        assert!(
            rows[2].contains("Branch / Tag"),
            "column header: {}",
            rows[2]
        );
    }

    #[test]
    fn the_title_bar_and_status_bar_carry_controls_and_status() {
        let mut app = loaded_app();
        let rows = screen(&mut app, 160, 14);
        assert!(
            rows[0].contains("Ocelli") && rows[0].contains("Refresh"),
            "{}",
            rows[0]
        );
        assert!(rows[0].contains("Ctrl+F search"), "{}", rows[0]);
        assert!(rows[13].contains("4 commits") && rows[13].contains("dark theme"));
    }

    #[test]
    // Verifies: G-9
    fn the_detail_pane_shows_the_commit_and_its_links() {
        let mut app = loaded_app();
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(text.contains("Parents   b c"), "{text}");
        assert!(text.contains("Children  —"), "{text}");
        press(&mut app, KeyCode::Esc);
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(text.contains("No commit selected"), "{text}");
    }

    #[test]
    fn the_sidebar_lists_sections_and_minimises_to_a_rail() {
        let mut app = loaded_app();
        let text = screen(&mut app, 160, 20).join("\n");
        for expected in [
            "Repository",
            "↑ super superproject",
            "▾ Local 2",
            "✓ main 2↑ checked out",
            "▸ Submodules 2",
        ] {
            assert!(text.contains(expected), "{expected:?} missing:\n{text}");
        }
        press(&mut app, KeyCode::Char('1'));
        let rows = screen(&mut app, 160, 20);
        let text = rows.join("\n");
        assert!(!text.contains("Repository"), "{text}");
        assert!(text.contains(" ⬢ 2"), "rail glyph with count:\n{text}");
        // The graph moved left into the freed width.
        assert!(
            rows[3]
                .chars()
                .skip(RAIL_WIDTH as usize)
                .collect::<String>()
                .contains("✓ main")
        );
    }

    #[test]
    fn search_shows_its_query_count_and_results_panel() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('/'));
        for c in "commit c".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(text.contains("⌕ commit c▏ 1"), "{text}");
        assert!(text.contains("Matches"), "{text}");
    }

    // Verifies: ADR-0026
    #[test]
    fn the_diff_overlay_covers_the_graph_and_leaves_the_file_list() {
        let mut app = loaded_app();
        app.on_msg(Msg::Files {
            sha: "d".into(),
            files: Ok(vec![FileChange {
                path: "f.txt".into(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 1,
                staging: None,
            }]),
        });
        // The pane's own title says both that the list is drawn and which view it starts in.
        let pane = "Changed files (1) · tree (t)";
        let before = screen(&mut app, 160, 20).join("\n");
        assert!(
            before.contains("commit a"),
            "a graph row is drawn:\n{before}"
        );
        assert!(
            before.contains(pane),
            "the file list is drawn, in tree view:\n{before}"
        );

        app.focus = Focus::Files;
        press(&mut app, KeyCode::Enter);
        app.on_msg(Msg::Diff {
            sha: "d".into(),
            path: "f.txt".into(),
            lines: Ok(vec![DiffLine {
                kind: DiffLineKind::Add,
                content: "a line only the diff has\n".into(),
                old_lineno: None,
                new_lineno: Some(1),
            }]),
        });
        let after = screen(&mut app, 160, 20).join("\n");
        assert!(
            after.contains("a line only the diff has"),
            "the diff is drawn:\n{after}"
        );
        assert!(
            !after.contains("commit a"),
            "the diff covers the graph pane, the way the window's covers the canvas:\n{after}"
        );
        assert!(
            after.contains(pane),
            "and the file list stays beside it, so the next file can be picked:\n{after}"
        );
    }

    #[test]
    fn the_diff_overlay_switches_to_side_by_side() {
        let mut app = loaded_app();
        app.on_msg(Msg::Files {
            sha: "d".into(),
            files: Ok(vec![FileChange {
                path: "f.txt".into(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 1,
                staging: None,
            }]),
        });
        app.focus = Focus::Files;
        press(&mut app, KeyCode::Enter);
        app.on_msg(Msg::Diff {
            sha: "d".into(),
            path: "f.txt".into(),
            lines: Ok(vec![
                DiffLine {
                    kind: DiffLineKind::Delete,
                    content: "old\n".into(),
                    old_lineno: Some(1),
                    new_lineno: None,
                },
                DiffLine {
                    kind: DiffLineKind::Add,
                    content: "new\n".into(),
                    old_lineno: None,
                    new_lineno: Some(1),
                },
            ]),
        });
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(text.contains("f.txt · unified (s)"), "{text}");
        press(&mut app, KeyCode::Char('s'));
        let rows = screen(&mut app, 160, 20);
        let paired = rows.iter().find(|r| r.contains("old") && r.contains("new"));
        assert!(
            paired.is_some(),
            "old and new side by side:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    // Verifies: ADR-0024
    fn the_light_theme_paints_a_light_background() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('T'));
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let bg = terminal.backend().buffer()[(40, 5)].bg;
        assert_eq!(bg, palette(crate::theme::Theme::Light).bg);
    }

    #[test]
    fn the_open_prompt_is_drawn_over_the_panes() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('o'));
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(
            text.contains("Open repository") && text.contains("/repo▏"),
            "{text}"
        );
    }

    #[test]
    fn the_tag_prompt_shows_the_commit_the_name_and_then_the_message() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('v'));
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(
            text.contains("Tag d") && text.contains("Name     v▏"),
            "{text}"
        );
        press(&mut app, KeyCode::Enter);
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(
            text.contains("Message  ▏") && text.contains("lightweight"),
            "{text}"
        );
    }

    #[test]
    // Verifies: ADR-0033
    fn the_question_before_deleting_names_what_is_lost() {
        let mut app = loaded_app();
        app.modal = Some(Modal::Confirm(Write::DeleteTag {
            name: "v1".into(),
            commit: "c".into(),
            annotated: true,
        }));
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(
            text.contains("This discards the tag v1 and its message")
                && text.contains("y go ahead · any other key keeps it"),
            "{text}"
        );
    }

    #[test]
    fn the_stash_prompt_shows_the_message_and_the_untracked_switch() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Tab);
        let text = screen(&mut app, 160, 20).join("\n");
        assert!(
            text.contains("Stash changes")
                && text.contains("Message    w▏")
                && text.contains("[x] include untracked files"),
            "{text}"
        );
    }

    #[test]
    fn the_working_nodes_files_show_what_is_staged() {
        use ocelli_core::model::{FileStatus, Staged};
        let mut app = loaded_app();
        let file = |path: &str, staged| FileChange {
            path: path.into(),
            old_path: None,
            status: FileStatus::Modified,
            additions: 0,
            deletions: 0,
            staging: Some(ocelli_core::model::Staging {
                staged,
                shown: ocelli_core::model::Shown {
                    workdir: None,
                    index: None,
                },
            }),
        };
        app.files = Some(crate::app::Files {
            sha: ocelli_core::model::WORKDIR_SHA.into(),
            files: Ok(vec![
                file("all", Staged::Yes),
                file("part", Staged::Partly),
                file("none", Staged::No),
            ]),
            selected: 0,
        });
        app.file_view = crate::files::FileView::Flat;
        let text = screen(&mut app, 160, 20).join("\n");
        for expected in ["● M all", "◐ M part", "○ M none"] {
            assert!(text.contains(expected), "{expected:?} missing:\n{text}");
        }
    }

    #[test]
    // Verifies: ADR-0029
    fn a_tiny_terminal_does_not_panic() {
        let mut app = loaded_app();
        for (w, h) in [(1, 1), (10, 3), (30, 2), (60, 6)] {
            screen(&mut app, w, h);
        }
        press(&mut app, KeyCode::Char('o'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('t'));
        for (w, h) in [(1, 1), (10, 3), (30, 2)] {
            screen(&mut app, w, h);
        }
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('c'));
        for (w, h) in [(1, 1), (10, 3), (30, 2)] {
            screen(&mut app, w, h);
        }
    }

    #[test]
    // Verifies: ADR-0040
    fn older_history_with_more_lanes_moves_nothing_until_scrolled_to() {
        use crate::app::tests::commit;
        use crate::loader::Msg;
        use ocelli_core::graph::Layout;
        use ocelli_core::load::Event;
        let mut layout = Layout::new();
        let mut app = App::new("/repo".into(), crate::raster::Glyphs::Unicode);
        // A main line longer than the screen is tall.
        let first: Vec<_> = (1..=20)
            .rev()
            .map(|i| commit(&format!("t{i}"), 20 + i, &[&format!("t{}", i - 1)], &[]))
            .collect();
        app.on_msg(Msg::Load(Event::Chunk(layout.lay_out(first))));
        let before = screen(&mut app, 120, 12);
        // Further down, five lines side by side, wider than the column's least width.
        let mut older = vec![commit("t0", 20, &["a0"], &[])];
        for (i, line) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            let tip = format!("{line}1");
            let root = format!("{line}0");
            if i > 0 {
                older.push(commit(&tip, 19 - i as i64, &[&root], &[]));
            }
            older.push(commit(&root, 10 - i as i64, &[], &[]));
        }
        app.on_msg(Msg::Load(Event::Chunk(layout.lay_out(older))));
        let after = screen(&mut app, 120, 12);
        assert_eq!(
            graph_lines(&after, 3, 3),
            graph_lines(&before, 3, 3),
            "the rows shown stay where they were"
        );
    }

    /// Every state `hints` answers for, named: each modal, each focus, each kind of node
    /// selected in the graph, and the search, the filter, and the diff that take the keys.
    fn every_hinted_state() -> Vec<(&'static str, App)> {
        use crate::app::tests::{on_a_stash, on_the_working_node};
        use crate::app::{DiffView, StashPrompt, TagPrompt};
        use ocelli_core::port::{ResetMode, Write};

        let with = |f: &dyn Fn(&mut App)| {
            let mut app = loaded_app();
            f(&mut app);
            app
        };
        let modal = |m: Modal| with(&|app: &mut App| app.modal = Some(m.clone()));
        let focus = |f: Focus| with(&|app: &mut App| app.focus = f);
        let tag = |message: Option<String>| TagPrompt {
            sha: "d".into(),
            name: "v2".into(),
            message,
        };
        let mut working_files = on_the_working_node();
        working_files.focus = Focus::Files;
        vec![
            ("the open prompt", modal(Modal::Open("/repo".into()))),
            (
                "the question before a write that discards work",
                modal(Modal::Confirm(Write::DropStash {
                    index: 0,
                    commit: "d".into(),
                })),
            ),
            ("the tag's name", modal(Modal::Tag(tag(None)))),
            (
                "the tag's message",
                modal(Modal::Tag(tag(Some(String::new())))),
            ),
            (
                "the stash prompt",
                modal(Modal::Stash(StashPrompt {
                    message: String::new(),
                    untracked: false,
                })),
            ),
            (
                "the commit prompt",
                modal(Modal::Commit {
                    head: "d".into(),
                    message: String::new(),
                }),
            ),
            (
                "the branch prompt",
                modal(Modal::Branch {
                    commit: "d".into(),
                    name: String::new(),
                    switch: true,
                }),
            ),
            (
                "the reset's mode",
                modal(Modal::Reset {
                    commit: "d".into(),
                    mode: ResetMode::Mixed,
                }),
            ),
            ("the conflicts", modal(Modal::Conflicts { selected: 0 })),
            (
                "the merge editor",
                with(&|app: &mut App| app.merge = Some(merge_editor())),
            ),
            (
                "the search field",
                with(&|app: &mut App| app.search.editing = true),
            ),
            (
                "the sidebar's filter",
                with(&|app: &mut App| {
                    app.focus = Focus::Sidebar;
                    app.side.editing = true;
                }),
            ),
            (
                "the diff",
                with(&|app: &mut App| {
                    app.diff = Some(DiffView {
                        path: "f".into(),
                        lines: None,
                        scroll: 0,
                    });
                }),
            ),
            ("the sidebar", focus(Focus::Sidebar)),
            (
                "the sidebar minimised",
                with(&|app: &mut App| {
                    app.focus = Focus::Sidebar;
                    app.side.minimised = true;
                }),
            ),
            ("the graph on a commit", focus(Focus::Graph)),
            (
                "the graph on the working-directory node",
                on_the_working_node(),
            ),
            ("the graph on a stash node", on_a_stash()),
            (
                "the graph before a load",
                App::new("/repo".into(), Glyphs::Unicode),
            ),
            ("the detail", focus(Focus::Detail)),
            ("a commit's files", focus(Focus::Files)),
            ("the working-directory node's files", working_files),
        ]
    }

    #[test]
    fn every_hint_fits_the_width_the_layout_is_designed_for() {
        for (state, mut app) in every_hinted_state() {
            for theme in [crate::theme::Theme::Dark, crate::theme::Theme::Light] {
                app.theme = theme;
                let keys = status_keys(&app, palette(theme));
                assert!(
                    keys.width() + usize::from(STATUS_ROOM) <= usize::from(DESIGN_WIDTH),
                    "the hint of {state} takes {} columns with the theme and quit keys, past the \
                     {} beside the status text in {DESIGN_WIDTH}: {}",
                    keys.width(),
                    DESIGN_WIDTH - STATUS_ROOM,
                    hints(&app)
                );
            }
        }
    }

    #[test]
    fn the_status_text_keeps_its_room_on_a_narrow_terminal() {
        let mut app = loaded_app();
        let rows = screen(&mut app, 40, 10);
        assert!(rows[9].contains("4 commits"), "{}", rows[9]);
    }
}
