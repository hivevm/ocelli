//! Drawing. The layout follows the desktop window: a title bar with the open,
//! refresh and search controls; the repository sidebar, the graph and the
//! commit detail with its changed files side by side; a status bar below. Only
//! the visible window of the graph is rasterised each frame (ADR-0017);
//! everything else is read from the [`App`] state.

use crate::app::{App, Focus};
use crate::chips::{Planned, RefKind, chip_width, chips, plan};
use crate::diffsplit::{CellKind, SplitRow, split_rows};
use crate::files::{FileRow, FileView};
use crate::raster::{self, rasterize};
use crate::sidebar::{self, Look, RailRow, RowKind};
use crate::theme::{Palette, lane, palette};
use ocelli_core::model::{CommitKind, DiffLine, DiffLineKind, FileStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// Width of the branch/tag column left of the graph lanes.
const REFS_WIDTH: usize = 22;
const SIDEBAR_WIDTH: u16 = 32;
const RAIL_WIDTH: u16 = 7;
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
    if app.prompt.is_some() {
        draw_prompt(frame, app, p, main);
    }
    if app.tag_prompt.is_some() {
        draw_tag_prompt(frame, app, p, main);
    }
    if app.stash_prompt.is_some() {
        draw_stash_prompt(frame, app, p, main);
    }
    if app.confirm.is_some() {
        draw_confirm(frame, app, p, main);
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
                let mut spans = vec![
                    Span::raw("  ".repeat(depth)),
                    status_letter(p, &f.status),
                    Span::styled(name, name_style),
                ];
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

fn draw_prompt(frame: &mut Frame, app: &App, p: &Palette, main: Rect) {
    let Some(text) = &app.prompt else { return };
    let lines = vec![
        input_line(p, "", text, true),
        Line::styled("Enter open · Esc cancel", Style::new().fg(p.text_dim)),
    ];
    draw_dialog(frame, p, main, " Open repository ".to_string(), lines);
}

/// The tag prompt (G-7): the name, then the message, the line being typed
/// carrying the cursor.
fn draw_tag_prompt(frame: &mut Frame, app: &App, p: &Palette, main: Rect) {
    let Some(tag) = &app.tag_prompt else { return };
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
/// (ADR-0034).
fn draw_confirm(frame: &mut Frame, app: &App, p: &Palette, main: Rect) {
    let Some(lost) = app.confirm.as_ref().and_then(|w| w.discards()) else {
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

/// The stash prompt (G-7): the message, and whether the untracked files go
/// with the changes.
fn draw_stash_prompt(frame: &mut Frame, app: &App, p: &Palette, main: Rect) {
    let Some(stash) = &app.stash_prompt else {
        return;
    };
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

fn hints(app: &App) -> &'static str {
    if app.prompt.is_some() {
        return "type a path";
    }
    if app.confirm.is_some() {
        return "y goes ahead · any other key keeps it";
    }
    if app.stash_prompt.is_some() {
        return "type a message, or none";
    }
    if let Some(tag) = &app.tag_prompt {
        return if tag.message.is_none() {
            "type the tag's name"
        } else {
            "type a message, or none"
        };
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
        Focus::Sidebar => "j/k move · Enter open/toggle · / filter · d delete tag · 1 minimise",
        Focus::Graph => {
            "j/k move · ]/[ parent/child · }/{ refs · / search · y copy · t tag · s stash · \
             a/p/d apply/pop/drop · Esc deselect"
        }
        Focus::Detail => "j/k link · Enter jump · y copy · t tag · a/p/d apply/pop/drop",
        Focus::Files => "j/k move · Enter diff/fold · t tree/flat",
    }
}

fn draw_status(frame: &mut Frame, app: &App, p: &Palette, area: Rect) {
    let right = Line::from(vec![
        Span::styled(hints(app), Style::new().fg(p.text_dim)),
        Span::styled("  T ", Style::new().fg(p.accent)),
        Span::styled(
            format!("{} theme  ", app.theme.name()),
            Style::new().fg(p.text_dim),
        ),
        Span::styled("q", Style::new().fg(p.accent)),
        Span::styled(" quit ", Style::new().fg(p.text_dim)),
    ]);
    let width = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    let bar = Style::new().bg(p.bg_chrome);
    frame.render_widget(
        Paragraph::new(format!(" {}", app.status)).style(bar.fg(p.text_muted)),
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
    // Verifies: ADR-0028
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
    // Verifies: G-12
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

    // Verifies: ADR-0020
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
    // Verifies: ADR-0021
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
    // Verifies: ADR-0034
    fn the_question_before_deleting_names_what_is_lost() {
        let mut app = loaded_app();
        app.confirm = Some(ocelli_core::port::Write::DeleteTag {
            name: "v1".into(),
            commit: "c".into(),
            annotated: true,
        });
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
    // Verifies: ADR-0028
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
    // Verifies: ADR-0031
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
}
