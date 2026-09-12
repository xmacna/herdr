//! Native, software-rendered WIC surface: no HWND, socket, PTY, or client loop.
use std::{error::Error, fs::File, io::BufWriter, path::Path, time::Instant};
use windows::{
    core::w,
    Win32::{
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Imaging::*,
        },
        System::{Com::*, ProcessStatus::*, Threading::GetCurrentProcess},
    },
};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 400;

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        // All COM interfaces are dropped before this guard, on the same thread.
        unsafe { CoUninitialize() };
    }
}

fn color(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

pub fn run(output: &Path) -> Result<(), Box<dyn Error>> {
    if !output.is_absolute() {
        return Err("output must be an absolute path, outside production state".into());
    }
    let started = Instant::now();
    // This process is a finite example; no message pump or window is created.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let _apartment = ComApartment;
    let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
    let write: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    let bitmap = unsafe {
        wic.CreateBitmap(
            WIDTH,
            HEIGHT,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )?
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
    let target = unsafe { d2d.CreateWicBitmapRenderTarget(&bitmap, &properties)? };
    let ink = unsafe { target.CreateSolidColorBrush(&color(0.14, 0.18, 0.18), None)? };
    let paper = unsafe { target.CreateSolidColorBrush(&color(0.99, 0.99, 0.97), None)? };
    let shade = unsafe { target.CreateSolidColorBrush(&color(0.80, 0.82, 0.80), None)? };
    let accent = unsafe { target.CreateSolidColorBrush(&color(0.22, 0.38, 0.34), None)? };
    let title = unsafe {
        write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            32.0,
            w!("pt-BR"),
        )?
    };
    let body = unsafe {
        write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            18.0,
            w!("pt-BR"),
        )?
    };
    let init_ms = started.elapsed().as_secs_f64() * 1000.0;
    let draw_started = Instant::now();
    unsafe {
        target.BeginDraw();
        target.Clear(Some(&color(0.91, 0.92, 0.90)));
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(63.0, 67.0, 743.0, 343.0),
                radiusX: 22.0,
                radiusY: 22.0,
            },
            &shade,
        );
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(60.0, 60.0, 740.0, 336.0),
                radiusX: 22.0,
                radiusY: 22.0,
            },
            &paper,
        );
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(92.0, 95.0, 100.0, 151.0),
                radiusX: 4.0,
                radiusY: 4.0,
            },
            &accent,
        );
        target.DrawText(
            &"Sistema Argos".encode_utf16().collect::<Vec<_>>(),
            &title,
            &rect(120.0, 99.0, 680.0, 150.0),
            &ink,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
        target.DrawText(
            &"Prova técnica fora de tela"
                .encode_utf16()
                .collect::<Vec<_>>(),
            &body,
            &rect(92.0, 179.0, 690.0, 219.0),
            &ink,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
        target.DrawText(&"Direct2D + DirectWrite · card e texto nativos\nSem janela, sessão, input ou aceite visual.".encode_utf16().collect::<Vec<_>>(), &body, &rect(92.0, 235.0, 690.0, 312.0), &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        // Draw operations report failures here; do not export a failed surface.
        target.EndDraw(None, None)?;
    }
    let draw_ms = draw_started.elapsed().as_secs_f64() * 1000.0;
    let read_started = Instant::now();
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    unsafe { bitmap.CopyPixels(std::ptr::null(), WIDTH * 4, &mut pixels)? };
    // Clear is opaque across the entire surface, so PBGRA -> RGBA is lossless.
    if pixels.chunks_exact(4).any(|p| p[3] != 255) {
        return Err("unexpected non-opaque pixels; premultiplied data not exported".into());
    }
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let background = pixels[..4].to_vec();
    let changed_pixels = pixels
        .chunks_exact(4)
        .filter(|p| *p != background.as_slice())
        .count();
    let dark_pixels = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 100 && p[1] < 130 && p[2] < 130)
        .count();
    if changed_pixels < 1000 || dark_pixels < 500 {
        return Err("rendered surface lacks expected card/text contrast".into());
    }
    let readback_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    let encode_started = Instant::now();
    // PNG only encodes pixels produced by the native APIs; it draws nothing.
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(output)?), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    writer.finish()?;
    let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut memory,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )?
    };
    let report = serde_json::json!({
        "api": "Direct2D + DirectWrite -> WIC bitmap; png crate only encodes",
        "render_target": "D2D1_RENDER_TARGET_TYPE_SOFTWARE",
        "width": WIDTH, "height": HEIGHT, "dpi": 96,
        "native_end_draw_succeeded": true, "opaque_pixels": true,
        "changed_pixels": changed_pixels, "dark_pixels": dark_pixels,
        "init_ms": init_ms, "draw_end_draw_ms": draw_ms,
        "readback_ms": readback_ms, "png_encode_ms": encode_ms,
        "total_ms": started.elapsed().as_secs_f64() * 1000.0,
        "working_set_bytes": memory.WorkingSetSize, "peak_working_set_bytes": memory.PeakWorkingSetSize,
        "window_created": false, "runtime_connected": false,
        "gpu_measured": false, "input_tested": false, "m0_complete": false,
        "output": output,
    });
    std::fs::write(
        output.with_extension("json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", report);
    Ok(())
}
