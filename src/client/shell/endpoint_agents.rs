use super::render::put_text;
use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let rows = agent_rows(endpoints, active_endpoint_id, config, &HashSet::new(), None);
    for (index, row) in rows
        .into_iter()
        .filter(|row| !row.agent.pane_id.is_empty())
        .take(area.height as usize)
        .enumerate()
    {
        let rect = Rect::new(area.x, area.y + index as u16, area.width, 1);
        if row.agent.focused {
            buffer.set_style(rect, Style::default().bg(config.palette.active_row_bg));
        }
        let initial = row.machine_label.chars().next().unwrap_or('?');
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{initial}{}",
                status_icon(row.agent.status, config.status_indicators)
            ),
            Style::default()
                .fg(if row.stale {
                    config.palette.overlay0
                } else {
                    status_color(row.agent.status, &config.palette)
                })
                .add_modifier(if row.stale {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                }),
        );
        if !row.agent.pane_id.is_empty() {
            hits.endpoint_agents
                .push((rect, row.endpoint_id, row.agent.pane_id));
        }
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    organization_collapsed: &HashSet<String>,
    selected_organization_key: Option<&str>,
    hits: &mut ShellHitMap,
) {
    if !super::agent_sidebar::render_agent_panel_header(
        buffer,
        area,
        agent_view_label,
        config,
        config.organization.enabled,
        hits,
    ) {
        return;
    }
    let rows = agent_rows(
        endpoints,
        active_endpoint_id,
        config,
        organization_collapsed,
        selected_organization_key,
    );
    super::agent_sidebar::reveal_selected_organization(
        area,
        &rows,
        config,
        agent_scroll,
        selected_organization_key,
        |row| row.agent.organization.as_ref().map(|row| row.key.as_str()),
        |row| {
            row.agent
                .organization
                .as_ref()
                .map_or(row.agent.rows.len(), |row| row.height())
        },
    );
    super::agent_sidebar::render_agent_list(
        buffer,
        area,
        &rows,
        agent_view_label.map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| {
            row.agent
                .organization
                .as_ref()
                .map_or(row.agent.rows.len(), |row| row.height())
        },
        |buffer, rect, row, hits| {
            super::agent_sidebar::render_agent_row(buffer, rect, &row.agent, config, hits);
            if row.stale {
                buffer.set_style(
                    rect,
                    Style::default()
                        .fg(config.palette.overlay0)
                        .add_modifier(Modifier::DIM),
                );
            }
            if !row.agent.pane_id.is_empty() {
                hits.endpoint_agents.push((
                    rect,
                    row.endpoint_id.clone(),
                    row.agent.pane_id.clone(),
                ));
            }
        },
    );
}

struct EndpointAgentRow {
    endpoint_id: ClientEndpointId,
    machine_label: String,
    stale: bool,
    agent: super::agent_sidebar::AgentRow,
}

fn agent_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    organization_collapsed: &HashSet<String>,
    selected_organization_key: Option<&str>,
) -> Vec<EndpointAgentRow> {
    if config.organization.enabled {
        return endpoints
            .iter()
            .filter_map(|endpoint| {
                let snapshot = endpoint.snapshot.as_deref()?;
                let endpoint_scope = endpoint.endpoint_id.storage_key();
                Some(
                    super::agent_sidebar::organization_rows(
                        snapshot,
                        config,
                        &endpoint_scope,
                        organization_collapsed,
                        selected_organization_key,
                    )
                    .into_iter()
                    .map(|agent| EndpointAgentRow {
                        endpoint_id: endpoint.endpoint_id.clone(),
                        machine_label: endpoint.label.clone(),
                        stale: endpoint.status != ClientEndpointStatus::Online,
                        agent,
                    })
                    .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();
    }
    let mut rendered_rows = endpoints
        .iter()
        .filter_map(|endpoint| {
            endpoint.snapshot.as_deref().map(|snapshot| {
                let endpoint_scope = endpoint.endpoint_id.storage_key();
                let rows = if config.organization.enabled {
                    super::agent_sidebar::organization_rows(
                        snapshot,
                        config,
                        &endpoint_scope,
                        organization_collapsed,
                        selected_organization_key,
                    )
                } else {
                    super::agent_sidebar::agent_rows(snapshot, config, Some(&endpoint.label))
                };
                rows.into_iter()
                    .map(|agent| ((endpoint.endpoint_id.clone(), agent.pane_id.clone()), agent))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect::<HashMap<_, _>>();

    super::aggregate_navigation::aggregate_agent_rows(endpoints, config.agent_panel_sort)
        .into_iter()
        .filter_map(|row| {
            let key = (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone());
            let mut agent = rendered_rows.remove(&key)?;
            agent.focused &= row.endpoint.endpoint_id == active_endpoint_id;
            Some(EndpointAgentRow {
                endpoint_id: row.endpoint.endpoint_id.clone(),
                machine_label: row.endpoint.label.to_owned(),
                stale: row.endpoint.stale(),
                agent,
            })
        })
        .collect()
}
