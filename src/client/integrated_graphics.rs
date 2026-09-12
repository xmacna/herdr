//! Opt-in off-screen presentation sink for the integrated client compositor.
//!
//! This module owns no client state, input, layout, selection, endpoint, or socket. It
//! consumes the semantic operations and complete frame produced by the shared compositor.

use std::{
    error::Error,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    time::Instant,
};

use super::{
    IntegratedOfficeOperation, IntegratedPresentationError, IntegratedPresentationFrame,
    IntegratedPresentationSink, IntegratedPresentationUpdate,
};

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Graphics::{
                Direct2D::{Common::*, *},
                DirectWrite::*,
                Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                Imaging::*,
            },
            Foundation::{HWND, RECT},
            System::{Com::*, ProcessStatus::*, Threading::GetCurrentProcess},
        },
    };

    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn fail(error: impl Error + Send + Sync + 'static) -> IntegratedPresentationError {
        IntegratedPresentationError::new(error.to_string())
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
        D2D_RECT_F {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    fn overlaps(left: ratatui::layout::Rect, right: ratatui::layout::Rect) -> bool {
        let left_right = u32::from(left.x) + u32::from(left.width);
        let right_right = u32::from(right.x) + u32::from(right.width);
        let left_bottom = u32::from(left.y) + u32::from(left.height);
        let right_bottom = u32::from(right.y) + u32::from(right.height);
        u32::from(left.x) < right_right
            && u32::from(right.x) < left_right
            && u32::from(left.y) < right_bottom
            && u32::from(right.y) < left_bottom
    }

    fn color(value: u32, fallback: (f32, f32, f32)) -> D2D1_COLOR_F {
        if value >> 24 == 0x02 {
            D2D1_COLOR_F {
                r: ((value >> 16) & 255) as f32 / 255.0,
                g: ((value >> 8) & 255) as f32 / 255.0,
                b: (value & 255) as f32 / 255.0,
                a: 1.0,
            }
        } else {
            D2D1_COLOR_F {
                r: fallback.0,
                g: fallback.1,
                b: fallback.2,
                a: 1.0,
            }
        }
    }

    fn draw_text(
        target: &ID2D1RenderTarget,
        format: &IDWriteTextFormat,
        brush: &ID2D1SolidColorBrush,
        text: &str,
        bounds: D2D_RECT_F,
    ) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            target.DrawText(
                &utf16,
                format,
                &bounds,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn paint_frame(
        target: &ID2D1RenderTarget,
        format: &IDWriteTextFormat,
        frame: &crate::protocol::FrameData,
        cell_w: f32,
        cell_h: f32,
    ) -> Result<(), IntegratedPresentationError> {
        for (index, cell) in frame.cells.iter().enumerate() {
            let col = index % usize::from(frame.width);
            let row = index / usize::from(frame.width);
            let background = unsafe {
                target
                    .CreateSolidColorBrush(&color(cell.bg, (0.05, 0.07, 0.08)), None)
                    .map_err(fail)?
            };
            let foreground = unsafe {
                target
                    .CreateSolidColorBrush(&color(cell.fg, (0.86, 0.90, 0.90)), None)
                    .map_err(fail)?
            };
            let bounds = rect(col as f32 * cell_w, row as f32 * cell_h, cell_w, cell_h);
            unsafe {
                target.FillRectangle(&bounds, &background);
            }
            if !cell.skip && !cell.symbol.is_empty() {
                draw_text(target, format, &foreground, &cell.symbol, bounds);
            }
        }
        if let Some(cursor) = &frame.cursor {
            if cursor.visible {
                let bounds = rect(
                    cursor.x as f32 * cell_w,
                    cursor.y as f32 * cell_h + cell_h - 2.0,
                    cell_w,
                    2.0,
                );
                let brush = unsafe {
                    target
                        .CreateSolidColorBrush(&color(0x0200D6A3, (0.0, 0.84, 0.64)), None)
                        .map_err(fail)?
                };
                unsafe {
                    target.FillRectangle(&bounds, &brush);
                }
            }
        }
        Ok(())
    }

    fn paint_organization(
        target: &ID2D1RenderTarget,
        title: &IDWriteTextFormat,
        body: &IDWriteTextFormat,
        frame: &IntegratedPresentationFrame,
        cell_w: f32,
        cell_h: f32,
    ) -> Result<(usize, usize), IntegratedPresentationError> {
        if !frame.office_operations_allowed {
            return Ok((0, frame.organization.len()));
        }
        let mut painted = 0usize;
        let mut occluded = 0usize;
        for operation in &frame.organization {
            let node = match operation {
                IntegratedOfficeOperation::Group(node)
                | IntegratedOfficeOperation::Person(node) => node,
            };
            if frame
                .overlay_occlusions
                .iter()
                .any(|overlay| overlaps(node.rect, *overlay))
            {
                occluded += 1;
                continue;
            }
            let x = f32::from(node.rect.x) * cell_w;
            let y = f32::from(node.rect.y) * cell_h;
            let width = f32::from(node.rect.width.max(1)) * cell_w;
            let height = f32::from(node.rect.height.max(1)) * cell_h;
            let fill = if node.selected {
                (0.16, 0.38, 0.32)
            } else {
                (0.10, 0.16, 0.18)
            };
            // The semantic card replaces its supplied rectangle. Clear it opaquely first so
            // ANSI/frame pixels cannot show through the card's rounded corners or alpha.
            let cover = unsafe {
                target
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.04,
                            g: 0.06,
                            b: 0.07,
                            a: 1.0,
                        },
                        None,
                    )
                    .map_err(fail)?
            };
            unsafe {
                target.FillRectangle(&rect(x, y, width, height), &cover);
            }
            let brush = unsafe {
                target
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: fill.0,
                            g: fill.1,
                            b: fill.2,
                            a: 1.0,
                        },
                        None,
                    )
                    .map_err(fail)?
            };
            unsafe {
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(x, y, width, height),
                        radiusX: 5.0,
                        radiusY: 5.0,
                    },
                    &brush,
                );
            }
            let ink = unsafe {
                target
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.92,
                            g: 0.96,
                            b: 0.95,
                            a: 1.0,
                        },
                        None,
                    )
                    .map_err(fail)?
            };
            draw_text(
                target,
                title,
                &ink,
                &node.name,
                rect(x + 6.0, y + 4.0, (width - 12.0).max(1.0), 20.0),
            );
            let detail = if node.meta.is_empty() {
                node.role.as_deref().unwrap_or("")
            } else {
                &node.meta
            };
            draw_text(
                target,
                body,
                &ink,
                detail,
                rect(
                    x + 6.0,
                    y + 25.0,
                    (width - 12.0).max(1.0),
                    (height - 29.0).max(1.0),
                ),
            );
            let segment = unsafe {
                target
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.46,
                            g: 0.84,
                            b: 0.70,
                            a: 1.0,
                        },
                        None,
                    )
                    .map_err(fail)?
            };
            if node.box_start {
                unsafe {
                    target.FillRectangle(&rect(x, y, width, 2.0), &segment);
                }
            }
            if node.box_end {
                unsafe {
                    target.FillRectangle(&rect(x, y + height - 2.0, width, 2.0), &segment);
                }
            }
            painted += 1;
        }
        Ok((painted, occluded))
    }

    pub(super) fn render(
        output: &Path,
        frame: &IntegratedPresentationFrame,
        update_kind: &str,
    ) -> Result<(), IntegratedPresentationError> {
        if !output.is_absolute() {
            return Err(IntegratedPresentationError::new(
                "integrated graphics output must be absolute",
            ));
        }
        let width = (u32::from(frame.frame.width).max(1) * 12).max(320);
        let height = (u32::from(frame.frame.height).max(1) * 22).max(180);
        let started = Instant::now();
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(fail)?;
        }
        let _apartment = ComApartment;
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(fail)? };
        let write: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(fail)? };
        let wic: IWICImagingFactory = unsafe {
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(fail)?
        };
        let bitmap = unsafe {
            wic.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )
            .map_err(fail)?
        };
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = unsafe {
            d2d.CreateWicBitmapRenderTarget(&bitmap, &properties)
                .map_err(fail)?
        };
        let title = unsafe {
            write
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    15.0,
                    w!("en-US"),
                )
                .map_err(fail)?
        };
        let body = unsafe {
            write
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    11.0,
                    w!("en-US"),
                )
                .map_err(fail)?
        };
        unsafe {
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F {
                r: 0.04,
                g: 0.06,
                b: 0.07,
                a: 1.0,
            }));
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        paint_frame(&target, &body, &frame.frame, 12.0, 22.0)?;
        let (painted_operations, occluded_operations) =
            paint_organization(&target, &title, &body, frame, 12.0, 22.0)?;
        unsafe {
            target.EndDraw(None, None).map_err(fail)?;
        }
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        unsafe {
            bitmap
                .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
                .map_err(fail)?;
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(output).map_err(fail)?),
            width,
            height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(fail)?;
        writer.write_image_data(&pixels).map_err(fail)?;
        writer.finish().map_err(fail)?;
        let mut memory = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut memory,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
            .map_err(fail)?;
        }
        let report = serde_json::json!({"api":"Direct2D + DirectWrite -> WIC bitmap; png crate only encodes","update_kind":update_kind,"width":width,"height":height,"identity": {"endpoint_id":frame.identity.endpoint_id,"generation":frame.identity.generation,"boot_id":frame.identity.boot_id,"projection_revision":frame.identity.projection_revision,"surface_revision":frame.identity.surface_revision},"organization_operations":frame.organization.len(),"organization_operations_painted":painted_operations,"organization_operations_occluded":occluded_operations,"organization_operations_allowed":frame.office_operations_allowed,"organization_hits":frame.organization_hits.len(),"overlay_occlusions":frame.overlay_occlusions.len(),"organization_order":"complete_frame_then_non_occluded_operations","palette":"truecolor 0x02RRGGBB is rendered; other packed/indexed values use explicit fallback colors","attributes_rendered":false,"unicode_fidelity":"not guaranteed; DirectWrite receives text but cell-level shaping/width/attributes are not preserved as a proof","window_created":false,"runtime_connected":false,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"working_set_bytes":memory.WorkingSetSize,"output":output});
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&report).map_err(fail)?,
        )
        .map_err(fail)?;
        Ok(())
    }

    pub(super) fn render_hwnd(
        hwnd: HWND,
        frame: &IntegratedPresentationFrame,
        update_kind: &str,
    ) -> Result<(), IntegratedPresentationError> {
        let _ = update_kind;
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(fail)?; }
        let _apartment = ComApartment;
        let d2d: ID2D1Factory = unsafe {
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(fail)?
        };
        let write: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(fail)? };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0, dpiY: 96.0, usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let mut client_rect = RECT::default();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client_rect)
                .map_err(fail)?;
        }
        let client_width = client_rect.right.saturating_sub(client_rect.left) as u32;
        let client_height = client_rect.bottom.saturating_sub(client_rect.top) as u32;
        if client_width == 0 || client_height == 0 {
            return Err(IntegratedPresentationError::new(
                "native host client area is empty",
            ));
        }
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U { width: client_width, height: client_height },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let target = unsafe { d2d.CreateHwndRenderTarget(&props, &hwnd_props).map_err(fail)? };
        let body = unsafe { write.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 11.0, w!("en-US")).map_err(fail)? };
        let title = unsafe { write.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 15.0, w!("en-US")).map_err(fail)? };
        unsafe {
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F { r: 0.04, g: 0.06, b: 0.07, a: 1.0 }));
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        paint_frame(&target, &body, &frame.frame, 12.0, 22.0)?;
        paint_organization(&target, &title, &body, frame, 12.0, 22.0)?;
        unsafe { target.EndDraw(None, None).map_err(fail)?; }
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) struct IntegratedGraphicsSink {
    output: Option<PathBuf>,
    hwnd: Option<windows::Win32::Foundation::HWND>,
    frozen: bool,
    awaiting_full: bool,
    sequence: u64,
}

#[cfg(windows)]
impl IntegratedGraphicsSink {
    pub(crate) fn new(output: impl Into<PathBuf>) -> Self {
        Self {
            output: Some(output.into()),
            hwnd: None,
            frozen: false,
            awaiting_full: false,
            sequence: 0,
        }
    }

    pub(crate) fn new_hwnd(
        hwnd: usize,
    ) -> Result<Self, IntegratedPresentationError> {
        if hwnd == 0 {
            return Err(IntegratedPresentationError::new("native host supplied a null HWND"));
        }
        Ok(Self { output: None, hwnd: Some(windows::Win32::Foundation::HWND(hwnd as *mut core::ffi::c_void)), frozen: false, awaiting_full: false, sequence: 0 })
    }

    pub(crate) fn redraw(
        &mut self,
        frame: &IntegratedPresentationFrame,
    ) -> Result<(), IntegratedPresentationError> {
        let hwnd = self.hwnd.ok_or_else(|| IntegratedPresentationError::new("redraw requires an HWND sink"))?;
        native::render_hwnd(hwnd, frame, "redraw")
    }

    fn current_output(&self) -> Result<PathBuf, IntegratedPresentationError> {
        if self.sequence == 0 {
            return self.output.clone().ok_or_else(|| IntegratedPresentationError::new("HWND sink has no bitmap output"));
        }
        let output = self.output.as_ref().ok_or_else(|| IntegratedPresentationError::new("HWND sink has no bitmap output"))?;
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let stem = output
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("integrated-presentation");
        let extension = output
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        Ok(parent.join(format!(
            "{stem}-{sequence:04}.{extension}",
            sequence = self.sequence
        )))
    }
}

#[cfg(windows)]
impl IntegratedPresentationSink for IntegratedGraphicsSink {
    fn present(
        &mut self,
        update: IntegratedPresentationUpdate<'_>,
    ) -> Result<(), IntegratedPresentationError> {
        match update {
            IntegratedPresentationUpdate::Frozen => {
                self.frozen = true;
                Ok(())
            }
            IntegratedPresentationUpdate::Thaw => {
                self.frozen = false;
                self.awaiting_full = true;
                Ok(())
            }
            IntegratedPresentationUpdate::Full(frame) => {
                if self.frozen {
                    return Err(IntegratedPresentationError::new(
                        "presentation sink is frozen",
                    ));
                }
                self.awaiting_full = false;
                if let Some(hwnd) = self.hwnd {
                    native::render_hwnd(hwnd, frame, "full")
                } else {
                    let output = self.current_output()?;
                    native::render(&output, frame, "full")
                }
            }
            IntegratedPresentationUpdate::Patch { frame, .. } => {
                if self.frozen {
                    return Err(IntegratedPresentationError::new(
                        "presentation sink is frozen",
                    ));
                }
                if self.awaiting_full {
                    return Err(IntegratedPresentationError::new(
                        "patch received after Thaw; next update must be Full",
                    ));
                }
                if let Some(hwnd) = self.hwnd {
                    native::render_hwnd(hwnd, frame, "patch")
                } else {
                    let output = self.current_output()?;
                    native::render(&output, frame, "patch")
                }
            }
            IntegratedPresentationUpdate::Clear(_) => {
                self.frozen = false;
                self.awaiting_full = false;
                self.sequence = self.sequence.saturating_add(1);
                Ok(())
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct IntegratedGraphicsSink;

#[cfg(not(windows))]
impl IntegratedGraphicsSink {
    pub(crate) fn new(_: impl Into<PathBuf>) -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl IntegratedPresentationSink for IntegratedGraphicsSink {
    fn present(
        &mut self,
        _: IntegratedPresentationUpdate<'_>,
    ) -> Result<(), IntegratedPresentationError> {
        Err(IntegratedPresentationError::new(
            "integrated graphics sink requires Windows native APIs",
        ))
    }
}
