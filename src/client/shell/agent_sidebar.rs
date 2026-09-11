// Modified under the Apache License, Version 2.0; see LICENSE.
use std::collections::{HashMap, HashSet};

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
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
    pub(super) organization: Option<super::organization::OrganizationRow>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
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
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    organization_collapsed: &HashSet<String>,
    selected_organization_key: Option<&str>,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        config.organization.enabled,
        hits,
    ) {
        return;
    }

    let rows = if config.organization.enabled {
        organization_rows(
            snapshot,
            config,
            "local",
            organization_collapsed,
            selected_organization_key,
        )
    } else {
        agent_rows(snapshot, config, None)
    };
    reveal_selected_organization(
        area,
        &rows,
        config,
        agent_scroll,
        selected_organization_key,
        |row| row.organization.as_ref().map(|row| row.key.as_str()),
        |row| {
            row.organization
                .as_ref()
                .map_or(row.rows.len(), |row| row.height())
        },
    );
    render_agent_list(
        buffer,
        area,
        &rows,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| {
            row.organization
                .as_ref()
                .map_or(row.rows.len(), |row| row.height())
        },
        |buffer, rect, row, hits| {
            if !row.pane_id.is_empty() {
                hits.agents.push((rect, row.pane_id.clone()));
            }
            render_agent_row(buffer, rect, row, config, hits);
        },
    );
}

pub(super) fn reveal_selected_organization<T>(
    area: Rect,
    rows: &[T],
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    selected: Option<&str>,
    key: impl Fn(&T) -> Option<&str>,
    row_lines: impl Fn(&T) -> usize,
) {
    let Some(selected) = selected else {
        return;
    };
    let Some(target) = rows.iter().position(|row| key(row) == Some(selected)) else {
        return;
    };
    let heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    *agent_scroll = super::scroll::list_scroll_start_to_reveal(
        &heights,
        &gaps,
        area.height.saturating_sub(3),
        *agent_scroll,
        target,
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    organization: bool,
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
        if organization {
            " organization"
        } else {
            " agents"
        },
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = agent_view_label.unwrap_or(if organization {
        "tree"
    } else {
        match config.agent_panel_sort {
            crate::config::AgentPanelSortConfig::Spaces => "grouped",
            crate::config::AgentPanelSortConfig::Priority => "priority",
        }
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() && !organization
    {
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
    rows: &[T],
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
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
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
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| {
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
                rows,
                organization: None,
            })
        })
        .collect()
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    if let Some(organization) = row.organization.as_ref() {
        render_organization_row(buffer, rect, organization, row.focused, config, hits);
        return;
    }
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
    let status_style = Style::default()
        .fg(status_color(row.status, palette))
        .add_modifier(if row.focused {
            Modifier::empty()
        } else {
            Modifier::DIM
        });
    let secondary = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::DIM);
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

pub(super) fn organization_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    endpoint_scope: &str,
    collapsed: &HashSet<String>,
    selected: Option<&str>,
) -> Vec<AgentRow> {
    super::organization::rows(
        snapshot,
        &config.organization,
        endpoint_scope,
        collapsed,
        selected,
    )
    .into_iter()
    .map(|organization| AgentRow {
        pane_id: organization.pane_id.clone().unwrap_or_default(),
        status: organization.status,
        focused: organization
            .pane_id
            .as_deref()
            .is_some_and(|pane_id| snapshot.focused_pane_id.as_deref() == Some(pane_id)),
        rows: Vec::new(),
        organization: Some(organization),
    })
    .collect()
}

fn render_organization_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &super::organization::OrganizationRow,
    focused: bool,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let background = if row.selected {
        palette.selection_bg
    } else if focused {
        palette.active_row_bg
    } else {
        ratatui::style::Color::Reset
    };
    if background != ratatui::style::Color::Reset {
        buffer.set_style(rect, Style::default().bg(background));
    }
    let indent = row.depth.saturating_mul(2);
    let marker = if row.expandable {
        if row.collapsed {
            "▸"
        } else {
            "▾"
        }
    } else if row.invalid {
        "?"
    } else {
        status_icon(row.status, config.status_indicators)
    };
    let marker_x = rect
        .x
        .saturating_add(indent)
        .min(rect.right().saturating_sub(1));
    if row.expandable {
        hits.organization_toggles.push((
            Rect::new(
                marker_x,
                rect.y,
                1.min(rect.width),
                u16::from(rect.height > 0),
            ),
            row.key.clone(),
        ));
    }
    put_text(
        buffer,
        marker_x,
        rect.y,
        rect.right().saturating_sub(marker_x),
        marker,
        Style::default().fg(if row.invalid {
            palette.yellow
        } else {
            status_color(row.status, palette)
        }),
    );
    let label_x = marker_x.saturating_add(2);
    let label_style = Style::default()
        .fg(if row.selected || focused {
            palette.text
        } else {
            palette.subtext0
        })
        .add_modifier(if row.selected || focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
    put_text(
        buffer,
        label_x,
        rect.y,
        rect.right().saturating_sub(label_x),
        &row.label,
        label_style,
    );
    if rect.height > 1 && !row.meta.is_empty() {
        let meta_x = label_x.saturating_add(1);
        for (line_index, line) in row.meta.lines().enumerate() {
            let y = rect.y.saturating_add(1).saturating_add(line_index as u16);
            if y >= rect.bottom() {
                break;
            }
            put_text(
                buffer,
                meta_x,
                y,
                rect.right().saturating_sub(meta_x),
                line,
                Style::default()
                    .fg(palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
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
