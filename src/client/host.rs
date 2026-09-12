//! Host selection and capability policy for the client shell.
//!
//! ANSI remains the default.  The Win32 mode is deliberately a capability description: the
//! platform host owns its window and message loop, while the client keeps the normal event,
//! dispatch, endpoint, and presentation paths.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostMode {
    Ansi,
    Win32Experimental,
}

pub(crate) const NATIVE_CELL_WIDTH_PX: u32 = 12;
pub(crate) const NATIVE_CELL_HEIGHT_PX: u32 = 22;
pub(crate) const NATIVE_WINDOW_WIDTH_PX: u32 = 1280;
pub(crate) const NATIVE_WINDOW_HEIGHT_PX: u32 = 720;

pub(crate) fn native_grid_size(width_px: u32, height_px: u32) -> io::Result<(u16, u16)> {
    let cols = width_px / NATIVE_CELL_WIDTH_PX;
    let rows = height_px / NATIVE_CELL_HEIGHT_PX;
    let cols = u16::try_from(cols.max(1))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "native host width is too large"))?;
    let rows = u16::try_from(rows.max(1))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "native host height is too large"))?;
    Ok((cols, rows))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostCapabilities {
    pub(crate) ansi_output: bool,
    pub(crate) presentation: bool,
    pub(crate) input: bool,
    pub(crate) resize: bool,
    pub(crate) cursor: bool,
    pub(crate) mouse: bool,
    pub(crate) keyboard_report: bool,
    pub(crate) theme: bool,
    pub(crate) clipboard: bool,
    pub(crate) bell: bool,
    pub(crate) title: bool,
}

impl HostCapabilities {
    pub(crate) const fn ansi() -> Self {
        Self {
            ansi_output: true,
            presentation: false,
            input: true,
            resize: true,
            cursor: true,
            mouse: true,
            keyboard_report: true,
            theme: true,
            clipboard: true,
            bell: true,
            title: true,
        }
    }

    pub(crate) const fn win32_experimental() -> Self {
        Self {
            ansi_output: false,
            presentation: true,
            input: true,
            resize: true,
            cursor: true,
            mouse: false,
            keyboard_report: false,
            theme: false,
            clipboard: false,
            bell: false,
            title: false,
        }
    }
}

impl HostMode {
    pub(crate) fn from_environment() -> io::Result<Self> {
        match std::env::var("HERDR_CLIENT_HOST") {
            Ok(value) if value.trim().is_empty() || value == "ansi" => Ok(Self::Ansi),
            Ok(value) if value == "win32-experimental" => {
                if cfg!(windows) {
                    Ok(Self::Win32Experimental)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "HERDR_CLIENT_HOST=win32-experimental requires Windows",
                    ))
                }
            }
            Ok(value) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported HERDR_CLIENT_HOST value: {value}"),
            )),
            Err(std::env::VarError::NotPresent) => Ok(Self::Ansi),
            Err(std::env::VarError::NotUnicode(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HERDR_CLIENT_HOST is not valid Unicode",
            )),
        }
    }

    pub(crate) const fn capabilities(self) -> HostCapabilities {
        match self {
            Self::Ansi => HostCapabilities::ansi(),
            Self::Win32Experimental => HostCapabilities::win32_experimental(),
        }
    }
}

#[cfg(windows)]
pub(super) struct NativeHostLease(Option<crate::platform::windows_host::NativeHostSession>);

#[cfg(windows)]
impl NativeHostLease {
    pub(super) fn new(session: crate::platform::windows_host::NativeHostSession) -> Self {
        Self(Some(session))
    }
}

#[cfg(windows)]
impl Drop for NativeHostLease {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            if let Err(error) = session.teardown() {
                tracing::warn!(%error, "native host teardown failed");
            }
        }
    }
}

#[cfg(windows)]
pub(super) fn forward_native_event(
    event: crate::platform::windows_host::NativeHostEvent,
    event_tx: &tokio::sync::mpsc::Sender<super::events::ClientLoopEvent>,
) -> Result<(), crate::platform::windows_host::NativeHostError> {
    use crate::platform::windows_host::{NativeHostError, NativeHostEvent};

    let event = match event {
        NativeHostEvent::Ready {
            hwnd,
            width,
            height,
        } => super::events::ClientLoopEvent::NativeHostReady {
            hwnd,
            width,
            height,
        },
        NativeHostEvent::Input(events) => super::events::ClientLoopEvent::StdinEvents(events),
        NativeHostEvent::Resize { width, height } => {
            super::events::ClientLoopEvent::NativeHostResize { width, height }
        }
        NativeHostEvent::Focus { focused } => {
            super::events::ClientLoopEvent::NativeHostFocus { focused }
        }
        NativeHostEvent::Redraw => super::events::ClientLoopEvent::NativeHostRedraw,
        NativeHostEvent::Close => super::events::ClientLoopEvent::NativeHostClose,
        NativeHostEvent::Error(error) => {
            let message = error.to_string();
            event_tx
                .blocking_send(super::events::ClientLoopEvent::NativeHostError(message))
                .map_err(|_| NativeHostError::Unsupported("client event loop is closed"))?;
            return Err(error);
        }
    };

    event_tx
        .blocking_send(event)
        .map_err(|_| NativeHostError::Unsupported("client event loop is closed"))
}
