//! Explicit opt-in finite observation using the existing endpoint and terminal codecs.
//! No raw terminal bytes, transcripts, cwd or token metadata are exported.
use super::*;
use crate::api::{
    client::{ApiClient, ConnectionTarget},
    schema::{EmptyParams, Method, Request, ResponseResult, SessionSnapshot},
};
use crate::ipc::{LocalStream, LocalStreamReadCount};
use crate::protocol::{self, endpoint::*, ClientMessage, ServerMessage, TerminalFrame};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;

// Retain read_exact progress across nonblocking polls; a timeout closes only our reader.
struct FiniteReader<'a> {
    stream: &'a mut LocalStream,
    deadline: Instant,
}
impl Read for FiniteReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "finite readonly observation expired",
                ));
            }
            match crate::ipc::poll_local_stream_read_count(self.stream, buffer)? {
                LocalStreamReadCount::Data(n) => return Ok(n),
                LocalStreamReadCount::Closed => return Ok(0),
                LocalStreamReadCount::Pending => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}
fn read_message(stream: &mut LocalStream) -> ProbeResult<ServerMessage> {
    // Windows poll_local_stream_read_count already uses PeekNamedPipe before
    // ReadFile. Keep the original blocking pipe mode; changing it while the
    // server writes can fail with ERROR_PIPE_BUSY (231).
    Ok(protocol::read_message(
        &mut FiniteReader {
            stream,
            deadline: Instant::now() + Duration::from_secs(8),
        },
        protocol::MAX_GRAPHICS_FRAME_SIZE,
    )?)
}
fn send_observation(stream: &mut LocalStream, message: &ClientMessage) -> ProbeResult<()> {
    // An executable allowlist: never accept arbitrary endpoint requests or input.
    let allowed = matches!(
        message,
        ClientMessage::TerminalHello { .. } | ClientMessage::ObserveTerminal { .. }
    ) || matches!(message, ClientMessage::EndpointControl {kind, data} if kind==ENDPOINT_HELLO_KIND && serde_json::from_str::<EndpointClientHello>(data).is_ok_and(|h| !h.surface_active && !h.direct_graphics && !h.mouse_capture));
    if !allowed {
        return Err("mutation outside readonly probe allowlist".into());
    }
    protocol::write_message(stream, message)
        .map_err(|e| format!("observer protocol write: {e}"))?;
    Ok(())
}
fn connect_observer(socket: &Path) -> ProbeResult<LocalStream> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match crate::ipc::connect_local_stream(socket) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(format!("observer connect: {error}").into()),
        }
    }
}
fn endpoint_snapshot(socket: &Path) -> ProbeResult<ClientShellSnapshot> {
    let mut stream = connect_observer(socket)?;
    let hello = EndpointClientHello {
        generation: ENDPOINT_PROTOCOL_GENERATION,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: protocol::ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    send_observation(
        &mut stream,
        &ClientMessage::EndpointControl {
            kind: ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello)?,
        },
    )?;
    match read_message(&mut stream)? {
        ServerMessage::EndpointControl { kind, data } if kind == ENDPOINT_WELCOME_KIND => {
            let welcome: EndpointServerWelcome = serde_json::from_str(&data)?;
            if welcome.error.is_some()
                || welcome.generation != ENDPOINT_PROTOCOL_GENERATION
                || welcome.snapshot_codec != SNAPSHOT_CODEC_V1
            {
                return Err("incompatible endpoint welcome".into());
            }
        }
        _ => return Err("expected stable endpoint welcome".into()),
    }
    match read_message(&mut stream)? {
        ServerMessage::EndpointControl { kind, data } if kind == ENDPOINT_SNAPSHOT_KIND => {
            Ok(serde_json::from_str(&data)?)
        }
        _ => Err("expected existing ClientShellSnapshot codec".into()),
    }
    // Drop only this inactive connection. No Detach/input/resize request is emitted.
}
fn terminal_frame(
    socket: &Path,
    terminal: &str,
    cols: u16,
    rows: u16,
) -> ProbeResult<TerminalFrame> {
    let mut stream = connect_observer(socket)?;
    send_observation(
        &mut stream,
        &ClientMessage::TerminalHello {
            version: protocol::PROTOCOL_VERSION,
            cols,
            rows,
            cell_width_px: 0,
            cell_height_px: 0,
            pixel_mouse: false,
        },
    )?;
    match read_message(&mut stream)? {
        ServerMessage::Welcome {
            version,
            encoding: protocol::RenderEncoding::TerminalAnsi,
            error: None,
        } if version == protocol::PROTOCOL_VERSION => {}
        _ => return Err("terminal observe protocol mismatch".into()),
    }
    send_observation(
        &mut stream,
        &ClientMessage::ObserveTerminal {
            target: terminal.into(),
        },
    )?;
    for _ in 0..8 {
        match read_message(&mut stream)? {
            ServerMessage::Terminal(frame) if frame.full => return Ok(frame),
            ServerMessage::ServerShutdown { .. } => {
                return Err("observation rejected or terminal unavailable".into())
            }
            _ => {}
        }
    }
    Err("no full terminal frame in bounded observation".into())
}
fn session_snapshot(api: &ApiClient) -> ProbeResult<SessionSnapshot> {
    match api
        .request(Request {
            id: "p1:readonly:snapshot".into(),
            method: Method::SessionSnapshot(EmptyParams {}),
        })?
        .result
    {
        ResponseResult::SessionSnapshot { snapshot } => Ok(*snapshot),
        _ => Err("unexpected session.snapshot response".into()),
    }
}
fn absolute_env(name: &str) -> ProbeResult<PathBuf> {
    let value = PathBuf::from(std::env::var_os(name).ok_or(format!("explicit {name} required"))?);
    if !value.is_absolute() {
        return Err(format!("{name} must be absolute").into());
    }
    Ok(value)
}
fn identity_map(s: &SessionSnapshot) -> std::collections::BTreeMap<String, String> {
    s.panes
        .iter()
        .map(|p| (p.pane_id.clone(), p.terminal_id.clone()))
        .collect()
}

#[test]
#[ignore = "requires explicit live socket/config/target/output; readonly, no window"]
fn p1_live_readonly_existing_endpoint_and_terminal_frame() -> ProbeResult<()> {
    let socket = absolute_env("HERDR_P1_CLIENT_SOCKET")?;
    let api_path = absolute_env("HERDR_P1_API_SOCKET")?;
    let config_path = absolute_env("HERDR_P1_LIVE_CONFIG")?;
    let output = absolute_env("HERDR_P1_LIVE_OUTPUT")?;
    let target = std::env::var("HERDR_P1_LIVE_PANE")?;
    let config_bytes = std::fs::read(&config_path)?;
    let config: Config = toml::from_str(std::str::from_utf8(&config_bytes)?)?;
    let mut shell_config = ClientShellConfig::from_config(&config);
    shell_config.local_config_path = config_path.clone();
    let mut state = ClientShellState::new(shell_config);
    state.reload_organization_registry();
    if state.endpoint_error.is_some() {
        return Err("existing registry loader reported an error".into());
    }
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(api_path.clone()));
    println!("phase: API identity before");
    let before = session_snapshot(&api)?;
    let pane = before
        .panes
        .iter()
        .find(|p| p.pane_id == target)
        .ok_or("exact target pane absent")?;
    println!("phase: inactive endpoint snapshot");
    let snapshot = endpoint_snapshot(&socket)?;
    if !snapshot.panes.iter().any(|p| {
        p.pane_id == pane.pane_id && p.workspace_id == pane.workspace_id && p.tab_id == pane.tab_id
    }) {
        return Err("API/endpoint target mismatch".into());
    }
    state.set_snapshot(Box::new(snapshot.clone()));
    let rows = super::super::organization::rows(
        &snapshot,
        &state.config.organization,
        "local",
        &std::collections::HashSet::new(),
        None,
    );
    let selected = rows
        .iter()
        .find(|r| r.pane_id.as_deref() == Some(target.as_str()) && !r.invalid)
        .ok_or("target missing from existing organization projection")?;
    let geometry = before
        .layouts
        .iter()
        .flat_map(|l| &l.panes)
        .find(|p| p.pane_id == target)
        .ok_or("target layout unavailable")?
        .rect;
    if geometry.width == 0 || geometry.height == 0 {
        return Err("target has no observable geometry".into());
    }
    println!("phase: exact terminal observation");
    let frame = terminal_frame(&socket, &pane.terminal_id, geometry.width, geometry.height)?;
    println!("phase: API identity after");
    let after = session_snapshot(&api)?;
    if identity_map(&before) != identity_map(&after) {
        return Err(
            "pane/terminal identities changed during observation; no coherent proof".into(),
        );
    }
    let frame_hash = format!("{:x}", Sha256::digest(&frame.bytes));
    let config_hash = format!("{:x}", Sha256::digest(&config_bytes));
    let organization:Vec<_>=rows.iter().map(|r|serde_json::json!({"key":r.key,"parent_key":r.parent_key,"pane_id":r.pane_id,"label":r.label,"aggregate_status":r.status,"invalid":r.invalid})).collect();
    let report = serde_json::json!({
        "origin":"live endpoint and existing readonly TerminalObserve; no fixture",
        "observed_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(),
        "client_socket":socket,"api_socket":api_path,"config":config_path,"config_sha256":config_hash,
        "boot_id":snapshot.boot_id,"projection_revision":snapshot.revision,
        "agents":snapshot.agents.len(),"organization_rows":organization,
        "target":{"pane_id":pane.pane_id,"terminal_id":pane.terminal_id,"organization_key":selected.key},
        "frame":{"contract":"TerminalFrame (ANSI), not PaneSurfaceFrame","seq":frame.seq,"full":frame.full,"width":frame.width,"height":frame.height,"bytes":frame.bytes.len(),"sha256":frame_hash,"raw_exported":false},
        "preserved":{"pane_terminal_identities":true,"focused_pane":before.focused_pane_id==after.focused_pane_id,"layouts":before.layouts==after.layouts},
        "writes_to_protocol":["endpoint.hello.v1 surface_active=false","TerminalHello","ObserveTerminal exact terminal_id"],
        "pane_input_sent":false,"pane_resize_sent":false,"pane_focus_sent":false,"window_created":false,
        "semantic_surface_received":false,"terminal_rendered_natively":false,"m0_complete":false,"p1_complete":false,
        "limit":"Inactive endpoint supplies snapshot but no PaneSurfaceFrame. TerminalObserve supplies real ANSI. No second parser or fabricated semantic surface; live terminal graphical adapter remains pending."
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!("live readonly: {} agents; target {}; full frame {}x{}, {} bytes; identities preserved; raw bytes not exported",snapshot.agents.len(),target,frame.width,frame.height,frame.bytes.len());
    Ok(())
}
