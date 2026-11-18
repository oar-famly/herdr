use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) archived: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentSection {
    Active,
    Archived,
}

impl AgentSection {
    fn of(archived: bool) -> Self {
        if archived {
            Self::Archived
        } else {
            Self::Active
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AgentSectionsCollapsed {
    pub(super) active: bool,
    pub(super) archived: bool,
}

impl Default for AgentSectionsCollapsed {
    fn default() -> Self {
        Self {
            active: false,
            archived: true,
        }
    }
}

impl AgentSectionsCollapsed {
    pub(super) fn is_collapsed(self, section: AgentSection) -> bool {
        match section {
            AgentSection::Active => self.active,
            AgentSection::Archived => self.archived,
        }
    }

    pub(super) fn toggle(&mut self, section: AgentSection) {
        match section {
            AgentSection::Active => self.active = !self.active,
            AgentSection::Archived => self.archived = !self.archived,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AgentSectionHeader {
    pub(super) section: AgentSection,
    pub(super) count: usize,
    pub(super) collapsed: bool,
    pub(super) status: Option<crate::api::schema::AgentStatus>,
}

pub(super) enum AgentPanelEntry<'a, T> {
    Header(AgentSectionHeader),
    Row(&'a T),
}

/// Sections appear only once something is archived. A collapsed header carries the
/// most urgent status of the agents it hides.
pub(super) fn agent_panel_entries<'a, T>(
    rows: &'a [T],
    archived: impl Fn(&T) -> bool,
    status: impl Fn(&T) -> crate::api::schema::AgentStatus,
    collapsed: AgentSectionsCollapsed,
) -> Vec<AgentPanelEntry<'a, T>> {
    if !rows.iter().any(&archived) {
        return rows.iter().map(AgentPanelEntry::Row).collect();
    }
    let mut entries = Vec::with_capacity(rows.len() + 2);
    for section in [AgentSection::Active, AgentSection::Archived] {
        let members = rows
            .iter()
            .filter(|row| AgentSection::of(archived(row)) == section)
            .collect::<Vec<_>>();
        let is_collapsed = collapsed.is_collapsed(section);
        entries.push(AgentPanelEntry::Header(AgentSectionHeader {
            section,
            count: members.len(),
            collapsed: is_collapsed,
            status: is_collapsed
                .then(|| {
                    members
                        .iter()
                        .map(|row| status(row))
                        .max_by_key(|status| status_priority(*status))
                })
                .flatten(),
        }));
        if !is_collapsed {
            entries.extend(members.into_iter().map(AgentPanelEntry::Row));
        }
    }
    entries
}

pub(super) fn visible_in_sections<T>(
    items: Vec<T>,
    archived: impl Fn(&T) -> bool,
    collapsed: AgentSectionsCollapsed,
) -> Vec<T> {
    if !items.iter().any(&archived) {
        return items;
    }
    items
        .into_iter()
        .filter(|item| !collapsed.is_collapsed(AgentSection::of(archived(item))))
        .collect()
}

fn agent_is_archived(snapshot: &ClientShellSnapshot, pane_id: &str) -> bool {
    snapshot
        .agents
        .iter()
        .any(|agent| agent.pane_id == pane_id && agent.archived)
}

pub(super) fn visible_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
    collapsed: AgentSectionsCollapsed,
) -> Vec<String> {
    visible_in_sections(
        ordered_agent_pane_ids(snapshot, sort),
        |pane_id| agent_is_archived(snapshot, pane_id),
        collapsed,
    )
}

pub(super) fn agent_panel_entry_index(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
    collapsed: AgentSectionsCollapsed,
    pane_id: &str,
) -> Option<usize> {
    let ids = ordered_agent_pane_ids(snapshot, sort);
    agent_panel_entries(
        &ids,
        |pane_id| agent_is_archived(snapshot, pane_id),
        |_| crate::api::schema::AgentStatus::Unknown,
        collapsed,
    )
    .iter()
    .position(|entry| matches!(entry, AgentPanelEntry::Row(id) if id.as_str() == pane_id))
}

pub(super) fn agent_entry_gaps<T>(entries: &[AgentPanelEntry<'_, T>], row_gap: u16) -> Vec<u16> {
    entries
        .iter()
        .zip(entries.iter().skip(1).map(Some).chain([None]))
        .map(|(entry, next)| match (entry, next) {
            (_, None) => 0,
            (_, Some(AgentPanelEntry::Header(_))) => row_gap.max(1),
            (_, Some(AgentPanelEntry::Row(_))) => row_gap,
        })
        .collect()
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    let mut agents = if snapshot.agent_view_label.is_some() {
        snapshot
            .agent_order
            .iter()
            .filter_map(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id.as_str())
            })
            .collect::<Vec<_>>()
    } else {
        let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
        if sort == crate::config::AgentPanelSortConfig::Priority {
            agents.sort_by_key(|agent| {
                (
                    std::cmp::Reverse(status_priority(agent.agent_status)),
                    std::cmp::Reverse(agent.state_change_seq),
                )
            });
        }
        agents
    };
    agents.sort_by_key(|agent| agent.archived);
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    sections: AgentSectionsCollapsed,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    let entries = agent_panel_entries(&rows, |row| row.archived, |row| row.status, sections);
    render_agent_list(
        buffer,
        area,
        &entries,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.rows.len(),
        |buffer, rect, row, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            render_agent_row(buffer, rect, row, config);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        " agents",
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[AgentPanelEntry<'_, T>],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|entry| match entry {
            AgentPanelEntry::Header(_) => 1,
            AgentPanelEntry::Row(row) => row_lines(row).max(1).min(u16::MAX as usize) as u16,
        })
        .collect::<Vec<_>>();
    let gaps = agent_entry_gaps(rows, config.agents.row_gap);
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, entry) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        match entry {
            AgentPanelEntry::Header(header) => {
                render_section_header(buffer, rect, *header, config, hits)
            }
            AgentPanelEntry::Row(row) => render_row(buffer, rect, row, hits),
        }
        y = y.saturating_add(height).saturating_add(gaps[index]);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn render_section_header(
    buffer: &mut Buffer,
    rect: Rect,
    header: AgentSectionHeader,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let mut spans = vec![
        ratatui::text::Span::styled(
            format!(
                " {} {}",
                if header.collapsed { "▸" } else { "▾" },
                header.section.label()
            ),
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled(
            format!(" {}", header.count),
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM),
        ),
    ];
    if let Some(status) = header.status {
        spans.push(ratatui::text::Span::styled(
            format!(" {}", status_icon(status, config.status_indicators)),
            Style::default().fg(status_color(status, palette)),
        ));
    }
    let used = spans
        .iter()
        .map(|span| display_width(&span.content))
        .sum::<usize>();
    let rule = (rect.width as usize).saturating_sub(used + 1);
    if rule > 0 {
        spans.push(ratatui::text::Span::styled(
            format!(" {}", "─".repeat(rule)),
            Style::default().fg(palette.surface_dim),
        ));
    }
    let line = Rect::new(rect.x, rect.y, rect.width, 1);
    Paragraph::new(Line::from(spans)).render(line, buffer);
    if config.mouse_capture {
        hits.agent_sections.push((line, header.section));
    }
}

pub(super) fn render_compact_section_header(
    buffer: &mut Buffer,
    rect: Rect,
    header: AgentSectionHeader,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width.min(2),
        if header.collapsed { "▸" } else { "▾" },
        Style::default().fg(config.palette.overlay0),
    );
    if let Some(status) = header.status {
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(status, config.status_indicators),
            Style::default().fg(status_color(status, &config.palette)),
        );
    }
    if config.mouse_capture {
        hits.agent_sections.push((rect, header.section));
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine))
        .collect()
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
        .count();
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| tab.label.as_str());
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let labels = agent
        .state_labels
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let state_text = labels
        .get(status_text(agent.agent_status))
        .map(String::as_str)
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let canonical_agent = agent
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let rows = crate::ui::sidebar_agent_rows(
        &config.agents,
        crate::ui::AgentTokenContext {
            machine,
            workspace: &workspace.label,
            tab: tab_label,
            pane: agent
                .title
                .as_deref()
                .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
            agent_label,
            terminal_title: agent.terminal_title.as_deref(),
            terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
            canonical_agent,
            tokens: &tokens,
        },
        state_text,
    );
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        archived: agent.archived,
        rows,
    })
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color(row.status, palette));
    let secondary = Style::default().fg(palette.overlay0);
    let icon = (
        status_icon(row.status, config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;

    type Row = (&'static str, bool, AgentStatus);

    fn rows() -> Vec<Row> {
        vec![
            ("a", false, AgentStatus::Idle),
            ("b", true, AgentStatus::Working),
            ("c", false, AgentStatus::Working),
            ("d", true, AgentStatus::Blocked),
        ]
    }

    fn expanded() -> AgentSectionsCollapsed {
        AgentSectionsCollapsed {
            active: false,
            archived: false,
        }
    }

    fn render(rows: &[Row], collapsed: AgentSectionsCollapsed) -> Vec<String> {
        agent_panel_entries(rows, |row| row.1, |row| row.2, collapsed)
            .into_iter()
            .map(|entry| match entry {
                AgentPanelEntry::Header(header) => format!(
                    "[{} {}{} {:?}]",
                    header.section.label(),
                    header.count,
                    if header.collapsed { " collapsed" } else { "" },
                    header.status
                ),
                AgentPanelEntry::Row(row) => row.0.to_owned(),
            })
            .collect()
    }

    fn names(rows: Vec<Row>) -> Vec<&'static str> {
        rows.into_iter().map(|row| row.0).collect()
    }

    #[test]
    fn panel_has_no_sections_until_something_is_archived() {
        let active = vec![("a", false, AgentStatus::Idle)];
        assert_eq!(render(&active, AgentSectionsCollapsed::default()), ["a"]);
        let all_collapsed = AgentSectionsCollapsed {
            active: true,
            archived: true,
        };
        assert_eq!(
            names(visible_in_sections(active, |row| row.1, all_collapsed)),
            ["a"]
        );
    }

    #[test]
    fn archived_section_starts_collapsed_with_its_most_urgent_status() {
        assert_eq!(
            render(&rows(), AgentSectionsCollapsed::default()),
            [
                "[active 2 None]",
                "a",
                "c",
                "[archived 2 collapsed Some(Blocked)]"
            ]
        );
        assert_eq!(
            names(visible_in_sections(
                rows(),
                |row| row.1,
                AgentSectionsCollapsed::default()
            )),
            ["a", "c"]
        );
    }

    #[test]
    fn expanded_sections_keep_the_existing_order_within_each_section() {
        assert_eq!(
            render(&rows(), expanded()),
            ["[active 2 None]", "a", "c", "[archived 2 None]", "b", "d"]
        );
    }

    #[test]
    fn a_collapsed_active_section_hides_its_agents_from_navigation() {
        let only_archived = AgentSectionsCollapsed {
            active: true,
            archived: false,
        };
        assert_eq!(
            render(&rows(), only_archived),
            [
                "[active 2 collapsed Some(Working)]",
                "[archived 2 None]",
                "b",
                "d"
            ]
        );
        assert_eq!(
            names(visible_in_sections(rows(), |row| row.1, only_archived)),
            ["b", "d"]
        );
    }

    #[test]
    fn a_blank_line_separates_the_sections_but_not_a_title_from_its_agents() {
        let rows = rows();
        let entries = agent_panel_entries(&rows, |row| row.1, |row| row.2, expanded());
        assert_eq!(agent_entry_gaps(&entries, 0), [0, 0, 1, 0, 0, 0]);
        assert_eq!(agent_entry_gaps(&entries, 2), [2, 2, 2, 2, 2, 0]);

        let only_archived = AgentSectionsCollapsed {
            active: true,
            archived: false,
        };
        let entries = agent_panel_entries(&rows, |row| row.1, |row| row.2, only_archived);
        assert_eq!(agent_entry_gaps(&entries, 0), [1, 0, 0, 0]);
    }

    #[test]
    fn sectionless_panels_keep_the_configured_row_gap() {
        let rows = vec![
            ("a", false, AgentStatus::Idle),
            ("b", false, AgentStatus::Idle),
        ];
        let entries = agent_panel_entries(
            &rows,
            |row| row.1,
            |row| row.2,
            AgentSectionsCollapsed::default(),
        );
        assert_eq!(agent_entry_gaps(&entries, 0), [0, 0]);
    }
}
