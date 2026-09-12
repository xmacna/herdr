//! Real HWND host. No window is created by --capture or --check.
use super::{paint, Result, View};
use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
        },
        System::Com::*,
    },
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
struct Host<'a> {
    view: &'a mut View,
    target: Option<ID2D1HwndRenderTarget>,
    write: IDWriteFactory,
    factory: ID2D1Factory,
    width: u32,
    height: u32,
    failure: Option<String>,
    high_surrogate: Option<u16>,
}
unsafe extern "system" fn procedure(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Host<'static>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let host = &mut *pointer;
    match message {
        WM_SIZE => {
            host.width = (lparam as u32) & 65535;
            host.height = ((lparam as u32) >> 16) & 65535;
            if let Some(t) = &host.target {
                if t.Resize(&D2D_SIZE_U {
                    width: host.width.max(1),
                    height: host.height.max(1),
                })
                .is_err()
                {
                    host.target = None;
                }
            }
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_TIMER => {
            if host.view.poll() {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let scale = host.width.max(1) as f32 / 1280.;
            host.view.click(
                (lparam as u16 as i16) as f32 / scale,
                ((lparam as u32 >> 16) as u16 as i16) as f32 / scale,
            );
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as u16 as i16) as f32;
            let scale = host.width.max(1) as f32 / 1280.;
            let visible = host.height as f32 / scale;
            let max = host
                .view
                .scene
                .as_ref()
                .map(|s| (s.height as f32 - visible).max(0.))
                .unwrap_or(0.);
            host.view.scroll = (host.view.scroll - delta * 0.7).clamp(0., max);
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_KEYDOWN if wparam == 0x75 => {
            host.view.terminal_only = !host.view.terminal_only;
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_KEYDOWN
            if host.view.input.is_some() && GetForegroundWindow() == hwnd && GetFocus() == hwnd =>
        {
            use crossterm::event::{KeyCode, KeyModifiers};
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
            let ctrl = GetKeyState(0x11) < 0;
            let shift = GetKeyState(0x10) < 0;
            let alt = GetKeyState(0x12) < 0;
            if ctrl && wparam == 0x56 {
                if let Some(text) = crate::platform::read_clipboard_text() {
                    host.view
                        .input_event(crate::protocol::ClientPaneInputEvent::Paste(text));
                }
                return 0;
            }
            let code = match wparam {
                0x08 => Some(KeyCode::Backspace),
                0x09 => Some(if shift {
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                }),
                0x0d => Some(KeyCode::Enter),
                0x1b => Some(KeyCode::Esc),
                0x21 => Some(KeyCode::PageUp),
                0x22 => Some(KeyCode::PageDown),
                0x23 => Some(KeyCode::End),
                0x24 => Some(KeyCode::Home),
                0x25 => Some(KeyCode::Left),
                0x26 => Some(KeyCode::Up),
                0x27 => Some(KeyCode::Right),
                0x28 => Some(KeyCode::Down),
                0x2d => Some(KeyCode::Insert),
                0x2e => Some(KeyCode::Delete),
                0x70..=0x7b => Some(KeyCode::F((wparam - 0x70 + 1) as u8)),
                0x41..=0x5a if ctrl => {
                    Some(KeyCode::Char((wparam as u8 as char).to_ascii_lowercase()))
                }
                _ => None,
            };
            if let Some(code) = code {
                let mut modifiers = KeyModifiers::empty();
                if ctrl {
                    modifiers |= KeyModifiers::CONTROL;
                }
                if shift {
                    modifiers |= KeyModifiers::SHIFT;
                }
                if alt {
                    modifiers |= KeyModifiers::ALT;
                }
                if let Some(event) = crate::protocol::ClientPaneInputEvent::from_terminal_key(
                    crate::input::TerminalKey::new(code, modifiers),
                ) {
                    host.view.input_event(event);
                }
            }
            0
        }
        WM_CHAR
            if host.view.input.is_some() && GetForegroundWindow() == hwnd && GetFocus() == hwnd =>
        {
            // WM_CHAR carries UTF-16 units. Keep surrogate pairs together; no ANSI parser here.
            let unit = wparam as u16;
            if (0xd800..=0xdbff).contains(&unit) {
                host.high_surrogate = Some(unit);
            } else if unit >= 32 {
                let units = if let Some(high) = host.high_surrogate.take() {
                    vec![high, unit]
                } else {
                    vec![unit]
                };
                if let Ok(text) = String::from_utf16(&units) {
                    host.view
                        .input_event(crate::protocol::ClientPaneInputEvent::TextCommit(text));
                }
            }
            0
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let result = (|| -> Result<()> {
                if host.target.is_none() {
                    host.target = Some(host.factory.CreateHwndRenderTarget(
                        &D2D1_RENDER_TARGET_PROPERTIES {
                            dpiX: 96.,
                            dpiY: 96.,
                            ..Default::default()
                        },
                        &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                            hwnd: HWND(hwnd),
                            pixelSize: D2D_SIZE_U {
                                width: host.width.max(1),
                                height: host.height.max(1),
                            },
                            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                        },
                    )?);
                }
                let t = host.target.as_ref().unwrap();
                let base: ID2D1RenderTarget = t.cast()?;
                let scale = host.width.max(1) as f32 / 1280.;
                base.SetTransform(&windows_numerics::Matrix3x2 {
                    M11: scale,
                    M12: 0.,
                    M21: 0.,
                    M22: scale,
                    M31: 0.,
                    M32: 0.,
                });
                base.BeginDraw();
                let draw = paint::view(&base, &host.write, host.view, host.height as f32 / scale);
                let end = base.EndDraw(None, None);
                draw?;
                end?;
                Ok(())
            })();
            if let Err(e) = result {
                host.failure = Some(e.to_string());
                host.target = None;
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_CLOSE => {
            *host.view.request.lock().unwrap() = String::new();
            host.view.input = None;
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
pub(super) fn run(view: &mut View) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut host = Host {
            view,
            target: None,
            write,
            factory,
            width: 1280,
            height: 900,
            failure: None,
            high_surrogate: None,
        };
        let name: Vec<u16> = "HerdrArgosNativeExperimental\0".encode_utf16().collect();
        let title: Vec<u16> = "Sistema Argos — Herdr nativo experimental — observação\0"
            .encode_utf16()
            .collect();
        let instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1300,
            960,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            &mut host as *mut _ as *const _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        SetTimer(hwnd, 1, 150, None);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if result == 0 {
                break;
            }
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(e) = host.failure {
            return Err(e.into());
        }
        Ok(())
    }
}
