// Isolated capability experiment: uses the actual native shell and graphics
// transport. No socket, PTY, window, provider transcript, or product redesign.
use super::*;
use crate::protocol::{
    SurfaceGraphicsAsset, SurfaceGraphicsAssetKey, SurfaceGraphicsFormat, SurfaceGraphicsPlacement,
    SurfaceGraphicsScene, SurfaceGraphicsSource,
};

fn fixture() -> (ClientShellState, crate::protocol::PaneSurfaceFrame) {
    let mut config = Config::default();
    config.ui.sidebar_width = 50;
    config.ui.sidebar_min_width = 30;
    config.ui.sidebar_max_width = 65;
    config.ui.organization.enabled = true;
    config.ui.organization.locale = "pt".into();
    config.ui.organization.root = "Prova isolada".into();
    config.ui.organization.agents = vec![crate::config::OrganizationAgentConfig {
        id: "coordinator".into(),
        display_name: Some("Argos".into()),
        role: "coordinator".into(),
        pane_id: Some("pane_1".into()),
        function: Some("Coordenar equipe".into()),
        ..Default::default()
    }];
    let mut projected = snapshot();
    projected.agents = vec![ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some("coordinator".into()),
        display_agent: None,
        agent: None,
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: vec![],
        tokens: vec![],
        focused: true,
    }];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(projected));
    let mut pane_surface = surface();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 140, 48));
    buffer.set_string(
        0,
        0,
        "PROVA NATIVA ISOLADA - sem sessao produtiva",
        Style::default(),
    );
    buffer.set_string(
        0,
        1,
        "Pergunta, erro e aprovacao: texto original preservado",
        Style::default(),
    );
    buffer.set_string(
        0,
        2,
        "Grafico abaixo: capacidade de transporte, nao interface aprovada",
        Style::default(),
    );
    pane_surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    pane_surface.panes[0].rect.width = 140;
    pane_surface.panes[0].rect.height = 48;
    pane_surface.panes[0].inner_rect = pane_surface.panes[0].rect;
    state.set_pane_surface(pane_surface.clone());
    (state, pane_surface)
}

fn scene() -> SurfaceGraphicsScene {
    // A tiny checkerboard, deliberately not a proposal for product appearance.
    let pixels = vec![
        47, 107, 100, 255, 236, 235, 231, 255, 236, 235, 231, 255, 47, 107, 100, 255,
    ];
    let key = SurfaceGraphicsAssetKey {
        source: SurfaceGraphicsSource::PaneLayer {
            pane_id: "pane_1".into(),
            layer_id: "p0-synthetic-image".into(),
        },
        image_width: 2,
        image_height: 2,
        format: SurfaceGraphicsFormat::Rgba,
        data_len: pixels.len() as u64,
        data_fingerprint: 1,
    };
    SurfaceGraphicsScene {
        assets: vec![SurfaceGraphicsAsset {
            key: key.clone(),
            data: pixels,
        }],
        placements: vec![SurfaceGraphicsPlacement {
            asset: key,
            logical_placement_id: 1,
            x: 2,
            y: 5,
            cols: 8,
            rows: 4,
            source_x: 0,
            source_y: 0,
            source_width: 2,
            source_height: 2,
            x_offset: 0,
            y_offset: 0,
            z: 0,
            scrollback_offset: 0,
        }],
        retained_assets: vec![],
    }
}

#[test]
fn p0_native_graphics_preserves_terminal_cells_and_input_without_a_host() {
    let started = std::time::Instant::now();
    let (mut state, mut pane_surface) = fixture();
    let text_only = state.compose(209, 51).expect("native text composition");
    let original_input = state.handle_input_bytes(b"x").requests;
    let input_before = serde_json::to_value(&original_input).unwrap();
    assert!(matches!(original_input.as_slice(),
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"));

    pane_surface.surface_revision += 1;
    pane_surface.graphics = scene();
    state.set_pane_surface(pane_surface);
    let graphical = state
        .compose(209, 51)
        .expect("native graphical composition");
    assert_eq!(
        graphical.cells, text_only.cells,
        "no text/log filtering or replacement"
    );
    assert_eq!(graphical.cursor, text_only.cursor);
    assert_eq!(
        serde_json::to_value(state.handle_input_bytes(b"x").requests).unwrap(),
        input_before
    );
    let commands = String::from_utf8_lossy(&graphical.graphics);
    assert!(commands.contains("a=t,t=d"), "real image transport");
    assert!(commands.contains("a=p"), "real image placement");
    let pane = &state.hits.panes[0];
    let expected_origin = format!("\x1b[{};{}H", pane.inner_rect.y + 6, pane.inner_rect.x + 3);
    assert!(
        commands.contains(&expected_origin),
        "image follows pane, not sidebar chrome"
    );

    let retained = state.compose(209, 51).expect("retained frame");
    assert!(
        !String::from_utf8_lossy(&retained.graphics).contains("a=t,t=d"),
        "no repeated image upload"
    );
    state.overlay = Some(ClientShellOverlay::Onboarding);
    let covered = state.compose(209, 51).expect("overlay hides graphics");
    assert!(String::from_utf8_lossy(&covered.graphics).contains("a=d,d=i"));
    state.overlay = None;
    let restored = state.compose(209, 51).expect("restore original surface");
    assert_eq!(restored.cells, text_only.cells);
    assert!(String::from_utf8_lossy(&restored.graphics).contains("a=p"));

    if let Some(dir) = std::env::var_os("HERDR_P0_CAPTURE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, frame) in [
            ("text-only", &text_only),
            ("graphics", &graphical),
            ("restored", &restored),
        ] {
            std::fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec(frame).unwrap(),
            )
            .unwrap();
            let lines = frame
                .cells
                .chunks(frame.width as usize)
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.symbol.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(dir.join(format!("{name}.txt")), lines).unwrap();
        }
        let proof = serde_json::json!({
            "fixture": "synthetic; actual Rust client shell; no host or socket",
            "geometry": [209, 51],
            "terminal_cells_identical": true,
            "pane_input_identical": true,
            "pane_id": "pane_1",
            "image_origin": "pane surface, not sidebar",
            "graphic_bytes": graphical.graphics.len(),
            "retained_graphic_bytes": retained.graphics.len(),
            "overlay_hides_image": true,
            "host_display_verified": false,
            "simple_pro_implemented": false,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        std::fs::write(
            dir.join("proof.json"),
            serde_json::to_vec_pretty(&proof).unwrap(),
        )
        .unwrap();
    }
}
