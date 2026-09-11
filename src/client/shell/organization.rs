// Modified under the Apache License, Version 2.0; see LICENSE.
use std::collections::{HashMap, HashSet};

use crate::api::schema::AgentStatus;
use crate::config::{OrganizationAgentConfig, OrganizationConfig};
use crate::protocol::{ClientShellAgent, ClientShellSnapshot};

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
}

impl OrganizationRow {
    pub(super) fn height(&self) -> usize {
        1usize.saturating_add(
            self.meta
                .lines()
                .count()
                .saturating_sub(usize::from(self.meta.is_empty())),
        )
    }
}

fn status_max(left: AgentStatus, right: AgentStatus) -> AgentStatus {
    if super::status_priority(left) >= super::status_priority(right) {
        left
    } else {
        right
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
) -> String {
    let lines = [
        join_meta([
            entry.model.clone(),
            (!entry.role.trim().is_empty()).then(|| entry.role.clone()),
        ]),
        join_meta([entry.task.clone()]),
        join_meta([
            Some(if !runtime_present {
                "unbound · no runtime".to_owned()
            } else if attention > 0 {
                format!("! {attention}")
            } else {
                super::status_text(status).to_owned()
            }),
            entry.owner_id.clone().map(|owner| format!("owner {owner}")),
            entry.area.clone(),
        ]),
    ];
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
) {
    let entry = &entries[index];
    let key = keys[index].clone();
    let (status, attention) = aggregate[index];
    let expandable = !children[index].is_empty();
    let is_collapsed = collapsed.contains(&key);
    let label = if entry.label.trim().is_empty() {
        runtime[index]
            .map(runtime_label)
            .unwrap_or_else(|| entry.id.clone())
    } else {
        entry.label.clone()
    };
    let meta = if invalid[index] {
        "pending · invalid registration".to_owned()
    } else {
        configured_meta(entry, status, attention, runtime[index].is_some())
    };
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
    let mut pending = Vec::new();
    for index in 0..entries.len() {
        if invalid[index] {
            let (status, attention) = aggregate[index];
            let reason = if duplicate_panes.contains(&index) {
                "duplicate pane binding"
            } else {
                "duplicate, missing parent, or cycle"
            };
            let attention = if duplicate_panes.contains(&index) {
                0
            } else {
                attention
            };
            pending.push(OrganizationRow {
                key: keys[index].clone(),
                parent_key: Some(scoped_group(endpoint_scope, "pending")),
                label: if entries[index].label.is_empty() {
                    entries[index].id.clone()
                } else {
                    entries[index].label.clone()
                },
                meta: format!("pending · {reason}"),
                depth: 2,
                status,
                attention,
                pane_id: runtime[index].map(|agent| agent.pane_id.clone()),
                expandable: false,
                collapsed: false,
                selected: selected == Some(keys[index].as_str()),
                invalid: true,
            });
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
        label: config.root_label().to_owned(),
        meta: format!(
            "{} agents{}",
            snapshot.agents.len(),
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
    });
    if root_collapsed {
        return rows;
    }
    for index in configured_roots {
        emit_configured(
            index,
            1,
            Some(scope_root.clone()),
            &children,
            &entries,
            &runtime,
            &invalid,
            &aggregate,
            collapsed,
            selected,
            &mut rows,
            &keys,
        );
    }
    if !reserved_indices.is_empty() {
        let key = scoped_group(endpoint_scope, "reserve");
        let reserve_collapsed = collapsed.contains(&key);
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
            label: config.reserve_label().to_owned(),
            meta: format!(
                "{} entries{}",
                reserved_indices.len(),
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
                );
            }
        }
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
                meta: format!("unregistered · {}", super::status_text(agent.agent_status)),
                depth: 2,
                status: agent.agent_status,
                attention: usize::from(agent.agent_status == AgentStatus::Blocked),
                pane_id: Some(agent.pane_id.clone()),
                expandable: false,
                collapsed: false,
                selected: selected == Some(key.as_str()),
                invalid: true,
            }
        })
        .collect::<Vec<_>>();
    orphan_rows.sort_by(|left, right| left.label.cmp(&right.label));
    virtual_group(
        endpoint_scope,
        "orphan",
        config.orphan_label(),
        1,
        &orphan_rows,
        collapsed,
        selected,
        &mut rows,
    );
    virtual_group(
        endpoint_scope,
        "pending",
        config.pending_label(),
        1,
        &pending,
        collapsed,
        selected,
        &mut rows,
    );
    rows
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
            "{} entries{}",
            items.len(),
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
        let rows = rows(
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
        let rows = rows(
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
        let reserve = rows.iter().position(|row| row.label == "Reserve").unwrap();
        assert_eq!(rows[reserve + 1].label, "Reserved");
        assert_eq!(rows[reserve + 2].label, "Nested reserved");
    }

    #[test]
    fn synthetic_group_keys_do_not_collide_with_real_registration_ids() {
        let rows = rows(
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
        assert!(duplicate.meta.contains("duplicate pane binding"));
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
        let rows = rows(
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
                .find(|row| row.label == "Reserve")
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
            |row| row.meta.contains("unregistered") && row.pane_id.as_deref() == Some("w1:p1")
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
        assert_eq!(pane_rows.len(), 7, "pane rows: {pane_rows:?}; all rows: {all_rows:?}");
        for (key, pane) in pane_rows {
            let selected = rows(&snapshot, &config, "local", &HashSet::new(), Some(&key))
                .into_iter()
                .filter(|row| row.selected)
                .collect::<Vec<_>>();
            assert_eq!(selected.len(), 1, "selection key {key} must be unique");
            assert_eq!(selected[0].pane_id.as_deref(), Some(pane));
        }
    }
}
