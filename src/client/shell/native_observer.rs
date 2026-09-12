//! Existing Herdr read-only codecs shared by the finite proof and experimental host.
use crate::api::{
    client::ApiClient,
    schema::{EmptyParams, Method, Request, ResponseResult, SessionSnapshot},
};
use crate::ipc::{LocalStream, LocalStreamReadCount};
use crate::protocol::{
    self, endpoint::*, ClientMessage, ClientShellSnapshot, ServerMessage, TerminalFrame,
};
use std::{
    io::{self, Read},
    path::Path,
    time::{Duration, Instant},
};
pub(super) type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;
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
pub(super) fn endpoint_snapshot(socket: &Path) -> ProbeResult<ClientShellSnapshot> {
    Ok(endpoint_connection(
        socket,
        false,
        protocol::ClientSurfaceSize { cols: 80, rows: 24 },
    )?
    .1)
}

#[cfg(feature = "native-demo")]
pub(super) fn isolated_endpoint(
    socket: &Path,
    size: protocol::ClientSurfaceSize,
    _permit: &super::native_input::ActivePermit,
) -> ProbeResult<(LocalStream, ClientShellSnapshot)> {
    endpoint_connection(socket, true, size)
}
fn endpoint_connection(
    socket: &Path,
    active: bool,
    size: protocol::ClientSurfaceSize,
) -> ProbeResult<(LocalStream, ClientShellSnapshot)> {
    let mut stream = connect_observer(socket)?;
    let hello = EndpointClientHello {
        generation: ENDPOINT_PROTOCOL_GENERATION,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: size,
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: active,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    let message = ClientMessage::EndpointControl {
        kind: ENDPOINT_HELLO_KIND.into(),
        data: serde_json::to_string(&hello)?,
    };
    if active {
        protocol::write_message(&mut stream, &message)?;
    } else {
        send_observation(&mut stream, &message)?;
    }
    match read_message(&mut stream)? {
        ServerMessage::EndpointControl { kind, data } if kind == ENDPOINT_WELCOME_KIND => {
            let welcome: EndpointServerWelcome = serde_json::from_str(&data)?;
            if welcome.error.is_some()
                || welcome.generation != ENDPOINT_PROTOCOL_GENERATION
                || welcome.snapshot_codec != SNAPSHOT_CODEC_V1
                || (active && welcome.input_codec != INPUT_CODEC_V1)
            {
                return Err("incompatible endpoint welcome".into());
            }
        }
        _ => return Err("expected stable endpoint welcome".into()),
    }
    match read_message(&mut stream)? {
        ServerMessage::EndpointControl { kind, data } if kind == ENDPOINT_SNAPSHOT_KIND => {
            Ok((stream, serde_json::from_str(&data)?))
        }
        _ => Err("expected existing ClientShellSnapshot codec".into()),
    }
    // Drop only this inactive connection. No Detach/input/resize request is emitted.
}
pub(super) fn terminal_frame(
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
pub(super) fn session_snapshot(api: &ApiClient) -> ProbeResult<SessionSnapshot> {
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
