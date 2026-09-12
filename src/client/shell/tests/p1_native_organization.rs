//! Synthetic data, real organization projection + pane surface + native API.
use super::super::native_scene_probe::{self as scene, Hit};
use super::*;
use ratatui::style::Color;

fn fixture() -> (Config, ClientShellSnapshot, PaneSurfaceFrame) {
    let mut config = Config::default();
    config.ui.organization = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/p1-native-organization.json"
    )))
    .expect("existing OrganizationConfig contract");
    let mut snap = snapshot();
    let template = snap.panes[0].clone();
    snap.panes = config
        .ui
        .organization
        .agents
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let mut pane = template.clone();
            pane.pane_id = entry.pane_id.clone().unwrap();
            pane.focused = i == 0;
            pane
        })
        .collect();
    snap.agents = config
        .ui
        .organization
        .agents
        .iter()
        .enumerate()
        .map(|(i, entry)| ClientShellAgent {
            pane_id: entry.pane_id.clone().unwrap(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some(entry.id.clone()),
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: match i {
                0 | 3 => AgentStatus::Working,
                4 => AgentStatus::Blocked,
                6 => AgentStatus::Done,
                _ => AgentStatus::Idle,
            },
            state_change_seq: 1,
            state_labels: vec![],
            tokens: vec![],
            focused: i == 0,
        })
        .collect();
    let mut pane_surface = surface();
    let style = Style::default()
        .fg(Color::Rgb(215, 227, 220))
        .bg(Color::Rgb(29, 37, 35));
    let mut buffer = Buffer::empty(Rect::new(0, 0, 78, 18));
    buffer.set_style(buffer.area, style);
    for (y, line) in [
        "FIXTURE DETERMINISTICA - SEM SESSAO VIVA",
        "",
        "Contrato: PaneSurfaceFrame / CellData",
        "Texto de teste; nao e resposta de agente.",
        "",
        "0123456789  ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "Grade monoespacada independente dos cards.",
        "",
        "Nenhum comando foi executado nesta superficie.",
    ]
    .iter()
    .enumerate()
    {
        buffer.set_string(2, y as u16 + 1, *line, style);
    }
    pane_surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &buffer,
        Some(crate::protocol::CursorState {
            x: 2,
            y: 12,
            visible: true,
            shape: 4,
        }),
        &[],
    );
    pane_surface.panes[0].rect = SurfaceRect {
        x: 0,
        y: 0,
        width: 78,
        height: 18,
    };
    pane_surface.panes[0].inner_rect = pane_surface.panes[0].rect;
    (config, snap, pane_surface)
}

fn key_for(snapshot: &ClientShellSnapshot, config: &Config, pane: &str) -> String {
    super::super::organization::rows(
        snapshot,
        &config.ui.organization,
        "local",
        &HashSet::new(),
        None,
    )
    .into_iter()
    .find(|r| r.pane_id.as_deref() == Some(pane))
    .expect("bound existing pane")
    .key
}

#[test]
fn p1_native_organization_preserves_identity_surface_and_reserve() {
    let (config, mut snap, mut surface) = fixture();
    let original_surface = surface.clone();
    let selected = key_for(&snap, &config, "pane_1");
    let reserve_key = key_for(&snap, &config, "pane_2");
    let closed = scene::project(&snap, &config.ui.organization, &surface, &selected, false)
        .expect("project real model");
    assert_eq!(
        closed.surface.as_ref(),
        Some(&original_surface),
        "no terminal text or cell filtering"
    );
    assert_eq!(closed.cards.len(), 6);
    let manager = closed
        .cards
        .iter()
        .find(|c| c.row.pane_id.as_deref() == Some("pane_3"))
        .unwrap();
    assert_eq!(manager.own_status, AgentStatus::Idle);
    assert_ne!(
        manager.row.status, manager.own_status,
        "team aggregate is distinct from personal state"
    );
    assert!(manager.status_label.starts_with("Disponível"));
    assert_eq!(closed.reserve_keys, vec![reserve_key.clone()]);
    assert!(!closed.cards.iter().any(|c| c.row.key == reserve_key));
    assert_eq!(closed.hit(360.0, 108.0), Some(Hit::ToggleReserve));
    for card in closed
        .cards
        .iter()
        .filter(|c| c.row.visual_kind == super::super::organization::OrganizationVisualKind::Worker)
    {
        let parent = closed
            .cards
            .iter()
            .find(|p| Some(p.row.key.as_str()) == card.row.parent_key.as_deref())
            .expect("same model parent");
        assert!(parent.bounds.contains(card.bounds.0, card.bounds.1));
        assert!(parent
            .bounds
            .contains(card.bounds.2 - 1.0, card.bounds.3 - 1.0));
        assert_eq!(
            closed.hit(card.bounds.0 + 20.0, card.bounds.1 + 25.0),
            Some(Hit::Select {
                key: card.row.key.clone(),
                pane_id: card.row.pane_id.clone().unwrap()
            })
        );
    }
    // Feed the projected stable key into the existing ClientShell controller.
    let worker_key = key_for(&snap, &config, "pane_4");
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snap.clone()));
    state.set_pane_surface(surface.clone());
    state.mode = ClientShellMode::Navigate;
    state.navigate_organization_key = Some(worker_key);
    let focus = state.handle_input_bytes(b"\r");
    assert!(focus.actions.iter().any(|action|matches!(action,ClientShellAction::Endpoint {request,..} if matches!(&request.method,crate::api::schema::Method::PaneFocus(target) if target.pane_id=="pane_4"))));
    // Simulated projection from the same endpoint, never sent to a live pane.
    snap.focused_pane_id = Some("pane_2".into());
    for pane in &mut snap.panes {
        pane.focused = pane.pane_id == "pane_2";
    }
    for agent in &mut snap.agents {
        agent.focused = agent.pane_id == "pane_2";
    }
    surface.panes[0].pane_id = "pane_2".into();
    let opened = scene::project(&snap, &config.ui.organization, &surface, &reserve_key, true)
        .expect("same reserved identity");
    assert_eq!(opened.cards.len(), 7);
    assert_eq!(opened.selected_pane, "pane_2");
    assert_eq!(opened.reserve_keys, closed.reserve_keys);
    assert!(opened
        .cards
        .iter()
        .any(|c| c.row.key == reserve_key && c.row.selected));
    assert_eq!(opened.frame, original_surface.frame);
    assert!(closed
        .cards
        .iter()
        .find(|c| c.row.pane_id.as_deref() == Some("pane_5"))
        .unwrap()
        .row
        .meta
        .contains("atenção"));
    assert!(
        closed
            .cards
            .iter()
            .any(|c| c.row.pane_id.as_deref() == Some("pane_7")),
        "done identity remains present"
    );
    if let Some(dir) = std::env::var_os("HERDR_P1_CAPTURE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        assert!(dir.is_absolute());
        std::fs::create_dir_all(&dir).unwrap();
        for (name, projected) in [("organization", &closed), ("organization-reserve", &opened)] {
            scene::paint::render(projected, &dir.join(format!("{name}.png")))
                .expect("real Direct2D/WIC renderer");
            let mapping:Vec<_>=projected.cards.iter().map(|c|serde_json::json!({"key":c.row.key,"parent_key":c.row.parent_key,"pane_id":c.row.pane_id,"label":c.row.label,"team_status":c.row.status,"own_status":c.own_status,"status_label":c.status_label,"selected":c.row.selected,"bounds":c.bounds})).collect();
            std::fs::write(dir.join(format!("{name}-mapping.json")),serde_json::to_vec_pretty(&serde_json::json!({"origin":"deterministic fixture, existing OrganizationConfig/rows/PaneSurfaceFrame", "live_snapshot":false,"cards":mapping,"reserve_keys":projected.reserve_keys,"selected_pane":projected.selected_pane,"surface":projected.surface,"window_input":false,"focus_request_sent":false})).unwrap()).unwrap();
        }
        std::fs::write(dir.join("proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"identity_mapping":true,"worker_containment":true,"reserve_same_identity":true,"surface_cells_preserved":true,"existing_pane_focus_action":"pane_4","action_sent":false,"native_png":true,"live_data":false,"m0_complete":false,"p1_complete":false})).unwrap()).unwrap();
    }
}

#[test]
fn p1_native_organization_rejects_mismatched_surface_and_invalid_parent() {
    let (mut config, snap, mut surface) = fixture();
    let key = key_for(&snap, &config, "pane_1");
    surface.boot_id = "wrong-endpoint".into();
    assert!(scene::project(&snap, &config.ui.organization, &surface, &key, false).is_err());
    surface.boot_id = snap.boot_id.clone();
    config.ui.organization.agents[3].parent_id = Some("missing-manager".into());
    assert!(scene::project(&snap, &config.ui.organization, &surface, &key, false).is_err());
}
