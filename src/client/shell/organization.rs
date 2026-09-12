// Modified under the Apache License, Version 2.0; see LICENSE.
use std::collections::{HashMap, HashSet};

use crate::api::schema::AgentStatus;
use crate::config::{OrganizationAgentConfig, OrganizationConfig, OrganizationLocale};
use crate::protocol::{ClientShellAgent, ClientShellSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OrganizationVisualKind {
    Root,
    Coordinator,
    Reserve,
    Manager,
    Worker,
    Group,
    Unbound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OrganizationRow {
    pub(super) key: String,
    pub(super) parent_key: Option<String>,
    pub(super) label: String,
    pub(super) meta: String,
    pub(super) depth: u16,
    pub(super) status: AgentStatus,
    pub(super) attention: usize,
    pub(super) pane_id: Option<String>,
    pub(super) expandable: bool,
    pub(super) collapsed: bool,
    pub(super) selected: bool,
    pub(super) invalid: bool,
    pub(super) visual_kind: OrganizationVisualKind,
    pub(super) box_start: bool,
    pub(super) box_end: bool,
}

impl OrganizationRow {
    pub(super) fn height(&self) -> usize {
        let content = 1usize.saturating_add(
            self.meta
                .lines()
                .count()
                .saturating_sub(usize::from(self.meta.is_empty())),
        );
        match self.visual_kind {
            OrganizationVisualKind::Root => 3,
            OrganizationVisualKind::Coordinator
            | OrganizationVisualKind::Reserve
            | OrganizationVisualKind::Group
            | OrganizationVisualKind::Manager
            | OrganizationVisualKind::Worker => content.saturating_add(1),
            _ => content,
        }
    }
}

fn visual_kind(entry: &OrganizationAgentConfig, has_children: bool) -> OrganizationVisualKind {
    let role = entry.role.trim().to_ascii_lowercase();
    if is_coordinator(entry) {
        OrganizationVisualKind::Coordinator
    } else if matches!(role.as_str(), "manager" | "gerente") || has_children {
        OrganizationVisualKind::Manager
    } else {
        OrganizationVisualKind::Worker
    }
}

fn is_coordinator(entry: &OrganizationAgentConfig) -> bool {
    matches!(
        entry.role.trim().to_ascii_lowercase().as_str(),
        "coordinator" | "coordenador"
    )
}

fn status_max(left: AgentStatus, right: AgentStatus) -> AgentStatus {
    if super::status_priority(left) >= super::status_priority(right) {
        left
    } else {
        right
    }
}

fn organization_status_text(status: AgentStatus, locale: OrganizationLocale) -> &'static str {
    match (locale, status) {
        (OrganizationLocale::Portuguese, AgentStatus::Working) => "Trabalhando",
        (OrganizationLocale::Portuguese, AgentStatus::Idle) => "Disponível",
        (OrganizationLocale::Portuguese, AgentStatus::Done) => "Concluiu",
        (OrganizationLocale::Portuguese, AgentStatus::Blocked) => "Precisa de atenção",
        (OrganizationLocale::Portuguese, AgentStatus::Unknown) => "Estado não confirmado",
        (OrganizationLocale::English, AgentStatus::Working) => "Working",
        (OrganizationLocale::English, AgentStatus::Idle) => "Available",
        (OrganizationLocale::English, AgentStatus::Done) => "Done",
        (OrganizationLocale::English, AgentStatus::Blocked) => "Needs attention",
        (OrganizationLocale::English, AgentStatus::Unknown) => "State not confirmed",
    }
}

pub(super) fn navigation_hint(locale: OrganizationLocale) -> &'static str {
    match locale {
        OrganizationLocale::Portuguese => " Tab projetos · ↑↓ detalhes · ←→ recolher · Enter foco",
        OrganizationLocale::English => " Tab projects · ↑↓ details · ←→ fold · Enter focus",
    }
}

fn organization_role_text(role: &str, locale: OrganizationLocale) -> Option<String> {
    match (locale, role.trim().to_ascii_lowercase().as_str()) {
        (OrganizationLocale::Portuguese, "coordinator" | "coordenador") => {
            Some("Coordenador".to_owned())
        }
        (OrganizationLocale::Portuguese, "manager" | "gerente") => Some("Gerente".to_owned()),
        (OrganizationLocale::Portuguese, "worker" | "funcionário" | "funcionario") => {
            Some("Funcionário".to_owned())
        }
        (OrganizationLocale::English, "coordinator" | "coordenador") => {
            Some("Coordinator".to_owned())
        }
        (OrganizationLocale::English, "manager" | "gerente") => Some("Manager".to_owned()),
        (OrganizationLocale::English, "worker" | "funcionário" | "funcionario") => {
            Some("Worker".to_owned())
        }
        _ => None,
    }
}

fn runtime_label(agent: &ClientShellAgent) -> String {
    agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref())
        .unwrap_or("unnamed agent")
        .to_owned()
}

fn configured_label(entry: &OrganizationAgentConfig) -> Option<String> {
    entry
        .label
        .trim()
        .is_empty()
        .then(|| entry.display_name.clone())
        .flatten()
        .or_else(|| (!entry.label.trim().is_empty()).then(|| entry.label.clone()))
}

fn join_meta(parts: impl IntoIterator<Item = Option<String>>) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn configured_meta(
    entry: &OrganizationAgentConfig,
    status: AgentStatus,
    attention: usize,
    runtime_present: bool,
    locale: OrganizationLocale,
    detailed: bool,
) -> String {
    let state = if !runtime_present && entry.reserved {
        match locale {
            OrganizationLocale::Portuguese => "Reserva · sem sessão".to_owned(),
            OrganizationLocale::English => "Reserve · no session".to_owned(),
        }
    } else if !runtime_present {
        match locale {
            OrganizationLocale::Portuguese => "Cadastrado · sem sessão".to_owned(),
            OrganizationLocale::English => "Registered · no session".to_owned(),
        }
    } else if attention > 0 {
        format!(
            "{} · ! {attention}",
            organization_status_text(status, locale)
        )
    } else {
        organization_status_text(status, locale).to_owned()
    };
    let mut lines = vec![
        join_meta([Some(state), organization_role_text(&entry.role, locale)]),
        join_meta([entry.task.clone().or_else(|| entry.function.clone())]),
    ];
    if detailed {
        lines.push(join_meta([entry.model.clone(), entry.area.clone()]));
        lines.push(join_meta([
            Some(format!("ID: {}", entry.id)),
            entry.pane_id.clone(),
        ]));
        if let Some(owner) = &entry.owner_id {
            lines.push(format!(
                "{}: {owner}",
                match locale {
                    OrganizationLocale::Portuguese => "Responsável",
                    OrganizationLocale::English => "Owner",
                }
            ));
        }
    }
    lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn mark_cycles(parent: &[Option<usize>], invalid: &mut [bool]) {
    for start in 0..parent.len() {
        let mut path = Vec::new();
        let mut seen = HashMap::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if invalid[index] {
                break;
            }
            if let Some(position) = seen.insert(index, path.len()) {
                for cycle_index in path.into_iter().skip(position) {
                    invalid[cycle_index] = true;
                }
                break;
            }
            path.push(index);
            current = parent[index];
        }
    }
}

fn aggregate_states(
    children: &[Vec<usize>],
    runtime: &[Option<&ClientShellAgent>],
) -> Vec<(AgentStatus, usize)> {
    let mut aggregate = runtime
        .iter()
        .map(|agent| {
            let status = agent.map_or(AgentStatus::Unknown, |agent| agent.agent_status);
            (status, usize::from(status == AgentStatus::Blocked))
        })
        .collect::<Vec<_>>();
    let mut visited = vec![false; children.len()];
    let mut postorder = Vec::with_capacity(children.len());
    for start in 0..children.len() {
        if visited[start] {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((index, expanded)) = stack.pop() {
            if expanded {
                postorder.push(index);
                continue;
            }
            if visited[index] {
                continue;
            }
            visited[index] = true;
            stack.push((index, true));
            for child in children[index].iter().rev() {
                if !visited[*child] {
                    stack.push((*child, false));
                }
            }
        }
    }
    for index in postorder {
        for child in &children[index] {
            aggregate[index].0 = status_max(aggregate[index].0, aggregate[*child].0);
            aggregate[index].1 = aggregate[index].1.saturating_add(aggregate[*child].1);
        }
    }
    aggregate
}

fn scoped_key(scope: &str, namespace: &str, kind: &str, value: &str) -> String {
    format!(
        "organization:v1:scope:{}:{scope}:{namespace}:{kind}:{}:{value}",
        scope.len(),
        value.len()
    )
}

fn scoped_group(scope: &str, key: &str) -> String {
    scoped_key(scope, "group", "virtual", key)
}

fn scoped_agent(scope: &str, kind: &str, value: &str) -> String {
    scoped_key(scope, "agent", kind, value)
}

// The recursive renderer passes the complete immutable tree context so that
// each child uses the same identity, aggregate state, and selection rules.
#[allow(clippy::too_many_arguments)]
fn emit_configured(
    index: usize,
    depth: u16,
    parent_key: Option<String>,
    children: &[Vec<usize>],
    entries: &[OrganizationAgentConfig],
    runtime: &[Option<&ClientShellAgent>],
    invalid: &[bool],
    aggregate: &[(AgentStatus, usize)],
    collapsed: &HashSet<String>,
    selected: Option<&str>,
    rows: &mut Vec<OrganizationRow>,
    keys: &[String],
    locale: OrganizationLocale,
) {
    let entry = &entries[index];
    let key = keys[index].clone();
    let (status, attention) = aggregate[index];
    let expandable = !children[index].is_empty();
    let is_collapsed = collapsed.contains(&key);
    let label = if configured_label(entry).is_none() {
        runtime[index]
            .map(runtime_label)
            .unwrap_or_else(|| entry.id.clone())
    } else {
        configured_label(entry).unwrap_or_default()
    };
    let meta = if invalid[index] {
        match locale {
            OrganizationLocale::Portuguese => "Cadastro pendente · registro inválido".to_owned(),
            OrganizationLocale::English => "Registration pending · invalid registration".to_owned(),
        }
    } else {
        configured_meta(
            entry,
            runtime[index].map_or(status, |agent| agent.agent_status),
            attention,
            runtime[index].is_some(),
            locale,
            selected == Some(key.as_str()),
        )
    };
    let visual_kind = visual_kind(entry, expandable);
    rows.push(OrganizationRow {
        key: key.clone(),
        parent_key,
        label,
        meta,
        depth,
        status,
        attention,
        pane_id: runtime[index].map(|agent| agent.pane_id.clone()),
        expandable,
        collapsed: is_collapsed,
        selected: selected == Some(key.as_str()),
        invalid: invalid[index],
        visual_kind,
        box_start: visual_kind == OrganizationVisualKind::Manager,
        box_end: false,
    });
    if is_collapsed {
        return;
    }
    for child in &children[index] {
        emit_configured(
            *child,
            depth.saturating_add(1),
            Some(key.clone()),
            children,
            entries,
            runtime,
            invalid,
            aggregate,
            collapsed,
            selected,
            rows,
            keys,
            locale,
        );
    }
}

fn applicable(entry: &OrganizationAgentConfig, endpoint_scope: &str) -> bool {
    entry
        .endpoint_id
        .as_deref()
        .map_or(endpoint_scope == "local", |endpoint| {
            endpoint == endpoint_scope
        })
}

pub(super) fn rows(
    snapshot: &ClientShellSnapshot,
    config: &OrganizationConfig,
    endpoint_scope: &str,
    collapsed: &HashSet<String>,
    selected: Option<&str>,
) -> Vec<OrganizationRow> {
    rows_with_locale(
        snapshot,
        config,
        endpoint_scope,
        collapsed,
        selected,
        config.locale(),
        false,
    )
}

pub(super) fn rows_with_locale(
    snapshot: &ClientShellSnapshot,
    config: &OrganizationConfig,
    endpoint_scope: &str,
    collapsed: &HashSet<String>,
    selected: Option<&str>,
    locale: OrganizationLocale,
    reserve_expanded: bool,
) -> Vec<OrganizationRow> {
    let entries = config
        .agents
        .iter()
        .filter(|entry| applicable(entry, endpoint_scope))
        .cloned()
        .collect::<Vec<_>>();
    let mut id_counts = HashMap::<&str, usize>::new();
    for entry in &entries {
        if !entry.id.trim().is_empty() {
            *id_counts.entry(entry.id.trim()).or_default() += 1;
        }
    }
    let keys = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let id = entry.id.trim();
            if id.is_empty() {
                return scoped_agent(endpoint_scope, "invalid", &index.to_string());
            }
            if id_counts.get(id).copied().unwrap_or(0) > 1 {
                return scoped_agent(endpoint_scope, "duplicate", &index.to_string());
            }
            scoped_agent(endpoint_scope, "registration", id)
        })
        .collect::<Vec<_>>();
    let unique_ids = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            !entry.id.trim().is_empty() && id_counts.get(entry.id.trim()) == Some(&1)
        })
        .map(|(index, entry)| (entry.id.trim().to_owned(), index))
        .collect::<HashMap<_, _>>();
    let mut parent = vec![None; entries.len()];
    let mut invalid = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if entry.id.trim().is_empty() || id_counts.get(entry.id.trim()) != Some(&1) {
            invalid[index] = true;
        }
        if let Some(parent_id) = entry
            .parent_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        {
            match unique_ids.get(parent_id.trim()).copied() {
                Some(parent_index) if parent_index != index => parent[index] = Some(parent_index),
                _ => invalid[index] = true,
            }
        }
    }
    mark_cycles(&parent, &mut invalid);
    let mut pane_owners = HashMap::<&str, usize>::new();
    let mut duplicate_panes = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if let Some(pane_id) = entry.pane_id.as_deref().filter(|id| !id.trim().is_empty()) {
            if pane_owners.insert(pane_id, index).is_some() {
                duplicate_panes.insert(index);
                invalid[index] = true;
            }
        }
    }
    loop {
        let mut changed = false;
        for index in 0..entries.len() {
            if !invalid[index] && parent[index].is_some_and(|parent| invalid[parent]) {
                invalid[index] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut children = vec![Vec::new(); entries.len()];
    for (index, parent_index) in parent.iter().enumerate() {
        if invalid[index] {
            continue;
        }
        if let Some(parent_index) = parent_index {
            if !entries[index].reserved || entries[*parent_index].reserved {
                children[*parent_index].push(index);
            }
        }
    }
    let mut runtime_by_pane = HashMap::<&str, &ClientShellAgent>::new();
    for agent in &snapshot.agents {
        runtime_by_pane.entry(&agent.pane_id).or_insert(agent);
    }
    let runtime = entries
        .iter()
        .map(|entry| {
            entry
                .pane_id
                .as_deref()
                .and_then(|id| runtime_by_pane.get(id).copied())
        })
        .collect::<Vec<_>>();
    let aggregate = aggregate_states(&children, &runtime);
    let registered_panes = entries
        .iter()
        .filter_map(|entry| entry.pane_id.as_deref())
        .collect::<HashSet<_>>();
    let mut configured_roots = Vec::new();
    let mut reserved_indices = Vec::new();
    let mut reserved_coordinators = Vec::new();
    let mut pending = Vec::new();
    for index in 0..entries.len() {
        if invalid[index] {
            let (status, attention) = aggregate[index];
            let reason = if duplicate_panes.contains(&index) {
                match locale {
                    OrganizationLocale::Portuguese => "vínculo de pane duplicado",
                    OrganizationLocale::English => "duplicate pane binding",
                }
            } else {
                match locale {
                    OrganizationLocale::Portuguese => "id duplicado, pai ausente ou ciclo",
                    OrganizationLocale::English => "duplicate id, missing parent, or cycle",
                }
            };
            let attention = if duplicate_panes.contains(&index) {
                0
            } else {
                attention
            };
            pending.push(OrganizationRow {
                key: keys[index].clone(),
                parent_key: Some(scoped_group(endpoint_scope, "pending")),
                label: if configured_label(&entries[index]).is_none() {
                    entries[index].id.clone()
                } else {
                    configured_label(&entries[index]).unwrap_or_default()
                },
                meta: format!(
                    "{} · {reason}",
                    match locale {
                        OrganizationLocale::Portuguese => "Cadastro pendente",
                        OrganizationLocale::English => "Registration pending",
                    }
                ),
                depth: 2,
                status,
                attention,
                pane_id: runtime[index].map(|agent| agent.pane_id.clone()),
                expandable: false,
                collapsed: false,
                selected: selected == Some(keys[index].as_str()),
                invalid: true,
                visual_kind: OrganizationVisualKind::Unbound,
                box_start: false,
                box_end: false,
            });
        } else if entries[index].reserved
            && is_coordinator(&entries[index])
            && parent[index].is_none()
        {
            reserved_coordinators.push(index);
        } else if entries[index].reserved
            && parent[index].is_none_or(|parent| !entries[parent].reserved)
        {
            reserved_indices.push(index);
        } else if !entries[index].reserved && parent[index].is_none() {
            configured_roots.push(index);
        }
    }
    let scope_root = scoped_group(endpoint_scope, "root");
    let mut rows = Vec::new();
    let root_status = snapshot
        .agents
        .iter()
        .map(|agent| agent.agent_status)
        .max_by_key(|status| super::status_priority(*status))
        .unwrap_or(AgentStatus::Unknown);
    let root_attention = snapshot
        .agents
        .iter()
        .filter(|agent| agent.agent_status == AgentStatus::Blocked)
        .count();
    let root_collapsed = collapsed.contains(&scope_root);
    rows.push(OrganizationRow {
        key: scope_root.clone(),
        parent_key: None,
        label: config.root_label_for(locale).to_owned(),
        meta: format!(
            "{} {}{}",
            snapshot.agents.len(),
            match locale {
                OrganizationLocale::Portuguese => "agentes",
                OrganizationLocale::English => "agents",
            },
            if root_attention > 0 {
                format!(" · ! {root_attention}")
            } else {
                String::new()
            }
        ),
        depth: 0,
        status: root_status,
        attention: root_attention,
        pane_id: None,
        expandable: true,
        collapsed: root_collapsed,
        selected: selected == Some(scope_root.as_str()),
        invalid: false,
        visual_kind: OrganizationVisualKind::Root,
        box_start: false,
        box_end: false,
    });
    // Collapsing the workspace tree does not remove its coordination panel.
    let mut coordinator_collapsed = collapsed.clone();
    if root_collapsed {
        for (index, entry) in entries.iter().enumerate() {
            if is_coordinator(entry) {
                coordinator_collapsed.insert(keys[index].clone());
            }
        }
    }
    for index in configured_roots {
        if root_collapsed && !is_coordinator(&entries[index]) {
            continue;
        }
        emit_configured(
            index,
            1,
            Some(scope_root.clone()),
            &children,
            &entries,
            &runtime,
            &invalid,
            &aggregate,
            &coordinator_collapsed,
            selected,
            &mut rows,
            &keys,
            locale,
        );
    }
    // One existing reserve, not a second placeholder coordinator. Its expansion
    // is client presentation state and is independent of manager folding.
    reserved_coordinators.extend(reserved_indices);
    let reserved_indices = reserved_coordinators;
    if !reserved_indices.is_empty() {
        let key = scoped_group(endpoint_scope, "reserve");
        let reserve_collapsed = !reserve_expanded;
        let status = reserved_indices
            .iter()
            .map(|index| aggregate[*index].0)
            .max_by_key(|status| super::status_priority(*status))
            .unwrap_or(AgentStatus::Unknown);
        let attention = reserved_indices
            .iter()
            .map(|index| aggregate[*index].1)
            .sum::<usize>();
        rows.push(OrganizationRow {
            key: key.clone(),
            parent_key: Some(scope_root.clone()),
            label: config.reserve_label_for(locale).to_owned(),
            meta: format!(
                "{} {}{}",
                reserved_indices.len(),
                match locale {
                    OrganizationLocale::Portuguese => "na reserva",
                    OrganizationLocale::English => "in reserve",
                },
                if attention > 0 {
                    format!(" · ! {attention}")
                } else {
                    String::new()
                }
            ),
            depth: 1,
            status,
            attention,
            pane_id: None,
            expandable: true,
            collapsed: reserve_collapsed,
            selected: selected == Some(key.as_str()),
            invalid: false,
            visual_kind: OrganizationVisualKind::Reserve,
            box_start: false,
            box_end: false,
        });
        if !reserve_collapsed {
            for index in reserved_indices {
                emit_configured(
                    index,
                    2,
                    Some(key.clone()),
                    &children,
                    &entries,
                    &runtime,
                    &invalid,
                    &aggregate,
                    collapsed,
                    selected,
                    &mut rows,
                    &keys,
                    locale,
                );
            }
        }
    }
    if root_collapsed {
        move_coordinators_to_panel(&mut rows);
        return rows;
    }
    let mut orphan_rows = snapshot
        .agents
        .iter()
        .filter(|agent| !registered_panes.contains(agent.pane_id.as_str()))
        .map(|agent| {
            let key = scoped_agent(endpoint_scope, "orphan", &agent.pane_id);
            OrganizationRow {
                key: key.clone(),
                parent_key: Some(scoped_group(endpoint_scope, "orphan")),
                label: runtime_label(agent),
                meta: format!(
                    "{} · {}",
                    match locale {
                        OrganizationLocale::Portuguese => "Sem cadastro",
                        OrganizationLocale::English => "Unassigned",
                    },
                    organization_status_text(agent.agent_status, locale)
                ),
                depth: 2,
                status: agent.agent_status,
                attention: usize::from(agent.agent_status == AgentStatus::Blocked),
                pane_id: Some(agent.pane_id.clone()),
                expandable: false,
                collapsed: false,
                selected: selected == Some(key.as_str()),
                invalid: true,
                visual_kind: OrganizationVisualKind::Unbound,
                box_start: false,
                box_end: false,
            }
        })
        .collect::<Vec<_>>();
    orphan_rows.sort_by(|left, right| left.label.cmp(&right.label));
    virtual_group(
        endpoint_scope,
        "orphan",
        config.orphan_label_for(locale),
        1,
        &orphan_rows,
        collapsed,
        selected,
        &mut rows,
        locale,
    );
    virtual_group(
        endpoint_scope,
        "pending",
        config.pending_label_for(locale),
        1,
        &pending,
        collapsed,
        selected,
        &mut rows,
        locale,
    );
    move_coordinators_to_panel(&mut rows);
    annotate_visual_boxes(&mut rows);
    rows
}

fn move_coordinators_to_panel(rows: &mut Vec<OrganizationRow>) {
    let Some(root) = rows.first().cloned() else {
        return;
    };
    let mut coordination = Vec::new();
    let mut rest = Vec::new();
    let mut reserve_keys = HashSet::new();
    for row in rows.iter().skip(1) {
        let in_reserve = row.visual_kind == OrganizationVisualKind::Reserve
            || row
                .parent_key
                .as_ref()
                .is_some_and(|key| reserve_keys.contains(key));
        if in_reserve {
            reserve_keys.insert(row.key.clone());
        }
        if row.visual_kind == OrganizationVisualKind::Coordinator || in_reserve {
            coordination.push(row.clone());
        } else {
            rest.push(row.clone());
        }
    }
    let mut reordered = Vec::with_capacity(rows.len());
    reordered.push(root);
    for row in &mut coordination {
        row.box_end = false;
    }
    if let Some(last) = coordination.last_mut() {
        last.box_end = true;
    } else {
        reordered[0].box_end = true;
    }
    reordered.append(&mut coordination);
    reordered.append(&mut rest);
    *rows = reordered;
}

pub(super) fn is_reserve_key(key: &str) -> bool {
    let Some((length, scoped)) = key
        .strip_prefix("organization:v1:scope:")
        .and_then(|value| value.split_once(':'))
    else {
        return false;
    };
    length
        .parse::<usize>()
        .ok()
        .is_some_and(|length| scoped.get(length..) == Some(":group:virtual:7:reserve"))
}

pub(super) fn reserve_key(scope: &str) -> String {
    scoped_group(scope, "reserve")
}

fn annotate_visual_boxes(rows: &mut [OrganizationRow]) {
    for index in 0..rows.len() {
        if rows[index].visual_kind != OrganizationVisualKind::Manager {
            continue;
        }
        let depth = rows[index].depth;
        let end = rows
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, row)| row.depth <= depth)
            .map_or(rows.len(), |(index, _)| index);
        rows[index].box_start = true;
        rows[index].box_end = end == index + 1;
        for row in rows.iter_mut().take(end).skip(index + 1) {
            row.box_start = false;
            row.box_end = false;
        }
        if end > index + 1 {
            rows[end - 1].box_end = true;
        }
    }
}

fn virtual_group(
    scope: &str,
    group: &str,
    label: &str,
    depth: u16,
    items: &[OrganizationRow],
    collapsed: &HashSet<String>,
    selected: Option<&str>,
    rows: &mut Vec<OrganizationRow>,
    locale: OrganizationLocale,
) {
    if items.is_empty() {
        return;
    }
    let key = scoped_group(scope, group);
    let status = items
        .iter()
        .map(|row| row.status)
        .max_by_key(|status| super::status_priority(*status))
        .unwrap_or(AgentStatus::Unknown);
    let attention = items.iter().map(|row| row.attention).sum::<usize>();
    let is_collapsed = collapsed.contains(&key);
    rows.push(OrganizationRow {
        key: key.clone(),
        parent_key: Some(scoped_group(scope, "root")),
        label: label.to_owned(),
        meta: format!(
            "{} {}{}",
            items.len(),
            match locale {
                OrganizationLocale::Portuguese => "cadastros",
                OrganizationLocale::English => "registrations",
            },
            if attention > 0 {
                format!(" · ! {attention}")
            } else {
                String::new()
            }
        ),
        depth,
        status,
        attention,
        pane_id: None,
        expandable: true,
        collapsed: is_collapsed,
        selected: selected == Some(key.as_str()),
        invalid: group == "pending",
        visual_kind: OrganizationVisualKind::Group,
        box_start: false,
        box_end: false,
    });
    if !is_collapsed {
        rows.extend(items.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot(panes: &[(&str, AgentStatus)]) -> ClientShellSnapshot {
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.agents = panes
            .iter()
            .map(|(pane_id, status)| ClientShellAgent {
                pane_id: (*pane_id).into(),
                workspace_id: "ws_1".into(),
                tab_id: "tab_1".into(),
                name: Some((*pane_id).into()),
                display_agent: None,
                agent: Some("pi".into()),
                title: None,
                terminal_title: None,
                terminal_title_stripped: None,
                agent_status: *status,
                state_change_seq: 1,
                state_labels: Vec::new(),
                tokens: Vec::new(),
                focused: false,
            })
            .collect();
        snapshot
    }
    fn config(entries: Vec<OrganizationAgentConfig>) -> OrganizationConfig {
        OrganizationConfig {
            enabled: true,
            root: "Argos".into(),
            agents: entries,
            ..Default::default()
        }
    }

    fn expanded_rows(
        snapshot: &ClientShellSnapshot,
        config: &OrganizationConfig,
        scope: &str,
        collapsed: &HashSet<String>,
        selected: Option<&str>,
    ) -> Vec<OrganizationRow> {
        rows_with_locale(
            snapshot,
            config,
            scope,
            collapsed,
            selected,
            config.locale(),
            true,
        )
    }

    #[test]
    fn organization_reserve_defaults_closed_and_reveals_the_real_pane_once() {
        let snapshot = test_snapshot(&[("reserve-pane", AgentStatus::Idle)]);
        let config = config(vec![OrganizationAgentConfig {
            id: "reserve-id".into(),
            label: "Argos Reserva".into(),
            pane_id: Some("reserve-pane".into()),
            role: "coordinator".into(),
            reserved: true,
            ..Default::default()
        }]);
        let closed = rows(&snapshot, &config, "local", &HashSet::new(), None);
        let group = closed
            .iter()
            .find(|row| row.visual_kind == OrganizationVisualKind::Reserve)
            .unwrap();
        assert!(group.collapsed && group.expandable && group.box_end);
        assert!(group.pane_id.is_none());
        assert!(!closed.iter().any(|row| row.pane_id.is_some()));
        let open = expanded_rows(&snapshot, &config, "local", &HashSet::new(), None);
        let real = open
            .iter()
            .filter(|row| row.pane_id.as_deref() == Some("reserve-pane"))
            .collect::<Vec<_>>();
        assert_eq!(real.len(), 1);
        assert_eq!(real[0].parent_key.as_deref(), Some(group.key.as_str()));
        assert!(real[0].meta.contains("Disponível"));
        assert!(!real[0].meta.contains("sem sessão"));
        assert!(real[0].box_end);
        assert_ne!(reserve_key("local"), reserve_key("ssh:nyx"));
        assert!(is_reserve_key(&reserve_key("ssh:nyx")));
        assert!(!is_reserve_key(
            "organization:v1:scope:5:local:agent:registration:23:group:virtual:7:reserve"
        ));
    }

    #[test]
    fn organization_parent_keeps_own_runtime_state_and_child_attention() {
        let snapshot = test_snapshot(&[
            ("manager", AgentStatus::Idle),
            ("worker", AgentStatus::Blocked),
        ]);
        let config = config(vec![
            OrganizationAgentConfig {
                id: "manager".into(),
                pane_id: Some("manager".into()),
                role: "manager".into(),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "worker".into(),
                pane_id: Some("worker".into()),
                parent_id: Some("manager".into()),
                role: "worker".into(),
                ..Default::default()
            },
        ]);
        let rows = rows(&snapshot, &config, "local", &HashSet::new(), None);
        let manager = rows
            .iter()
            .find(|row| row.pane_id.as_deref() == Some("manager"))
            .unwrap();
        assert!(manager.meta.starts_with("Disponível · ! 1"));
        assert!(!manager.meta.contains("Precisa de atenção"));
        assert_eq!(manager.attention, 1);
        assert_eq!(
            manager.status,
            AgentStatus::Blocked,
            "box still signals child attention"
        );
    }

    #[test]
    fn organization_selection_reveals_metadata_without_changing_identity_or_state() {
        let snapshot = test_snapshot(&[("pane-worker", AgentStatus::Blocked)]);
        let config = config(vec![OrganizationAgentConfig {
            id: "worker".into(),
            label: "Interface".into(),
            pane_id: Some("pane-worker".into()),
            role: "worker".into(),
            model: Some("gpt-5.6-luna".into()),
            function: Some("Revisar interface".into()),
            ..Default::default()
        }]);
        let compact = rows(&snapshot, &config, "local", &HashSet::new(), None);
        let compact = compact.iter().find(|row| row.label == "Interface").unwrap();
        assert!(compact.meta.contains("Precisa de atenção"));
        assert!(compact.meta.contains("Revisar interface"));
        assert!(!compact.meta.contains("gpt-5.6-luna"));
        assert!(!compact.meta.contains("ID:"));
        let detailed = rows(
            &snapshot,
            &config,
            "local",
            &HashSet::new(),
            Some(&compact.key),
        );
        let detailed = detailed.iter().find(|row| row.selected).unwrap();
        assert!(detailed.meta.contains("gpt-5.6-luna"));
        assert!(detailed.meta.contains("ID: worker · pane-worker"));
        assert_eq!(detailed.pane_id, compact.pane_id);
        assert_eq!(detailed.status, compact.status);
        assert_eq!(detailed.attention, compact.attention);
    }

    #[test]
    fn organization_rows_use_human_roles_states_and_explicit_registration_status() {
        let rows = expanded_rows(
            &test_snapshot(&[
                ("working", AgentStatus::Working),
                ("idle", AgentStatus::Idle),
                ("done", AgentStatus::Done),
                ("blocked", AgentStatus::Blocked),
            ]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "working".into(),
                    label: "Equipe ativa".into(),
                    pane_id: Some("working".into()),
                    role: "manager".into(),
                    model: Some("gpt-5.6-luna".into()),
                    task: Some("Revisar a entrega".into()),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "idle".into(),
                    label: "Equipe disponivel".into(),
                    pane_id: Some("idle".into()),
                    role: "worker".into(),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "done".into(),
                    label: "Equipe concluida".into(),
                    pane_id: Some("done".into()),
                    role: "funcionario".into(),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "blocked".into(),
                    label: "Equipe atencao".into(),
                    pane_id: Some("blocked".into()),
                    role: "coordenador".into(),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "reserved".into(),
                    label: "Reserva Fable".into(),
                    role: "worker".into(),
                    reserved: true,
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        let meta = |label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .map(|row| row.meta.as_str())
                .unwrap_or_else(|| panic!("missing organization row {label}"))
        };
        assert!(meta("Equipe ativa").contains("Gerente"));
        assert!(!meta("Equipe ativa").contains("gpt-5.6-luna"));
        assert!(meta("Equipe ativa").contains("Revisar a entrega"));
        assert!(meta("Equipe ativa").contains("Trabalhando"));
        assert!(meta("Equipe disponivel").contains("Disponível"));
        assert!(meta("Equipe concluida").contains("Concluiu"));
        assert!(meta("Equipe atencao").contains("Precisa de atenção · ! 1"));
        assert!(meta("Reserva Fable").contains("Reserva · sem sessão"));
        assert!(!meta("Equipe atencao").contains("Aguardando"));
    }

    #[test]
    fn organization_rows_render_the_same_registration_in_english() {
        let mut organization = config(vec![OrganizationAgentConfig {
            id: "worker".into(),
            label: "Interface".into(),
            pane_id: Some("worker".into()),
            role: "worker".into(),
            function: Some("Native interface".into()),
            ..Default::default()
        }]);
        organization.locale = "en".into();
        let rows = rows(
            &test_snapshot(&[("worker", AgentStatus::Working)]),
            &organization,
            "local",
            &HashSet::new(),
            None,
        );
        let worker = rows.iter().find(|row| row.label == "Interface").unwrap();
        assert!(worker.meta.contains("Worker"));
        assert!(worker.meta.contains("Working"));
        assert!(worker.meta.contains("Native interface"));
        assert!(!worker.meta.contains("Trabalhando"));
    }

    #[test]
    fn invalid_parent_keeps_child_visible_and_focusable() {
        let rows = rows(
            &test_snapshot(&[("w1:p1", AgentStatus::Working)]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "area".into(),
                    label: "Area".into(),
                    parent_id: Some("missing".into()),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "worker".into(),
                    label: "Worker".into(),
                    parent_id: Some("area".into()),
                    pane_id: Some("w1:p1".into()),
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        assert!(rows
            .iter()
            .any(|row| row.label == "Worker" && row.pane_id.as_deref() == Some("w1:p1")));
    }
    #[test]
    fn reserved_parent_keeps_descendant_under_reserve() {
        let rows = expanded_rows(
            &test_snapshot(&[("w1:p1", AgentStatus::Working)]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "reserve".into(),
                    label: "Reserved manager".into(),
                    reserved: true,
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "worker".into(),
                    label: "Worker".into(),
                    parent_id: Some("reserve".into()),
                    pane_id: Some("w1:p1".into()),
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        let reserve = rows
            .iter()
            .position(|row| row.label == "Reserved manager")
            .expect("reserved row");
        assert_eq!(rows[reserve + 1].label, "Worker");
    }

    #[test]
    fn reserved_with_parent_is_emitted_once_and_nested_reserved_stays_nested() {
        let rows = expanded_rows(
            &test_snapshot(&[("w1:p1", AgentStatus::Working)]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "area".into(),
                    label: "Area".into(),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "reserved".into(),
                    label: "Reserved".into(),
                    parent_id: Some("area".into()),
                    reserved: true,
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "nested".into(),
                    label: "Nested reserved".into(),
                    parent_id: Some("reserved".into()),
                    reserved: true,
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "worker".into(),
                    label: "Worker".into(),
                    parent_id: Some("nested".into()),
                    pane_id: Some("w1:p1".into()),
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        assert_eq!(rows.iter().filter(|row| row.label == "Reserved").count(), 1);
        assert_eq!(
            rows.iter()
                .filter(|row| row.label == "Nested reserved")
                .count(),
            1
        );
        assert_eq!(rows.iter().filter(|row| row.label == "Worker").count(), 1);
        let reserve = rows.iter().position(|row| row.label == "Reserva").unwrap();
        assert_eq!(rows[reserve + 1].label, "Reserved");
        assert_eq!(rows[reserve + 2].label, "Nested reserved");
    }

    #[test]
    fn synthetic_group_keys_do_not_collide_with_real_registration_ids() {
        let rows = expanded_rows(
            &test_snapshot(&[]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "root".into(),
                    label: "Real root".into(),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "reserve".into(),
                    label: "Real reserve".into(),
                    reserved: true,
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        let keys = rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(keys.len(), rows.len());
        assert!(rows.iter().any(|row| row.label == "Real root"));
        assert!(rows.iter().any(|row| row.label == "Real reserve"));
    }

    #[test]
    fn duplicate_pane_binding_is_pending_but_original_pane_remains_focusable() {
        let rows = rows(
            &test_snapshot(&[("w1:p1", AgentStatus::Blocked)]),
            &config(vec![
                OrganizationAgentConfig {
                    id: "primary".into(),
                    label: "Primary".into(),
                    pane_id: Some("w1:p1".into()),
                    ..Default::default()
                },
                OrganizationAgentConfig {
                    id: "duplicate".into(),
                    label: "Duplicate".into(),
                    pane_id: Some("w1:p1".into()),
                    ..Default::default()
                },
            ]),
            "local",
            &HashSet::new(),
            None,
        );
        assert_eq!(rows.iter().filter(|row| row.label == "Primary").count(), 1);
        let duplicate = rows.iter().find(|row| row.label == "Duplicate").unwrap();
        assert!(duplicate.invalid);
        assert!(duplicate.meta.contains("vínculo de pane duplicado"));
        assert_eq!(duplicate.pane_id.as_deref(), Some("w1:p1"));
        assert_eq!(duplicate.attention, 0);
        assert_eq!(
            rows.iter()
                .filter(|row| row.pane_id.as_deref() == Some("w1:p1"))
                .count(),
            2
        );
    }
    #[test]
    fn reservation_alone_has_no_attention() {
        let rows = expanded_rows(
            &test_snapshot(&[]),
            &config(vec![OrganizationAgentConfig {
                id: "reserve".into(),
                reserved: true,
                ..Default::default()
            }]),
            "local",
            &HashSet::new(),
            None,
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.visual_kind == OrganizationVisualKind::Reserve)
                .map(|row| row.attention),
            Some(0)
        );
    }
    #[test]
    fn endpoint_scope_prevents_same_pane_id_cross_machine_match() {
        let config = config(vec![OrganizationAgentConfig {
            id: "remote".into(),
            label: "Remote".into(),
            endpoint_id: Some("ssh:one".into()),
            pane_id: Some("w1:p1".into()),
            ..Default::default()
        }]);
        let snapshot = test_snapshot(&[("w1:p1", AgentStatus::Working)]);
        let other = rows(&snapshot, &config, "ssh:two", &HashSet::new(), None);
        assert!(!other.iter().any(|row| row.label == "Remote"));
        assert!(other.iter().any(
            |row| row.meta.contains("Sem cadastro") && row.pane_id.as_deref() == Some("w1:p1")
        ));
        assert!(rows(&snapshot, &config, "ssh:one", &HashSet::new(), None)
            .iter()
            .any(|row| row.pane_id.as_deref() == Some("w1:p1")));
    }

    #[test]
    fn registration_key_namespaces_are_distinct_and_each_pane_remains_focusable() {
        let entries = vec![
            OrganizationAgentConfig {
                id: String::new(),
                label: "Invalid index zero".into(),
                pane_id: Some("invalid-pane".into()),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "invalid:0".into(),
                label: "Literal invalid key".into(),
                pane_id: Some("literal-invalid-pane".into()),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "duplicate".into(),
                label: "Duplicate first".into(),
                pane_id: Some("duplicate-one".into()),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "duplicate".into(),
                label: "Duplicate second".into(),
                pane_id: Some("duplicate-two".into()),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "duplicate:duplicate:3".into(),
                label: "Literal duplicate key".into(),
                pane_id: Some("literal-duplicate-pane".into()),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "orphan:w1:p1".into(),
                label: "Registered orphan-shaped id".into(),
                pane_id: Some("registered-pane".into()),
                ..Default::default()
            },
        ];
        let snapshot = test_snapshot(&[
            ("invalid-pane", AgentStatus::Working),
            ("literal-invalid-pane", AgentStatus::Working),
            ("duplicate-one", AgentStatus::Working),
            ("duplicate-two", AgentStatus::Working),
            ("literal-duplicate-pane", AgentStatus::Working),
            ("registered-pane", AgentStatus::Working),
            ("w1:p1", AgentStatus::Working),
        ]);
        let config = config(entries);
        let all_rows = rows(&snapshot, &config, "local", &HashSet::new(), None);
        let pane_rows = all_rows
            .iter()
            .filter_map(|row| row.pane_id.as_deref().map(|pane| (row.key.clone(), pane)))
            .collect::<Vec<_>>();
        let keys = pane_rows
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(keys.len(), pane_rows.len());
        assert_eq!(
            pane_rows.len(),
            7,
            "pane rows: {pane_rows:?}; all rows: {all_rows:?}"
        );
        for (key, pane) in pane_rows {
            let selected = rows(&snapshot, &config, "local", &HashSet::new(), Some(&key))
                .into_iter()
                .filter(|row| row.selected)
                .collect::<Vec<_>>();
            assert_eq!(selected.len(), 1, "selection key {key} must be unique");
            assert_eq!(selected[0].pane_id.as_deref(), Some(pane));
        }
    }

    #[test]
    fn collapsed_root_keeps_active_and_reserved_coordinators_in_argos_panel() {
        let config = config(vec![
            OrganizationAgentConfig {
                id: "argos".into(),
                label: "Argos active".into(),
                role: "coordinator".into(),
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "argos-reserve".into(),
                label: "Argos reserve".into(),
                role: "coordinator".into(),
                reserved: true,
                ..Default::default()
            },
            OrganizationAgentConfig {
                id: "manager".into(),
                label: "Manager".into(),
                parent_id: Some("argos".into()),
                role: "manager".into(),
                ..Default::default()
            },
        ]);
        let snapshot = test_snapshot(&[]);
        let root = scoped_group("local", "root");
        let rows = rows(&snapshot, &config, "local", &HashSet::from([root]), None);
        let labels = rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Argos active"));
        assert!(labels.contains(&"Reserva"));
        assert!(!labels.contains(&"Argos reserve"));
        assert!(!labels.contains(&"Manager"));
    }

    #[test]
    fn root_without_coordinator_receives_a_closed_visual_boundary() {
        let rows = rows(
            &test_snapshot(&[]),
            &config(vec![OrganizationAgentConfig {
                id: "manager".into(),
                label: "Manager".into(),
                role: "manager".into(),
                ..Default::default()
            }]),
            "local",
            &HashSet::new(),
            None,
        );
        assert_eq!(rows[0].visual_kind, OrganizationVisualKind::Root);
        assert!(rows[0].box_end);
    }
}
