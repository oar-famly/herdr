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
    pub(super) agent_group: Option<String>,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

/// An agent panel section: a user-defined group, or `None` for the default section
/// holding every agent that is not filed into a group.
pub(super) type AgentSection = Option<String>;

pub(super) const DEFAULT_SECTION_LABEL: &str = "active";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AgentSectionsCollapsed {
    collapsed: std::collections::HashSet<AgentSection>,
}

impl AgentSectionsCollapsed {
    pub(super) fn from_keys(keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            collapsed: keys
                .into_iter()
                .map(|key| (!key.is_empty()).then_some(key))
                .collect(),
        }
    }

    pub(super) fn keys(&self) -> Vec<String> {
        let mut keys = self
            .collapsed
            .iter()
            .map(|section| section.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub(super) fn is_collapsed(&self, section: &AgentSection) -> bool {
        self.collapsed.contains(section)
    }

    pub(super) fn toggle(&mut self, section: &AgentSection) {
        if !self.collapsed.remove(section) {
            self.collapsed.insert(section.clone());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentSectionHeader {
    pub(super) section: AgentSection,
    pub(super) label: String,
    pub(super) count: usize,
    pub(super) collapsed: bool,
    pub(super) status: Option<crate::api::schema::AgentStatus>,
}

pub(super) enum AgentPanelEntry<'a, T> {
    Header(AgentSectionHeader),
    Row(&'a T),
}

fn section_of(
    groups: &[crate::protocol::ClientShellAgentGroup],
    group: Option<&str>,
) -> AgentSection {
    group
        .filter(|group| groups.iter().any(|candidate| candidate.group_id == *group))
        .map(str::to_owned)
}

pub(super) fn section_rank(
    groups: &[crate::protocol::ClientShellAgentGroup],
    group: Option<&str>,
) -> usize {
    group
        .and_then(|group| {
            groups
                .iter()
                .position(|candidate| candidate.group_id == group)
        })
        .map_or(0, |index| index + 1)
}

/// Sections appear only once a group exists. Every group gets a header, even when
/// empty. A collapsed header carries the most urgent status of the agents it hides.
pub(super) fn agent_panel_entries<'a, T>(
    rows: &'a [T],
    group_of: impl Fn(&T) -> Option<&str>,
    status: impl Fn(&T) -> crate::api::schema::AgentStatus,
    groups: &[crate::protocol::ClientShellAgentGroup],
    collapsed: &AgentSectionsCollapsed,
) -> Vec<AgentPanelEntry<'a, T>> {
    if groups.is_empty() {
        return rows.iter().map(AgentPanelEntry::Row).collect();
    }
    let sections = std::iter::once((None, DEFAULT_SECTION_LABEL.to_owned())).chain(
        groups
            .iter()
            .map(|group| (Some(group.group_id.clone()), group.label.clone())),
    );
    let mut entries = Vec::with_capacity(rows.len() + groups.len() + 1);
    for (section, label) in sections {
        let members = rows
            .iter()
            .filter(|row| section_of(groups, group_of(row)) == section)
            .collect::<Vec<_>>();
        let is_collapsed = collapsed.is_collapsed(&section);
        entries.push(AgentPanelEntry::Header(AgentSectionHeader {
            status: is_collapsed
                .then(|| {
                    members
                        .iter()
                        .map(|row| status(row))
                        .max_by_key(|status| status_priority(*status))
                })
                .flatten(),
            section,
            label,
            count: members.len(),
            collapsed: is_collapsed,
        }));
        if !is_collapsed {
            entries.extend(members.into_iter().map(AgentPanelEntry::Row));
        }
    }
    entries
}

pub(super) fn visible_in_sections<T>(
    items: Vec<T>,
    group_of: impl Fn(&T) -> Option<String>,
    groups: &[crate::protocol::ClientShellAgentGroup],
    collapsed: &AgentSectionsCollapsed,
) -> Vec<T> {
    if groups.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|item| !collapsed.is_collapsed(&section_of(groups, group_of(item).as_deref())))
        .collect()
}

pub(super) fn active_agent_groups<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
) -> &'a [crate::protocol::ClientShellAgentGroup] {
    endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == active_endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .map_or(&[], |snapshot| snapshot.agent_groups.as_slice())
}

fn agent_group_of(snapshot: &ClientShellSnapshot, pane_id: &str) -> Option<String> {
    snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)
        .and_then(|agent| agent.agent_group.clone())
}

pub(super) fn visible_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
    collapsed: &AgentSectionsCollapsed,
) -> Vec<String> {
    visible_in_sections(
        ordered_agent_pane_ids(snapshot, sort),
        |pane_id| agent_group_of(snapshot, pane_id),
        &snapshot.agent_groups,
        collapsed,
    )
}

pub(super) fn agent_panel_entry_index(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
    collapsed: &AgentSectionsCollapsed,
    pane_id: &str,
) -> Option<usize> {
    let ids = ordered_agent_pane_ids(snapshot, sort)
        .into_iter()
        .map(|id| {
            let group = agent_group_of(snapshot, &id);
            (id, group)
        })
        .collect::<Vec<_>>();
    agent_panel_entries(
        &ids,
        |(_, group)| group.as_deref(),
        |_| crate::api::schema::AgentStatus::Unknown,
        &snapshot.agent_groups,
        collapsed,
    )
    .iter()
    .position(|entry| matches!(entry, AgentPanelEntry::Row((id, _)) if id.as_str() == pane_id))
}

pub(super) fn agent_entry_gaps<T>(entries: &[AgentPanelEntry<'_, T>], row_gap: u16) -> Vec<u16> {
    entries
        .iter()
        .zip(entries.iter().skip(1).map(Some).chain([None]))
        .map(|(_, next)| match next {
            None => 0,
            Some(AgentPanelEntry::Header(_)) => row_gap.max(1),
            Some(AgentPanelEntry::Row(_)) => row_gap,
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
    agents.sort_by_key(|agent| section_rank(&snapshot.agent_groups, agent.agent_group.as_deref()));
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
    sections: &AgentSectionsCollapsed,
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
    let entries = agent_panel_entries(
        &rows,
        |row| row.agent_group.as_deref(),
        |row| row.status,
        &snapshot.agent_groups,
        sections,
    );
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
                render_section_header(buffer, rect, header, config, hits)
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
    header: &AgentSectionHeader,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let mut spans = vec![ratatui::text::Span::styled(
        format!(" {}", if header.collapsed { "▸" } else { "▾" }),
        Style::default().fg(palette.accent),
    )];
    if header.collapsed {
        let status = header
            .status
            .unwrap_or(crate::api::schema::AgentStatus::Unknown);
        spans.push(ratatui::text::Span::styled(
            format!(" {}", status_icon(status, config.status_indicators)),
            Style::default().fg(status_color(status, palette)),
        ));
    }
    spans.extend([
        ratatui::text::Span::styled(
            format!(" {}", header.label),
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
    ]);
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
        hits.agent_sections.push((line, header.section.clone()));
    }
}

pub(super) fn render_compact_section_header(
    buffer: &mut Buffer,
    rect: Rect,
    header: &AgentSectionHeader,
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
        hits.agent_sections.push((rect, header.section.clone()));
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
        agent_group: agent.agent_group.clone(),
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
    use crate::protocol::ClientShellAgentGroup;

    type Row = (&'static str, Option<&'static str>, AgentStatus);

    fn groups() -> Vec<ClientShellAgentGroup> {
        ["reviews", "parked"]
            .iter()
            .enumerate()
            .map(|(index, label)| ClientShellAgentGroup {
                group_id: format!("g{}", index + 1),
                label: (*label).into(),
            })
            .collect()
    }

    fn rows() -> Vec<Row> {
        vec![
            ("a", None, AgentStatus::Idle),
            ("b", Some("g2"), AgentStatus::Working),
            ("c", Some("gone"), AgentStatus::Working),
            ("d", Some("g2"), AgentStatus::Blocked),
            ("e", Some("g1"), AgentStatus::Idle),
        ]
    }

    fn collapsed(keys: &[&str]) -> AgentSectionsCollapsed {
        AgentSectionsCollapsed::from_keys(keys.iter().map(|key| (*key).to_owned()))
    }

    fn render(
        rows: &[Row],
        groups: &[ClientShellAgentGroup],
        collapsed: &AgentSectionsCollapsed,
    ) -> Vec<String> {
        agent_panel_entries(rows, |row| row.1, |row| row.2, groups, collapsed)
            .into_iter()
            .map(|entry| match entry {
                AgentPanelEntry::Header(header) => format!(
                    "[{} {}{} {:?}]",
                    header.label,
                    header.count,
                    if header.collapsed { " collapsed" } else { "" },
                    header.status
                ),
                AgentPanelEntry::Row(row) => row.0.to_owned(),
            })
            .collect()
    }

    #[test]
    fn panel_has_no_sections_until_a_group_exists() {
        assert_eq!(
            render(&rows(), &[], &collapsed(&[""])),
            ["a", "b", "c", "d", "e"]
        );
    }

    #[test]
    fn sections_follow_group_order_and_unknown_groups_fall_back_to_active() {
        assert_eq!(
            render(&rows(), &groups(), &AgentSectionsCollapsed::default()),
            [
                "[active 2 None]",
                "a",
                "c",
                "[reviews 1 None]",
                "e",
                "[parked 2 None]",
                "b",
                "d"
            ]
        );
    }

    #[test]
    fn empty_groups_still_get_a_header() {
        let rows = vec![("a", None, AgentStatus::Idle)];
        assert_eq!(
            render(&rows, &groups(), &AgentSectionsCollapsed::default()),
            [
                "[active 1 None]",
                "a",
                "[reviews 0 None]",
                "[parked 0 None]"
            ]
        );
    }

    #[test]
    fn collapsed_sections_show_their_most_urgent_status_and_hide_from_navigation() {
        let state = collapsed(&["g2", ""]);
        assert_eq!(
            render(&rows(), &groups(), &state),
            [
                "[active 2 collapsed Some(Working)]",
                "[reviews 1 None]",
                "e",
                "[parked 2 collapsed Some(Blocked)]"
            ]
        );
        assert_eq!(
            visible_in_sections(rows(), |row| row.1.map(str::to_owned), &groups(), &state)
                .iter()
                .map(|row| row.0)
                .collect::<Vec<_>>(),
            ["e"]
        );
    }

    #[test]
    fn collapse_state_round_trips_through_preference_keys() {
        let mut state = AgentSectionsCollapsed::default();
        state.toggle(&None);
        state.toggle(&Some("g2".into()));
        state.toggle(&Some("g2".into()));
        state.toggle(&Some("g1".into()));
        assert_eq!(state.keys(), ["", "g1"]);
        assert_eq!(AgentSectionsCollapsed::from_keys(state.keys()), state);
    }

    #[test]
    fn a_blank_line_separates_sections_but_not_a_title_from_its_agents() {
        let rows = rows();
        let groups = groups();
        let entries = agent_panel_entries(
            &rows,
            |row| row.1,
            |row| row.2,
            &groups,
            &AgentSectionsCollapsed::default(),
        );
        assert_eq!(agent_entry_gaps(&entries, 0), [0, 0, 1, 0, 1, 0, 0, 0]);
        assert_eq!(agent_entry_gaps(&entries, 2), [2, 2, 2, 2, 2, 2, 2, 0]);
    }
}
