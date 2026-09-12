//! Projection of existing organization rows and pane surfaces, not a second registry.
use super::organization::{self, OrganizationRow, OrganizationVisualKind as Kind};
use crate::api::schema::AgentStatus;
use crate::config::OrganizationConfig;
use crate::protocol::{ClientShellSnapshot, PaneSurfaceFrame};
use std::collections::HashSet;

#[path = "../../platform/windows/organization_probe.rs"]
pub(super) mod paint;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct Bounds(pub f32, pub f32, pub f32, pub f32);
impl Bounds {
    pub(super) fn contains(self, x: f32, y: f32) -> bool {
        x >= self.0 && x < self.2 && y >= self.1 && y < self.3
    }
}
pub(super) struct Card {
    pub(super) row: OrganizationRow,
    pub(super) own_status: AgentStatus,
    pub(super) status_label: String,
    pub(super) bounds: Bounds,
}
pub(super) struct Scene {
    pub(super) cards: Vec<Card>,
    pub(super) reserve_keys: Vec<String>,
    pub(super) reserve_open: bool,
    pub(super) toggle: Bounds,
    pub(super) surface: PaneSurfaceFrame,
    pub(super) selected_pane: String,
    pub(super) selected_label: String,
    pub(super) height: u32,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Hit {
    ToggleReserve,
    Select { key: String, pane_id: String },
}
impl Scene {
    pub(super) fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if self.toggle.contains(x, y) && !self.reserve_keys.is_empty() {
            return Some(Hit::ToggleReserve);
        }
        // Children are painted inside parents; their selection takes precedence.
        self.cards.iter().rev().find_map(|card| {
            card.bounds
                .contains(x, y)
                .then(|| {
                    card.row.pane_id.as_ref().map(|pane| Hit::Select {
                        key: card.row.key.clone(),
                        pane_id: pane.clone(),
                    })
                })
                .flatten()
        })
    }
}

pub(super) fn project(
    snapshot: &ClientShellSnapshot,
    config: &OrganizationConfig,
    surface: &PaneSurfaceFrame,
    selected_key: &str,
    reserve_open: bool,
) -> Result<Scene, String> {
    let rows = organization::rows(
        snapshot,
        config,
        "local",
        &HashSet::new(),
        Some(selected_key),
    );
    if rows
        .iter()
        .any(|r| r.invalid || r.visual_kind == Kind::Unbound)
    {
        return Err("probe requires valid, bound existing organization rows".into());
    }
    let selected = rows
        .iter()
        .find(|r| r.selected)
        .ok_or("unknown selection key")?;
    let selected_pane = selected
        .pane_id
        .clone()
        .ok_or("selection has no runtime pane")?;
    if snapshot.boot_id != surface.boot_id
        || snapshot.revision != surface.projection_revision
        || snapshot.focused_pane_id.as_deref() != Some(selected_pane.as_str())
        || !surface
            .panes
            .iter()
            .any(|p| p.focused && p.pane_id == selected_pane)
    {
        return Err("selection/snapshot/surface identity mismatch".into());
    }
    if surface.frame.cells.len()
        != usize::from(surface.frame.width) * usize::from(surface.frame.height)
    {
        return Err("surface cell geometry mismatch".into());
    }
    let reserved = |r: &OrganizationRow| {
        config
            .agents
            .iter()
            .any(|entry| entry.reserved && entry.pane_id.is_some() && entry.pane_id == r.pane_id)
    };
    let reserve_keys: Vec<_> = rows
        .iter()
        .filter(|r| reserved(r))
        .map(|r| r.key.clone())
        .collect();
    let card_for = |row: &OrganizationRow, bounds: Bounds| {
        let own_status = snapshot
            .agents
            .iter()
            .find(|a| Some(a.pane_id.as_str()) == row.pane_id.as_deref())
            .map(|a| a.agent_status)
            .unwrap_or(AgentStatus::Unknown);
        let personal = organization::organization_status_text(own_status, config.locale());
        let aggregate = organization::organization_status_text(row.status, config.locale());
        let status_label = if row.visual_kind == Kind::Worker {
            row.meta.clone()
        } else {
            match config.locale() {
                crate::config::OrganizationLocale::Portuguese => {
                    format!("Estado: {personal}\nEquipe: {aggregate}")
                }
                crate::config::OrganizationLocale::English => {
                    format!("State: {personal}\nTeam: {aggregate}")
                }
            }
        };
        Card {
            row: row.clone(),
            own_status,
            status_label,
            bounds,
        }
    };
    let mut cards = Vec::new();
    let mut placed = HashSet::new();
    let mut y = 76.0;
    for row in rows.iter().filter(|r| r.visual_kind == Kind::Coordinator) {
        if reserved(row) && !reserve_open {
            continue;
        }
        cards.push(card_for(row, Bounds(32.0, y, 428.0, y + 132.0)));
        placed.insert(row.key.clone());
        y += 148.0;
    }
    for row in rows
        .iter()
        .filter(|r| r.visual_kind == Kind::Manager && !reserved(r))
    {
        let children: Vec<_> = rows
            .iter()
            .filter(|c| c.parent_key.as_deref() == Some(row.key.as_str()))
            .collect();
        if children
            .iter()
            .any(|c| c.visual_kind != Kind::Worker || reserved(c))
        {
            return Err("nested/reserved worker layouts are outside this bounded probe".into());
        }
        let height = 120.0 + children.len() as f32 * 114.0;
        cards.push(card_for(row, Bounds(32.0, y, 428.0, y + height)));
        placed.insert(row.key.clone());
        for (i, child) in children.iter().enumerate() {
            let top = y + 108.0 + i as f32 * 114.0;
            cards.push(card_for(child, Bounds(48.0, top, 412.0, top + 102.0)));
            placed.insert(child.key.clone());
        }
        y += height + 16.0;
    }
    for row in rows.iter().filter(|r| r.pane_id.is_some()) {
        if !placed.contains(&row.key) && !(reserved(row) && !reserve_open) {
            return Err("bound identity would be omitted by this probe".into());
        }
    }
    Ok(Scene {
        cards,
        reserve_keys,
        reserve_open,
        toggle: Bounds(333.0, 90.0, 412.0, 124.0),
        surface: surface.clone(),
        selected_pane,
        selected_label: selected.label.clone(),
        height: (y + 54.0).max(800.0) as u32,
    })
}
