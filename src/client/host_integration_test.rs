#![cfg(all(test, windows))]

use super::dispatcher::integrated_client_event_to_presentation as dispatch;
use super::endpoint::{ClientEndpointId, EndpointNegotiation, EndpointRegistry};
use super::host::{forward_native_event, HostMode};
use super::integrated_graphics_test::{
    snapshot, state, surface, NoopPrefixInputSource, RecordingTransport, BOOT, COLS, ROWS,
};
use super::*;
use crate::platform::windows_host::{
    keyboard_modifiers, NativeHostConfig, NativeHostError, NativeHostEvent, NativeHostSession,
    NativeMessageNormalizer,
};
use crate::protocol::{
    ClientInputEvent, ClientKeyCode, ClientKeyKind, ClientMessage, ClientPaneInputEvent,
};
use std::sync::{Arc, Mutex};

// This test deliberately never calls NativeHostSession::start: no HWND, desktop,
// physical keyboard, clipboard, server or PTY is exercised here.
#[test]
fn host_integration() {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::path::PathBuf::from(format!(
        "F:/argos-build/host-preparation-20260912/test-artifacts/run-{run}"
    ));
    std::fs::create_dir_all(&root).unwrap();
    let prepared = NativeHostSession::new(NativeHostConfig {
        title: "Windowless host contract".into(),
        width: 1060,
        height: 1200,
    })
    .unwrap();
    assert!(prepared.hwnd().is_none());
    assert!(super::integrated_graphics::IntegratedGraphicsSink::new_hwnd(0).is_err());
    assert_eq!(super::host::native_grid_size(1272, 704).unwrap(), (106, 32));
    assert!(NativeHostSession::new(NativeHostConfig {
        title: String::new(),
        width: 0,
        height: 0
    })
    .is_err());

    // Same bit decoder as production GetKeyState capture; low toggle bits are not down.
    let ctrl = keyboard_modifiers(0, i16::MIN, 0, 0, 0);
    assert_eq!(ctrl, 2);
    assert_eq!(keyboard_modifiers(1, 1, 1, 1, 1), 0);
    assert_eq!(
        keyboard_modifiers(i16::MIN, i16::MIN, i16::MIN, 0, i16::MIN),
        15
    );
    let mut normalizer = NativeMessageNormalizer::default();
    let raw_repeat = 3isize | (0x2eisize << 16) | (1isize << 30);
    let repeated = normalizer
        .key_message(0x43, raw_repeat, true, ctrl)
        .unwrap();
    assert!(matches!(
        &repeated[..],
        [ClientInputEvent::Key {
            code: ClientKeyCode::Char('c'),
            modifiers: 2,
            kind: ClientKeyKind::Repeat,
            repeat_count: 3,
            ..
        }]
    ));
    assert!(
        normalizer
            .wm_char_message(3, raw_repeat, ctrl)
            .unwrap()
            .is_empty(),
        "Ctrl+C WM_CHAR must not duplicate the key"
    );
    assert!(
        normalizer.key_message(0x10, 1, true, 1).unwrap().is_empty(),
        "modifier alone is not an unsupported key"
    );
    let mut unicode = NativeMessageNormalizer::default();
    assert!(unicode.wm_char_message(0xd83d, 1, 0).unwrap().is_empty());
    let pair = unicode.wm_char_message(0xde80, 1, 0).unwrap();
    assert!(matches!(
        &pair[..],
        [ClientInputEvent::Key {
            code: ClientKeyCode::Char('🚀'),
            ..
        }]
    ));
    assert!(unicode.wm_char_message(0xdc00, 1, 0).is_err());

    let mut printable = NativeMessageNormalizer::default();
    assert!(printable
        .key_message(0x41, raw_repeat, true, 0)
        .unwrap()
        .is_empty());
    let text_repeat = printable
        .wm_char_message('a' as u16, raw_repeat, 0)
        .unwrap();
    assert!(matches!(
        &text_repeat[..],
        [ClientInputEvent::Key {
            code: ClientKeyCode::Char('a'),
            kind: ClientKeyKind::Repeat,
            repeat_count: 3,
            ..
        }]
    ));
    for vk in [0x0d, 0x08, 0x09] {
        let mut controls = NativeMessageNormalizer::default();
        assert_eq!(controls.key_message(vk, 1, true, 0).unwrap().len(), 1);
        assert!(
            controls
                .wm_char_message(vk as u16, 1, 0)
                .unwrap()
                .is_empty(),
            "control WM_CHAR must not duplicate the key"
        );
    }

    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut state = state(&root, frames.clone());
    state.set_host_mode(HostMode::Win32Experimental);
    assert!(!state.host_capabilities().ansi_output);
    assert!(HostMode::Ansi.capabilities().ansi_output);
    let initial = state.shell.as_mut().unwrap().compose(COLS, ROWS).unwrap();
    state.present_frame(initial.clone());
    assert!(
        !state.blit_encoder.is_current(&initial),
        "native presentation must not commit ANSI output"
    );
    assert!(state.take_integrated_presentation_error().is_none());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut endpoints = EndpointRegistry::new(
        RecordingTransport(sent.clone()),
        1,
        EndpointNegotiation::default(),
    );
    let mut pending = None;
    let mut commands = endpoint_commands::EndpointCommands::default();
    let mut prefix = NoopPrefixInputSource::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let mut delivered = Vec::new();
    macro_rules! route {
        ($events:expr) => {{
            let events = $events;
            delivered.extend(events.clone());
            forward_native_event(NativeHostEvent::Input(events), &tx).unwrap();
            let ClientLoopEvent::StdinEvents(events) = rx.try_recv().unwrap() else {
                panic!("same client input channel");
            };
            assert!(!dispatch(
                &mut state,
                &events,
                &mut endpoints,
                &mut pending,
                &mut commands,
                &mut prefix,
                &tx
            )
            .unwrap());
            assert!(state.take_integrated_presentation_error().is_none());
        }};
    }
    let mut keys = NativeMessageNormalizer::default();
    route!(keys
        .key_message(0x42, 1 | (0x30 << 16), true, ctrl)
        .unwrap());
    route!(keys.wm_char_message(2, 1, ctrl).unwrap());
    // SHIFT + OEM_2 -> '?' from WM_CHAR must reach the common prefix/help binding.
    route!(keys.key_message(0xbf, 1 | (0x35 << 16), true, 1).unwrap());
    route!(keys
        .wm_char_message(
            '?' as u16,
            1 | (0x35 << 16),
            keyboard_modifiers(i16::MIN, 0, 0, 0, 0)
        )
        .unwrap());
    assert_eq!(frames.lock().unwrap().last().unwrap()["phase"], "open");
    let pane_input_count = |messages: &[ClientMessage]| {
        messages
            .iter()
            .filter(|m| matches!(m, ClientMessage::ClientShellPaneInput { .. }))
            .count()
    };
    let before = pane_input_count(&sent.lock().unwrap());
    route!(keys.key_message(0x58, 1 | (0x2d << 16), true, 0).unwrap());
    route!(keys
        .wm_char_message('x' as u16, 1 | (0x2d << 16), 0)
        .unwrap());
    assert_eq!(
        before,
        pane_input_count(&sent.lock().unwrap()),
        "overlay consumes x before endpoint"
    );
    route!(keys.key_message(0x1b, 1, true, 0).unwrap());
    assert_eq!(frames.lock().unwrap().last().unwrap()["phase"], "closed");
    route!(keys.key_message(0x42, 1, true, ctrl).unwrap());
    route!(keys.wm_char_message(2, 1, ctrl).unwrap());
    route!(keys.key_message(0x4c, 1, true, 0).unwrap());
    route!(keys.wm_char_message('l' as u16, 1, 0).unwrap());
    let selection = sent
        .lock()
        .unwrap()
        .iter()
        .find_map(|message| {
            if let ClientMessage::ClientShellEndpointRequest { boot_id, request } = message {
                let parsed: crate::api::schema::Request = serde_json::from_str(request).unwrap();
                if let crate::api::schema::Method::PaneFocusDirection(params) = &parsed.method {
                    assert_eq!(boot_id, BOOT);
                    assert_eq!(params.pane_id.as_deref(), Some("pane-1"));
                    assert_eq!(params.direction, crate::api::schema::PaneDirection::Right);
                    return Some(parsed);
                }
            }
            None
        })
        .expect("normalized keys produce existing focus request");
    assert!(commands.accepts_response(&ClientEndpointId::Local, 1, BOOT, &selection.id));
    let size = state.shell.as_ref().unwrap().surface_size(COLS, ROWS);
    let previous = surface(size.cols, size.rows, 7, "pane-1");
    let panes: Vec<_> = previous
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rect = ratatui::layout::Rect::new(p.rect.x, p.rect.y, p.rect.width, p.rect.height);
            crate::layout::PaneInfo {
                id: crate::layout::PaneId::from_raw(i as u32 + 1),
                rect,
                inner_rect: rect,
                scrollbar_rect: None,
                borders: ratatui::widgets::Borders::NONE,
                is_focused: p.focused,
            }
        })
        .collect();
    let source = panes.iter().find(|p| p.is_focused).unwrap();
    let target =
        crate::layout::find_in_direction(source, crate::layout::NavDirection::Right, &panes)
            .unwrap();
    let target_index = panes.iter().position(|p| p.id == target).unwrap();
    let target_pane = &previous.panes[target_index].pane_id;
    assert_eq!(target_pane, "pane-2");
    state
        .shell
        .as_mut()
        .unwrap()
        .set_pane_surface(surface(size.cols, size.rows, 8, target_pane));
    install_client_shell_snapshot(
        &mut state,
        &ClientEndpointId::Local,
        Box::new(snapshot(8, target_pane)),
        false,
        &mut endpoints,
        &mut prefix,
    )
    .unwrap();
    const PASTE: &str = "HOST-PASTE ação 🚀\nsegunda linha";
    route!(NativeMessageNormalizer::paste(PASTE).unwrap());
    let messages = sent.lock().unwrap();
    assert!(messages.iter().any(|m| matches!(m, ClientMessage::ClientShellPaneInput { pane_id, events } if pane_id == target_pane && events.iter().any(|e| matches!(e, ClientPaneInputEvent::Paste(text) if text == PASTE)))));
    assert!(!messages.iter().any(|m| matches!(m, ClientMessage::ClientShellPaneInput { pane_id, events } if pane_id != target_pane && events.iter().any(|e| matches!(e, ClientPaneInputEvent::Paste(_))))));
    drop(messages);
    forward_native_event(
        NativeHostEvent::Resize {
            width: 1060,
            height: 1200,
        },
        &tx,
    )
    .unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientLoopEvent::NativeHostResize {
            width: 1060,
            height: 1200
        }
    ));
    forward_native_event(NativeHostEvent::Focus { focused: false }, &tx).unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientLoopEvent::NativeHostFocus { focused: false }
    ));
    forward_native_event(NativeHostEvent::Close, &tx).unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientLoopEvent::NativeHostClose
    ));
    let error = NativeHostError::Platform("injected host failure".into());
    assert_eq!(
        forward_native_event(NativeHostEvent::Error(error.clone()), &tx),
        Err(error)
    );
    assert!(
        matches!(rx.try_recv().unwrap(), ClientLoopEvent::NativeHostError(message)
        if message.contains("injected host failure")),
        "host failure must wake the client loop"
    );
    drop(rx);
    assert!(
        forward_native_event(NativeHostEvent::Close, &tx).is_err(),
        "closed client channel must not silently discard event"
    );
    let receipt = serde_json::json!({
        "test": "host_integration", "window_created": false, "lifecycle_executed": false,
        "production_input": false, "clipboard_read": false, "endpoint_fixture": true,
        "physical_keyboard_proven": false, "wndproc_delivery_proven": false,
        "raw_lparam": raw_repeat, "repeat_count": 3, "toggle_bits_ignored": true,
        "events": delivered, "selection_request": selection, "selected_pane": target_pane,
        "successor": "controlled fixture uses common layout navigation and snapshot installer; no server execution",
        "paste": PASTE, "frames": &*frames.lock().unwrap(), "host_error_propagated": true,
        "closed_channel_rejected": true, "ansi_encoder_not_committed": true
    });
    std::fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    println!("HOST_INTEGRATION_ARTIFACTS={}", root.display());
}
