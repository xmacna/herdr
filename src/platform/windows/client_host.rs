//! Explicit, opt-in Win32 host boundary for the native client.
//!
//! Construction and normalization are side-effect free.  `start` is the only
//! future entry point allowed to own a window/message loop; callers must opt in
//! explicitly and must not use this module as a second client state machine.

use crate::protocol::{ClientInputEvent, ClientKeyCode, ClientKeyKind, ClientKeySource};
use std::sync::mpsc;
use std::thread::JoinHandle;

const MAX_PASTE_BYTES: usize = 8 * 1024 * 1024;
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0d;
const VK_ESCAPE: u32 = 0x1b;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_INSERT: u32 = 0x2d;
const VK_DELETE: u32 = 0x2e;
const VK_F1: u32 = 0x70;
const VK_F24: u32 = 0x87;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_LWIN: i32 = 0x5b;
const VK_RWIN: i32 = 0x5c;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeHostConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl NativeHostConfig {
    pub(crate) fn validate(&self) -> Result<(), NativeHostError> {
        if self.title.trim().is_empty() {
            return Err(NativeHostError::InvalidConfig("window title is empty"));
        }
        if self.width == 0
            || self.height == 0
            || self.width > i32::MAX as u32
            || self.height > i32::MAX as u32
        {
            return Err(NativeHostError::InvalidConfig("window size is zero"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeHostEvent {
    Ready {
        hwnd: usize,
        width: u32,
        height: u32,
    },
    Input(Vec<ClientInputEvent>),
    Resize {
        width: u32,
        height: u32,
    },
    Focus {
        focused: bool,
    },
    Redraw,
    Close,
    Error(NativeHostError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeHostError {
    InvalidConfig(&'static str),
    InvalidUtf16,
    PasteTooLarge,
    Unsupported(&'static str),
    Platform(String),
}

impl std::fmt::Display for NativeHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid native host config: {message}"),
            Self::InvalidUtf16 => f.write_str("invalid UTF-16 sequence from WM_CHAR"),
            Self::PasteTooLarge => f.write_str("native paste exceeds the 8 MiB limit"),
            Self::Unsupported(message) => f.write_str(message),
            Self::Platform(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for NativeHostError {}

#[derive(Debug)]
pub(crate) struct NativeHostSession {
    config: NativeHostConfig,
    hwnd_raw: Option<usize>,
    thread_id: Option<u32>,
    join: Option<JoinHandle<Result<(), NativeHostError>>>,
}

#[derive(Debug, Default)]
pub(crate) struct NativeMessageNormalizer {
    pending_high_surrogate: Option<u16>,
    awaiting_text_key: bool,
    suppress_char: bool,
}

impl NativeMessageNormalizer {
    pub(crate) fn key(
        &mut self,
        virtual_key: u32,
        modifiers: u8,
        key_down: bool,
        repeat_count: u16,
    ) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        if is_modifier_vk(virtual_key) {
            self.awaiting_text_key = false;
            self.suppress_char = false;
            return Ok(Vec::new());
        }
        let code = virtual_key_code(virtual_key).unwrap_or(ClientKeyCode::Null);
        self.awaiting_text_key = false;
        self.suppress_char = false;
        if key_down {
            if let ClientKeyCode::Char(ch) = code {
                if modifiers & (2 | 4 | 8) == 0 {
                    self.awaiting_text_key = true;
                    return Ok(Vec::new());
                }
                let code = if modifiers & 2 != 0 {
                    ClientKeyCode::Char(ch.to_ascii_lowercase())
                } else {
                    code
                };
                self.suppress_char = true;
                return Ok(vec![ClientInputEvent::Key {
                    code,
                    modifiers,
                    kind: ClientKeyKind::Press,
                    repeat_count: repeat_count.max(1),
                    generated_text: None,
                    source: ClientKeySource::Synthesized,
                }]);
            }
            if code == ClientKeyCode::Null {
                return Ok(Vec::new());
            }
            if matches!(
                code,
                ClientKeyCode::Backspace | ClientKeyCode::Tab | ClientKeyCode::Enter
            ) {
                self.suppress_char = true;
            }
        }
        Ok(vec![ClientInputEvent::Key {
            code,
            modifiers,
            kind: if key_down {
                ClientKeyKind::Press
            } else {
                ClientKeyKind::Release
            },
            repeat_count: repeat_count.max(1),
            generated_text: None,
            source: ClientKeySource::Synthesized,
        }])
    }

    pub(crate) fn key_message(
        &mut self,
        virtual_key: u32,
        lparam: isize,
        key_down: bool,
        modifiers: u8,
    ) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        let repeat_count = (lparam as u32 & 0xffff) as u16;
        let previous = (lparam as u32 & (1 << 30)) != 0;
        let mut events = self.key(virtual_key, modifiers, key_down, repeat_count.max(1))?;
        if previous && key_down {
            for event in &mut events {
                if let ClientInputEvent::Key { kind, .. } = event {
                    *kind = ClientKeyKind::Repeat;
                }
            }
        }
        Ok(events)
    }

    pub(crate) fn wm_char(&mut self, unit: u16) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        self.wm_char_message(unit, 1, 0)
    }

    pub(crate) fn wm_char_with_modifiers(
        &mut self,
        unit: u16,
        modifiers: u8,
    ) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        self.wm_char_message(unit, 1, modifiers)
    }

    pub(crate) fn wm_char_message(
        &mut self,
        unit: u16,
        lparam: isize,
        modifiers: u8,
    ) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        let repeat_count = ((lparam as u32) & 0xffff) as u16;
        let previous = ((lparam as u32) & (1 << 30)) != 0;
        let scalar = match (self.pending_high_surrogate.take(), unit) {
            (Some(high), low @ 0xdc00..=0xdfff) => {
                Some(0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00))
            }
            (Some(_), _) if (0xd800..=0xdbff).contains(&unit) => {
                self.pending_high_surrogate = Some(unit);
                None
            }
            (Some(_), _) => return Err(NativeHostError::InvalidUtf16),
            (None, high @ 0xd800..=0xdbff) => {
                self.pending_high_surrogate = Some(high);
                None
            }
            (None, 0xdc00..=0xdfff) => return Err(NativeHostError::InvalidUtf16),
            (None, value) => Some(u32::from(value)),
        };
        match scalar.and_then(char::from_u32) {
            Some(ch) => {
                if self.suppress_char {
                    self.suppress_char = false;
                    return Ok(Vec::new());
                }
                // Enter/Tab/Escape/Backspace already have semantic key messages.
                // WM_CHAR control codes must not become a second pane event.
                if ch.is_control() {
                    return Ok(Vec::new());
                }
                let ch = if modifiers & 2 != 0 {
                    ch.to_ascii_lowercase()
                } else {
                    ch
                };
                self.awaiting_text_key = false;
                Ok(vec![ClientInputEvent::Key {
                    code: ClientKeyCode::Char(ch),
                    modifiers,
                    repeat_count: repeat_count.max(1),
                    kind: if previous {
                        ClientKeyKind::Repeat
                    } else {
                        ClientKeyKind::Press
                    },
                    generated_text: Some(ch.to_string()),
                    source: ClientKeySource::Synthesized,
                }])
            }
            None => Ok(Vec::new()),
        }
    }

    pub(crate) fn paste(text: &str) -> Result<Vec<ClientInputEvent>, NativeHostError> {
        if text.len() > MAX_PASTE_BYTES {
            return Err(NativeHostError::PasteTooLarge);
        }
        Ok(vec![ClientInputEvent::Paste {
            text: text.to_owned(),
        }])
    }
}

impl NativeHostSession {
    pub(crate) fn new(config: NativeHostConfig) -> Result<Self, NativeHostError> {
        config.validate()?;
        Ok(Self {
            config,
            hwnd_raw: None,
            thread_id: None,
            join: None,
        })
    }

    pub(crate) fn config(&self) -> &NativeHostConfig {
        &self.config
    }

    pub(crate) fn start<F>(
        config: NativeHostConfig,
        mut callback: F,
    ) -> Result<Self, NativeHostError>
    where
        F: FnMut(NativeHostEvent) -> Result<(), NativeHostError> + Send + 'static,
    {
        config.validate()?;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let session_config = config.clone();
        let join = std::thread::Builder::new()
            .name("herdr-native-host".into())
            .spawn(move || {
                let result = run_message_loop(config, &mut callback, &ready_tx);
                if let Err(error) = &result {
                    // On early failure release start's readiness wait; after readiness wake the client.
                    let _ = ready_tx.try_send(Err(error.clone()));
                    let _ = callback(NativeHostEvent::Error(error.clone()));
                }
                result
            })
            .map_err(platform_error)?;
        match ready_rx.recv() {
            Ok(Ok((hwnd_raw, thread_id))) => Ok(Self {
                config: session_config,
                hwnd_raw: Some(hwnd_raw),
                thread_id: Some(thread_id),
                join: Some(join),
            }),
            response => {
                let original = match response {
                    Ok(Err(error)) => error,
                    Err(error) => platform_error(error),
                    _ => unreachable!(),
                };
                let completion = join
                    .join()
                    .map_err(|_| platform_error("native host thread panicked"))?;
                Err(completion.err().unwrap_or(original))
            }
        }
    }

    pub(crate) fn normalizer() -> NativeMessageNormalizer {
        NativeMessageNormalizer::default()
    }
    pub(crate) fn hwnd(&self) -> Option<windows::Win32::Foundation::HWND> {
        self.hwnd_raw
            .map(|raw| windows::Win32::Foundation::HWND(raw as *mut std::ffi::c_void))
    }

    pub(crate) fn teardown(mut self) -> Result<(), NativeHostError> {
        use windows::Win32::{
            Foundation::{LPARAM, WPARAM},
            UI::WindowsAndMessaging::{PostMessageW, PostThreadMessageW, WM_CLOSE, WM_QUIT},
        };
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        let post_error = if !join.is_finished() {
            let result = match self.hwnd() {
                Some(hwnd) => unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(platform_error),
                None => Err(platform_error("native host has no window")),
            };
            if let Err(error) = result {
                // The owned window may already be gone. Wake its own message queue to finish cleanup.
                if let Some(thread_id) = self.thread_id {
                    unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
                        .map_err(platform_error)?;
                }
                Some(error)
            } else {
                None
            }
        } else {
            None
        };
        let completion = join
            .join()
            .map_err(|_| platform_error("native host thread panicked"))?;
        completion.and_then(|()| post_error.map_or(Ok(()), Err))
    }
}

fn platform_error(error: impl std::fmt::Display) -> NativeHostError {
    NativeHostError::Platform(error.to_string())
}

#[cfg(windows)]
fn run_message_loop<F>(
    config: NativeHostConfig,
    callback: &mut F,
    ready_tx: &mpsc::SyncSender<Result<(usize, u32), NativeHostError>>,
) -> Result<(), NativeHostError>
where
    F: FnMut(NativeHostEvent) -> Result<(), NativeHostError>,
{
    use std::{cell::RefCell, mem::size_of};
    use windows::{
        core::w,
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            Graphics::Gdi::ValidateRect,
            System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
            UI::{Input::KeyboardAndMouse::GetKeyState, WindowsAndMessaging::*},
        },
    };
    const WAKE: u32 = WM_APP + 71;
    struct Context {
        sender: mpsc::Sender<NativeHostEvent>,
        failure: RefCell<Option<NativeHostError>>,
    }
    impl Context {
        fn send(&self, hwnd: HWND, event: NativeHostEvent) {
            let result = self
                .sender
                .send(event)
                .map_err(platform_error)
                .and_then(|()| {
                    unsafe { PostMessageW(Some(hwnd), WAKE, WPARAM(0), LPARAM(0)) }
                        .map_err(platform_error)
                });
            if let Err(error) = result {
                *self.failure.borrow_mut() = Some(error);
                unsafe {
                    PostQuitMessage(1);
                }
            }
        }
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCREATE {
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            return LRESULT(1);
        }
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Context;
        let context = unsafe { ptr.as_ref() };
        match message {
            WAKE => LRESULT(0),
            WM_PAINT => {
                let valid = unsafe { ValidateRect(Some(hwnd), None) }
                    .ok()
                    .map_err(platform_error);
                if let Some(context) = context {
                    match valid {
                        Ok(()) => context.send(hwnd, NativeHostEvent::Redraw),
                        Err(error) => {
                            *context.failure.borrow_mut() = Some(error);
                            unsafe {
                                PostQuitMessage(1);
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_SIZE | WM_SETFOCUS | WM_KILLFOCUS => {
                if let Some(context) = context {
                    let event = if message == WM_SIZE {
                        NativeHostEvent::Resize {
                            width: (lparam.0 as u32) & 0xffff,
                            height: ((lparam.0 as u32) >> 16) & 0xffff,
                        }
                    } else {
                        NativeHostEvent::Focus {
                            focused: message == WM_SETFOCUS,
                        }
                    };
                    context.send(hwnd, event);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(context) = context {
                    if let Err(error) = context.sender.send(NativeHostEvent::Close) {
                        *context.failure.borrow_mut() = Some(platform_error(error));
                    }
                }
                unsafe {
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            WM_NCDESTROY => unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            },
            WM_CLOSE => {
                if let Err(error) = unsafe { DestroyWindow(hwnd) } {
                    if let Some(context) = context {
                        *context.failure.borrow_mut() = Some(platform_error(error));
                    }
                    unsafe {
                        PostQuitMessage(1);
                    }
                }
                LRESULT(0)
            }
            // Semantic input is handled once by the queue normalizer below.
            WM_CHAR | WM_KEYDOWN | WM_KEYUP => LRESULT(0),
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
    struct Class(HINSTANCE);
    impl Drop for Class {
        fn drop(&mut self) {
            let _ = unsafe { UnregisterClassW(w!("HerdrIntegratedClientHost"), Some(self.0)) };
        }
    }
    struct OwnedWindow(HWND);
    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.0)) }.as_bool() {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
    }
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(platform_error)?
        .into();
    let definition = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: w!("HerdrIntegratedClientHost"),
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&definition) } == 0 {
        return Err(platform_error(windows::core::Error::from_thread()));
    }
    let _class = Class(instance);
    let (events_tx, events_rx) = mpsc::channel();
    // Context outlives OwnedWindow; DestroyWindow can still deliver messages during unwind.
    let context = Box::new(Context {
        sender: events_tx,
        failure: RefCell::new(None),
    });
    let title: Vec<u16> = config.title.encode_utf16().chain([0]).collect();
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("HerdrIntegratedClientHost"),
            windows::core::PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            config.width as i32,
            config.height as i32,
            None,
            None,
            Some(instance),
            Some((&*context as *const Context).cast()),
        )
    }
    .map_err(platform_error)?;
    let _window = OwnedWindow(hwnd);
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(platform_error)?;
    let width = (rect.right - rect.left).max(1) as u32;
    let height = (rect.bottom - rect.top).max(1) as u32;
    callback(NativeHostEvent::Ready {
        hwnd: hwnd.0 as usize,
        width,
        height,
    })?;
    ready_tx
        .send(Ok((hwnd.0 as usize, unsafe { GetCurrentThreadId() })))
        .map_err(platform_error)?;
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNORMAL) };
    let mut normalizer = NativeMessageNormalizer::default();
    let mut message = MSG::default();
    loop {
        while let Ok(event) = events_rx.try_recv() {
            callback(event)?;
        }
        if let Some(error) = context.failure.borrow_mut().take() {
            return Err(error);
        }
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err(platform_error(windows::core::Error::from_thread()));
        }
        if result == 0 {
            while let Ok(event) = events_rx.try_recv() {
                callback(event)?;
            }
            return context.failure.borrow_mut().take().map_or(Ok(()), Err);
        }
        let modifiers = || {
            keyboard_modifiers(
                unsafe { GetKeyState(VK_SHIFT) },
                unsafe { GetKeyState(VK_CONTROL) },
                unsafe { GetKeyState(VK_MENU) },
                unsafe { GetKeyState(VK_LWIN) },
                unsafe { GetKeyState(VK_RWIN) },
            )
        };
        let events = match message.message {
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => normalizer.key_message(
                message.wParam.0 as u32,
                message.lParam.0,
                message.message == WM_KEYDOWN || message.message == WM_SYSKEYDOWN,
                modifiers(),
            )?,
            WM_CHAR | WM_SYSCHAR => normalizer.wm_char_message(
                message.wParam.0 as u16,
                message.lParam.0,
                modifiers(),
            )?,
            _ => Vec::new(),
        };
        if !events.is_empty() {
            callback(NativeHostEvent::Input(events))?;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(windows)]
pub(crate) fn keyboard_modifiers(
    shift: i16,
    control: i16,
    alt: i16,
    left_win: i16,
    right_win: i16,
) -> u8 {
    let down = |state: i16| state < 0;
    (if down(shift) { 1 } else { 0 })
        | (if down(control) { 2 } else { 0 })
        | (if down(alt) { 4 } else { 0 })
        | (if down(left_win) || down(right_win) {
            8
        } else {
            0
        })
}

fn virtual_key_code(value: u32) -> Option<ClientKeyCode> {
    Some(match value {
        VK_BACK => ClientKeyCode::Backspace,
        VK_TAB => ClientKeyCode::Tab,
        VK_RETURN => ClientKeyCode::Enter,
        VK_ESCAPE => ClientKeyCode::Esc,
        VK_SPACE => ClientKeyCode::Char(' '),
        VK_PRIOR => ClientKeyCode::PageUp,
        VK_NEXT => ClientKeyCode::PageDown,
        VK_END => ClientKeyCode::End,
        VK_HOME => ClientKeyCode::Home,
        VK_LEFT => ClientKeyCode::Left,
        VK_UP => ClientKeyCode::Up,
        VK_RIGHT => ClientKeyCode::Right,
        VK_DOWN => ClientKeyCode::Down,
        VK_INSERT => ClientKeyCode::Insert,
        VK_DELETE => ClientKeyCode::Delete,
        VK_F1..=VK_F24 => ClientKeyCode::F((value - VK_F1 + 1) as u8),
        value if (0x30..=0x39).contains(&value) || (0x41..=0x5a).contains(&value) => {
            ClientKeyCode::Char(char::from_u32(value).expect("ASCII virtual key"))
        }
        _ => ClientKeyCode::Null,
    })
}

fn is_modifier_vk(value: u32) -> bool {
    matches!(value, 0x10 | 0x11 | 0x12 | 0x5b | 0x5c)
}
