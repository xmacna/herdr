#![cfg(all(test, windows))]

use super::*;
use crate::client::dispatcher::integrated_client_event_to_presentation as dispatch;
use crate::client::endpoint::{EndpointNegotiation, EndpointRegistry, EndpointTransport};
use crate::config::OrganizationAgentConfig;
use crate::protocol::*;
use crossterm::event::KeyModifiers;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

const COLS: u16 = 106;
const ROWS: u16 = 60;
const BOOT: &str = "integrated-fixture-boot";
const PASTE: &str = "P1-INTEGRATED-PASTE";

#[derive(Clone)]
struct RecordingTransport(Arc<Mutex<Vec<ClientMessage>>>);
impl EndpointTransport for RecordingTransport {
    fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}
#[derive(Default)]
struct NoopPrefixInputSource {
    switched: usize,
    restored: usize,
}
impl crate::platform::PrefixInputSource for NoopPrefixInputSource {
    fn switch_to_ascii(&mut self) {
        self.switched += 1;
    }
    fn restore(&mut self) {
        self.restored += 1;
    }
}

fn png_pixels(path: &Path) -> (u32, u32, usize, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    (info.width, info.height, info.color_type.samples(), bytes)
}

struct EvidenceSink {
    root: std::path::PathBuf,
    states: Arc<Mutex<Vec<serde_json::Value>>>,
}
impl EvidenceSink {
    fn new(root: &Path, states: Arc<Mutex<Vec<serde_json::Value>>>) -> Self {
        Self {
            root: root.to_owned(),
            states,
        }
    }
}
impl IntegratedPresentationSink for EvidenceSink {
    fn present(
        &mut self,
        update: IntegratedPresentationUpdate<'_>,
    ) -> Result<(), IntegratedPresentationError> {
        if let IntegratedPresentationUpdate::Full(frame) = update {
            // Recording wraps the REAL sink; the dispatcher/finalizer calls this destination.
            let mut states = self.states.lock().unwrap();
            let index = states.len();
            let phase = if frame.overlay_occlusions.is_empty() {
                "closed"
            } else {
                "open"
            };
            let path = self.root.join(format!("{index:02}-overlay-{phase}.png"));
            let mut native = IntegratedGraphicsSink::new(&path);
            native.present(IntegratedPresentationUpdate::Full(frame))?;
            // Compare real raster pixels inside the compositor's occlusions with the same
            // complete frame rendered without semantic cards. This is not a mock image.
            let mut protected_pixels = 0usize;
            if !frame.overlay_occlusions.is_empty() {
                let reference_path = self.root.join(format!("{index:02}-overlay-reference.png"));
                let reference = IntegratedPresentationFrame {
                    identity: frame.identity.clone(),
                    organization: Vec::new(),
                    organization_hits: Vec::new(),
                    overlay_occlusions: frame.overlay_occlusions.clone(),
                    office_operations_allowed: false,
                    frame: frame.frame.clone(),
                };
                IntegratedGraphicsSink::new(&reference_path)
                    .present(IntegratedPresentationUpdate::Full(&reference))?;
                let (width, height, channels, pixels) = png_pixels(&path);
                let (rw, rh, rc, expected) = png_pixels(&reference_path);
                assert_eq!((width, height, channels), (rw, rh, rc));
                let cw = width / u32::from(frame.frame.width);
                let ch = height / u32::from(frame.frame.height);
                for r in &frame.overlay_occlusions {
                    for y in u32::from(r.y) * ch..u32::from(r.bottom()) * ch {
                        let start = (y * width + u32::from(r.x) * cw) as usize * channels;
                        let end = (y * width + u32::from(r.right()) * cw) as usize * channels;
                        assert_eq!(
                            &pixels[start..end],
                            &expected[start..end],
                            "semantic cards must not paint over the composed overlay"
                        );
                        protected_pixels += (end - start) / channels;
                    }
                }
                assert!(protected_pixels > 0);
            }
            let mut keys = std::collections::HashSet::new();
            assert_eq!(frame.organization.len(), frame.organization_hits.len());
            let operations = frame.organization.iter().map(|operation| {
                let node = match operation { IntegratedOfficeOperation::Group(n) | IntegratedOfficeOperation::Person(n) => n };
                assert!(keys.insert(node.key.clone()), "duplicate organization key");
                assert_eq!(frame.organization_hits.iter().filter(|hit| hit.key == node.key).count(), 1);
                let hit = frame.organization_hits.iter().find(|hit| hit.key == node.key).expect("semantic hit");
                assert_eq!(hit.rect, node.rect);
                assert_eq!(hit.pane_id, node.pane_id);
                assert!(node.rect.right() <= frame.frame.width && node.rect.bottom() <= frame.frame.height);
                serde_json::json!({"key":node.key,"parent":node.parent_key,"pane":node.pane_id,"kind":format!("{:?}",node.kind),"name":node.name,"focusable":node.focusable,"rect":[node.rect.x,node.rect.y,node.rect.width,node.rect.height],"box_start":node.box_start,"box_end":node.box_end})
            }).collect::<Vec<_>>();
            states.push(serde_json::json!({"protected_pixels":protected_pixels,"phase":phase,"image":path.file_name().unwrap().to_string_lossy(),"allowed":frame.office_operations_allowed,"occlusions":frame.overlay_occlusions.iter().map(|r|[r.x,r.y,r.width,r.height]).collect::<Vec<_>>(),"operations":operations,"endpoint":frame.identity.endpoint_id,"generation":frame.identity.generation,"boot":frame.identity.boot_id,"projection":frame.identity.projection_revision,"surface":frame.identity.surface_revision}));
        }
        Ok(())
    }
}

fn snapshot(revision: u64, focus: &str) -> ClientShellSnapshot {
    let people = [
        ("pane-1", "gerente", "tab-1"),
        ("pane-2", "worker", "tab-1"),
        ("pane-3", "argos", "tab-2"),
        ("pane-4", "reserva", "tab-2"),
    ];
    ClientShellSnapshot {
        boot_id: BOOT.into(),
        revision,
        config_diagnostic: None,
        product_announcement: None,
        update_available: None,
        update_install_command: "herdr update".into(),
        server_keybindings_toml: None,
        latest_release_notes_available: false,
        integration_updates_available: false,
        worktree_directory: "F:/synthetic-fixture".into(),
        release_notes: None,
        focused_workspace_id: Some("ws-1".into()),
        focused_tab_id: Some("tab-1".into()),
        focused_pane_id: Some(focus.into()),
        tab_bar_right: vec![],
        tab_bar_right_separator: " ".into(),
        agent_view_label: None,
        agent_order: vec![
            "pane-3".into(),
            "pane-4".into(),
            "pane-1".into(),
            "pane-2".into(),
        ],
        workspaces: vec![ClientShellWorkspace {
            workspace_id: "ws-1".into(),
            active_tab_id: "tab-1".into(),
            new_workspace_cwd: "F:/synthetic-fixture".into(),
            number: 1,
            label: "Escritório de teste".into(),
            custom_label: true,
            branch: Some("fixture".into()),
            git_ahead_behind: None,
            tokens: vec![],
            worktree: None,
            focused: true,
            agent_status: crate::api::schema::AgentStatus::Working,
        }],
        tabs: [("tab-1", "Trabalho", true), ("tab-2", "Coordenação", false)]
            .into_iter()
            .enumerate()
            .map(|(i, (id, label, focused))| ClientShellTab {
                tab_id: id.into(),
                workspace_id: "ws-1".into(),
                number: i + 1,
                label: label.into(),
                custom_label: true,
                zoomed: false,
                focused,
                agent_status: crate::api::schema::AgentStatus::Working,
            })
            .collect(),
        panes: people
            .iter()
            .map(|(pane, _, tab)| ClientShellPane {
                pane_id: (*pane).into(),
                workspace_id: "ws-1".into(),
                tab_id: (*tab).into(),
                label: None,
                cwd: Some("F:/synthetic-fixture".into()),
                foreground_cwd: None,
                focused: *pane == focus,
                right_click_passthrough: false,
            })
            .collect(),
        agents: people
            .iter()
            .enumerate()
            .map(|(i, (pane, name, tab))| ClientShellAgent {
                pane_id: (*pane).into(),
                workspace_id: "ws-1".into(),
                tab_id: (*tab).into(),
                name: Some((*name).into()),
                display_agent: None,
                agent: Some("codex".into()),
                title: None,
                terminal_title: None,
                terminal_title_stripped: None,
                agent_status: if *name == "reserva" {
                    crate::api::schema::AgentStatus::Idle
                } else {
                    crate::api::schema::AgentStatus::Working
                },
                state_change_seq: i as u64 + 1,
                state_labels: vec![],
                tokens: vec![],
                focused: *pane == focus,
            })
            .collect(),
        commands: vec![],
    }
}
fn organization_config() -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.ui.organization.enabled = true;
    config.ui.organization.agents = [
        ("argos", "Argos", "pane-3", "coordinator", None, false),
        (
            "reserva",
            "Reserva",
            "pane-4",
            "coordinator",
            None, // Existing contract: reserved coordinator belongs to the common Argos group.
            true,
        ),
        (
            "gerente",
            "Produto",
            "pane-1",
            "manager",
            Some("argos"),
            false,
        ),
        (
            "worker",
            "Pesquisa",
            "pane-2",
            "worker",
            Some("gerente"),
            false,
        ),
    ]
    .into_iter()
    .map(
        |(id, label, pane, role, parent, reserved)| OrganizationAgentConfig {
            id: id.into(),
            label: label.into(),
            display_name: Some(label.into()),
            name: Some(id.into()),
            pane_id: Some(pane.into()),
            endpoint_id: Some("local".into()),
            parent_id: parent.map(str::to_owned),
            role: role.into(),
            owner_id: None,
            area: None,
            task: None,
            function: Some("Ensaio isolado".into()),
            model: None,
            reserved,
        },
    )
    .collect();
    config
}
fn surface(cols: u16, rows: u16, revision: u64, focus: &str) -> PaneSurfaceFrame {
    let left = cols / 2;
    let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
    buffer.set_style(
        buffer.area,
        Style::default()
            .fg(Color::Rgb(225, 231, 239))
            .bg(Color::Rgb(20, 25, 32)),
    );
    for (x, title) in [(1, "Terminal gerente"), (left + 1, "Terminal trabalhador")] {
        buffer.set_string(x, 1, title, Style::default());
        buffer.set_string(x, 3, "PS F:\\fixture>", Style::default());
        buffer.set_string(
            x,
            5,
            "PERGUNTA: continuar?",
            Style::default().fg(Color::Rgb(240, 195, 90)),
        );
        buffer.set_string(
            x,
            7,
            "ERRO: exemplo visível",
            Style::default().fg(Color::Rgb(244, 130, 130)),
        );
    }
    let make_pane = |id: &str, x, width| PaneSurfacePane {
        pane_id: id.into(),
        content_revision: revision,
        rect: SurfaceRect {
            x,
            y: 0,
            width,
            height: rows,
        },
        inner_rect: SurfaceRect {
            x,
            y: 0,
            width,
            height: rows,
        },
        scrollbar_rect: None,
        scroll: None,
        focused: id == focus,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        alternate_screen_active: false,
        pixel_width: 0,
        pixel_height: 0,
    };
    PaneSurfaceFrame {
        boot_id: BOOT.into(),
        projection_revision: revision,
        surface_revision: revision + 2,
        frame: FrameData::from_ratatui_buffer_with_hyperlinks(
            &buffer,
            Some(CursorState {
                x: if focus == "pane-1" { 1 } else { left + 1 },
                y: 9,
                visible: true,
                shape: 2,
            }),
            &[],
        ),
        panes: vec![
            make_pane("pane-1", 0, left),
            make_pane("pane-2", left, cols - left),
        ],
        splits: vec![PaneSurfaceSplit {
            direction: PaneSurfaceSplitDirection::Horizontal,
            pos: left,
            area: SurfaceRect {
                x: 0,
                y: 0,
                width: cols,
                height: rows,
            },
            hit_rect: SurfaceRect {
                x: left,
                y: 0,
                width: 1,
                height: rows,
            },
            path: vec![],
        }],
        popup: None,
        graphics: SurfaceGraphicsScene::default(),
    }
}
fn state(output_root: &std::path::Path, states: Arc<Mutex<Vec<serde_json::Value>>>) -> ClientState {
    let mut shell = crate::client::shell::ClientShellState::new(
        crate::client::shell::ClientShellConfig::from_config(&organization_config()),
    );
    shell.set_endpoint_snapshot_for_generation(
        &endpoint::ClientEndpointId::Local,
        1,
        Box::new(snapshot(7, "pane-1")),
    );
    let size = shell.surface_size(COLS, ROWS);
    shell.set_pane_surface(surface(size.cols, size.rows, 7, "pane-1"));
    let mut state = ClientState {
        blit_encoder: render_ansi::BlitEncoder::new(),
        mouse_capture_active: false,
        endpoint_mouse_capture_requested: false,
        endpoint_sgr_pixels_requested: false,
        host_theme_updates: Vec::new(),
        direct_mouse_capture_preference: false,
        shell_mouse_capture_preference: false,
        direct_keyboard_protocol: Default::default(),
        pane_keyboard_report_all: false,
        keyboard_report_all_active: false,
        reported_size: (COLS, ROWS),
        reported_cell_size: (0, 0),
        sound_config: Default::default(),
        kitty_graphics_enabled: false,
        pixel_geometry_enabled: false,
        pixel_geometry_exact: false,
        attach_escape: None,
        remote_image_paste_key: None,
        redraw_on_focus_gained: false,
        repaint_pending: false,
        presentation_frozen: false,
        draw_host_cursor: false,
        detached_process_children: Vec::new(),
        integrated_presentation_sink: None,
        integrated_presentation_error: None,
        shell: Some(shell),
    };
    state.set_integrated_presentation_sink(Some(Box::new(EvidenceSink::new(output_root, states))));
    state
}

fn key(code: ClientKeyCode, modifiers: u8) -> ClientInputEvent {
    ClientInputEvent::Key {
        code,
        modifiers,
        kind: ClientKeyKind::Press,
        repeat_count: 1,
        generated_text: None,
        source: ClientKeySource::Synthesized,
    }
}
#[test]
fn integrated_client_event_to_presentation() {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::path::PathBuf::from(format!(
        "F:/argos-build/integrated-presentation-20260912/test-artifacts/run-{run}"
    ));
    std::fs::create_dir_all(&root).unwrap();
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut state = state(&root, frames.clone());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut endpoints = EndpointRegistry::new(
        RecordingTransport(sent.clone()),
        1,
        EndpointNegotiation::default(),
    );
    let mut pending = None;
    let mut commands = endpoint_commands::EndpointCommands::default();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(32);
    let mut prefix = NoopPrefixInputSource::default();
    let initial = state.shell.as_mut().unwrap().compose(COLS, ROWS).unwrap();
    // Assert default ANSI composition is identical and emits no semantic package.
    let mut ordinary =
        shell::ClientShellState::new(shell::ClientShellConfig::from_config(&organization_config()));
    ordinary.set_endpoint_snapshot_for_generation(
        &endpoint::ClientEndpointId::Local,
        1,
        Box::new(snapshot(7, "pane-1")),
    );
    let size = ordinary.surface_size(COLS, ROWS);
    ordinary.set_pane_surface(surface(size.cols, size.rows, 7, "pane-1"));
    let ansi = ordinary.compose(COLS, ROWS).unwrap();
    assert_eq!(initial, ansi);
    assert!(ordinary.take_integrated_presentation().is_none());
    state.present_frame(initial);
    assert!(state.take_integrated_presentation_error().is_none());

    let ctrl = KeyModifiers::CONTROL.bits();
    for event in [
        key(ClientKeyCode::Char('b'), ctrl),
        key(ClientKeyCode::Char('?'), 0),
    ] {
        assert!(!dispatch(
            &mut state,
            &[event],
            &mut endpoints,
            &mut pending,
            &mut commands,
            &mut prefix,
            &event_tx
        )
        .unwrap());
    }
    let open = frames.lock().unwrap().last().unwrap().clone();
    assert_eq!(open["phase"], "open");
    assert_eq!(
        open["allowed"], true,
        "Help must exercise real occlusion, not whole-office fallback"
    );
    let before = sent
        .lock()
        .unwrap()
        .iter()
        .filter(|m| matches!(m, ClientMessage::ClientShellPaneInput { .. }))
        .count();
    assert!(!dispatch(
        &mut state,
        &[key(ClientKeyCode::Char('x'), 0)],
        &mut endpoints,
        &mut pending,
        &mut commands,
        &mut prefix,
        &event_tx
    )
    .unwrap());
    assert_eq!(
        before,
        sent.lock()
            .unwrap()
            .iter()
            .filter(|m| matches!(m, ClientMessage::ClientShellPaneInput { .. }))
            .count()
    );
    assert!(!dispatch(
        &mut state,
        &[key(ClientKeyCode::Esc, 0)],
        &mut endpoints,
        &mut pending,
        &mut commands,
        &mut prefix,
        &event_tx
    )
    .unwrap());
    assert_eq!(frames.lock().unwrap().last().unwrap()["phase"], "closed");

    // Route the real focus command; the controlled endpoint then publishes its successor.
    assert!(!dispatch(
        &mut state,
        &[
            key(ClientKeyCode::Char('b'), ctrl),
            key(ClientKeyCode::Char('l'), 0)
        ],
        &mut endpoints,
        &mut pending,
        &mut commands,
        &mut prefix,
        &event_tx
    )
    .unwrap());
    let selection = sent
        .lock()
        .unwrap()
        .iter()
        .find_map(|m| match m {
            ClientMessage::ClientShellEndpointRequest { boot_id, request } => {
                let parsed: crate::api::schema::Request = serde_json::from_str(request).unwrap();
                match parsed.method {
                    crate::api::schema::Method::PaneFocusDirection(ref params)
                        if params.direction == crate::api::schema::PaneDirection::Right =>
                    {
                        assert_eq!(params.pane_id.as_deref(), Some("pane-1"));
                        assert_eq!(boot_id, BOOT);
                        Some(request.clone())
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("right-focus effect through EndpointRegistry");
    let request: crate::api::schema::Request = serde_json::from_str(&selection).unwrap();
    assert!(commands.accepts_response(&endpoint::ClientEndpointId::Local, 1, BOOT, &request.id));
    let crate::api::schema::Method::PaneFocusDirection(params) = &request.method else {
        panic!("focus request");
    };
    let previous = surface(size.cols, size.rows, 7, "pane-1");
    // Controlled endpoint derives its next projection from the captured request and the
    // existing directional navigation function, rather than injecting a literal target.
    let panes: Vec<crate::layout::PaneInfo> = previous
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rect = Rect::new(p.rect.x, p.rect.y, p.rect.width, p.rect.height);
            crate::layout::PaneInfo {
                id: crate::layout::PaneId::from_raw(i as u32 + 1),
                rect,
                inner_rect: rect,
                scrollbar_rect: None,
                borders: ratatui::widgets::Borders::NONE,
                is_focused: Some(p.pane_id.as_str()) == params.pane_id.as_deref(),
            }
        })
        .collect();
    let source_index = previous
        .panes
        .iter()
        .position(|p| Some(p.pane_id.as_str()) == params.pane_id.as_deref())
        .unwrap();
    let direction = match params.direction {
        crate::api::schema::PaneDirection::Right => crate::layout::NavDirection::Right,
        _ => panic!("fixture only expects the actual right-focus command"),
    };
    let target = crate::layout::find_in_direction(&panes[source_index], direction, &panes).unwrap();
    let target_index = panes.iter().position(|p| p.id == target).unwrap();
    let target_pane = &previous.panes[target_index].pane_id;
    assert_eq!(target_pane, "pane-2");
    let next = surface(size.cols, size.rows, 8, target_pane);
    assert!(next.panes[0].rect.x + next.panes[0].rect.width <= next.panes[1].rect.x);
    let successor = snapshot(8, target_pane);
    let controlled_projection = serde_json::to_value(&successor).unwrap();
    let successor: ClientShellSnapshot =
        serde_json::from_value(controlled_projection.clone()).unwrap();
    state.shell.as_mut().unwrap().set_pane_surface(next);
    install_client_shell_snapshot(
        &mut state,
        &endpoint::ClientEndpointId::Local,
        Box::new(successor),
        false,
        &mut endpoints,
        &mut prefix,
    )
    .unwrap();
    assert!(state.take_integrated_presentation_error().is_none());
    assert!(!dispatch(
        &mut state,
        &[ClientInputEvent::Paste { text: PASTE.into() }],
        &mut endpoints,
        &mut pending,
        &mut commands,
        &mut prefix,
        &event_tx
    )
    .unwrap());
    let messages = sent.lock().unwrap();
    assert!(messages.iter().any(|m|matches!(m,ClientMessage::ClientShellPaneInput{pane_id,events} if pane_id=="pane-2" && events.iter().any(|e|matches!(e,ClientPaneInputEvent::Paste(text) if text==PASTE)))));
    assert!(!messages.iter().any(|m|matches!(m,ClientMessage::ClientShellPaneInput{pane_id,events} if pane_id!="pane-2" && events.iter().any(|e|matches!(e,ClientPaneInputEvent::Paste(_))))));
    let states = frames.lock().unwrap();
    let first = &states[0];
    let ops = first["operations"].as_array().unwrap();
    let find = |pane: &str| {
        ops.iter()
            .find(|n| n["pane"] == pane)
            .unwrap_or_else(|| panic!("missing identity {pane}: {ops:?}"))
    };
    let manager = find("pane-1");
    let worker = find("pane-2");
    assert_eq!(manager["kind"], "Manager");
    assert_eq!(worker["kind"], "Worker");
    assert_eq!(worker["parent"], manager["key"]);
    assert_eq!(manager["focusable"], true);
    assert_eq!(worker["focusable"], true);
    let argos = find("pane-3");
    let reserve = find("pane-4");
    assert_eq!(argos["kind"], "Coordinator");
    assert_eq!(reserve["kind"], "Coordinator");
    assert_eq!(manager["parent"], argos["key"]);
    assert_eq!(
        reserve["parent"], argos["parent"],
        "reserved coordinator remains inside the common Argos group"
    );
    assert_eq!(manager["box_start"], true);
    assert_eq!(worker["box_end"], true);
    for (i, node) in ops.iter().enumerate() {
        let rect = &node["rect"];
        for other in ops.iter().skip(i + 1) {
            let r = &other["rect"];
            assert!(
                rect[1].as_u64().unwrap() + rect[3].as_u64().unwrap() <= r[1].as_u64().unwrap()
                    || r[1].as_u64().unwrap() + r[3].as_u64().unwrap() <= rect[1].as_u64().unwrap(),
                "semantic rows may not overlap"
            );
        }
    }
    for entry in states.iter() {
        assert_eq!(entry["boot"], BOOT);
        assert_eq!(entry["generation"], 1);
        let image = root.join(entry["image"].as_str().unwrap());
        assert_eq!(&std::fs::read(&image).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    }
    let open_path = root.join(open["image"].as_str().unwrap());
    let native: serde_json::Value =
        serde_json::from_slice(&std::fs::read(open_path.with_extension("json")).unwrap()).unwrap();
    assert!(native["organization_operations_occluded"].as_u64().unwrap() > 0);
    let closed_path = root.join(first["image"].as_str().unwrap());
    let closed_pixels = png_pixels(&closed_path);
    let open_pixels = png_pixels(&open_path);
    assert_eq!(
        (closed_pixels.0, closed_pixels.1, closed_pixels.2),
        (open_pixels.0, open_pixels.1, open_pixels.2)
    );
    assert_ne!(
        closed_pixels.3, open_pixels.3,
        "open and closed overlays must change the real raster"
    );
    assert!(open["protected_pixels"].as_u64().unwrap() > 0);
    let last = states.last().unwrap();
    assert_eq!(last["projection"], 8);
    assert_eq!(last["surface"], 10);
    struct FailingSink;
    impl IntegratedPresentationSink for FailingSink {
        fn present(
            &mut self,
            update: IntegratedPresentationUpdate<'_>,
        ) -> Result<(), IntegratedPresentationError> {
            if matches!(update, IntegratedPresentationUpdate::Full(_)) {
                Err(IntegratedPresentationError::new(
                    "isolated presentation failure",
                ))
            } else {
                Ok(())
            }
        }
    }
    drop(messages);
    state.set_integrated_presentation_sink(Some(Box::new(FailingSink)));
    let failure = dispatch(
        &mut state,
        &[key(ClientKeyCode::Char('b'), ctrl)],
        &mut endpoints,
        &mut pending,
        &mut commands,
        &mut prefix,
        &event_tx,
    )
    .unwrap_err();
    assert!(
        matches!(failure, ClientError::Presentation(ref text) if text == "isolated presentation failure")
    );
    let messages = sent.lock().unwrap();
    let receipt = serde_json::json!({"presentation_error_propagated":true,"test":"integrated_client_event_to_presentation","source":"common Windows dispatcher + finalizer + controlled EndpointRegistry + common compose + native sink","fixture":true,"production_input":false,"selection_request":serde_json::from_str::<serde_json::Value>(&selection).unwrap(),"endpoint_successor_simulated":true,"successor_derivation":"captured request + common layout::find_in_direction + existing snapshot installer; no server/PTY execution","controlled_projection":controlled_projection,"messages":serde_json::to_value(&*messages).unwrap(),"paste":PASTE,"prefix_switches":prefix.switched,"prefix_restores":prefix.restored,"ansi_frame_equal":true,"ansi_no_semantic_package":true,"frames":&*states});
    std::fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    println!(
        "INTEGRATED_PROOF_RECEIPT={}",
        root.join("receipt.json").display()
    );
}
