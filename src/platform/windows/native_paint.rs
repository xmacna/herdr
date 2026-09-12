//! Direct2D/DirectWrite presentation. Consumes the existing organization projection and cell buffer.
use super::{
    scene::{Bounds, Scene},
    Result, View,
};
use std::path::Path;
use windows::{
    core::w,
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
};
#[path = "offscreen_probe.rs"]
mod raster;
fn rect(b: Bounds) -> D2D_RECT_F {
    D2D_RECT_F {
        left: b.0,
        top: b.1,
        right: b.2,
        bottom: b.3,
    }
}
fn rgba(rgb: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.,
        g: ((rgb >> 8) & 255) as f32 / 255.,
        b: (rgb & 255) as f32 / 255.,
        a: 1.,
    }
}
unsafe fn brush(t: &ID2D1RenderTarget, c: u32) -> windows::core::Result<ID2D1SolidColorBrush> {
    t.CreateSolidColorBrush(&rgba(c), None)
}
unsafe fn text(
    t: &ID2D1RenderTarget,
    f: &IDWriteTextFormat,
    b: &ID2D1SolidColorBrush,
    s: &str,
    r: Bounds,
) {
    t.DrawText(
        &s.encode_utf16().collect::<Vec<_>>(),
        f,
        &rect(r),
        b,
        D2D1_DRAW_TEXT_OPTIONS_CLIP,
        DWRITE_MEASURING_MODE_NATURAL,
    );
}
unsafe fn panel(t: &ID2D1RenderTarget, c: u32, b: Bounds, r: f32) -> Result<()> {
    t.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: rect(b),
            radiusX: r,
            radiusY: r,
        },
        &brush(t, c)?,
    );
    Ok(())
}
fn rgb(c: ratatui::style::Color, foreground: bool) -> u32 {
    use ratatui::style::Color::*;
    let index = match c {
        Rgb(r, g, b) => return (r as u32) << 16 | (g as u32) << 8 | b as u32,
        Reset => return if foreground { 0xd8d8da } else { 0x191b1f },
        Indexed(i) => i,
        Black => 0,
        Red => 1,
        Green => 2,
        Yellow => 3,
        Blue => 4,
        Magenta => 5,
        Cyan => 6,
        Gray => 7,
        DarkGray => 8,
        LightRed => 9,
        LightGreen => 10,
        LightYellow => 11,
        LightBlue => 12,
        LightMagenta => 13,
        LightCyan => 14,
        White => 15,
    };
    let c = crate::ghostty::default_palette()[index as usize];
    (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}
pub(super) fn capture(scene: &Scene, output: &Path) -> Result<()> {
    raster::render(output, 1280, 900, |t, w| {
        draw(t, w, Some(scene), 0., false, None, 900., false)
    })
}
pub(super) fn view(t: &ID2D1RenderTarget, w: &IDWriteFactory, v: &View, height: f32) -> Result<()> {
    draw(
        t,
        w,
        v.scene.as_ref(),
        v.scroll,
        v.terminal_only,
        v.error.as_deref(),
        height,
        v.input.is_some(),
    )
}
fn draw(
    t: &ID2D1RenderTarget,
    w: &IDWriteFactory,
    scene: Option<&Scene>,
    scroll: f32,
    terminal_only: bool,
    error: Option<&str>,
    height: f32,
    operational: bool,
) -> Result<()> {
    unsafe {
        t.Clear(Some(&rgba(0xecebe7)));
        t.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let ink = brush(t, 0x25262a)?;
        let muted = brush(t, 0x65686d)?;
        let title = w.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            21.,
            w!("pt-BR"),
        )?;
        let body = w.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            14.,
            w!("pt-BR"),
        )?;
        text(
            t,
            &title,
            &ink,
            "Sistema Argos",
            Bounds(32., 20., 430., 56.),
        );
        text(
            t,
            &body,
            &muted,
            if operational {
                "Experimental · Pro isolado"
            } else {
                "Experimental · observação"
            },
            Bounds(460., 26., 950., 57.),
        );
        panel(t, 0xfdfcfa, Bounds(1010., 16., 1248., 58.), 12.)?;
        text(
            t,
            &body,
            &ink,
            if terminal_only {
                "Escritório  ·  F6"
            } else {
                "Expandir terminal  ·  F6"
            },
            Bounds(1026., 26., 1238., 53.),
        );
        let Some(scene) = scene else {
            text(
                t,
                &title,
                &ink,
                error.unwrap_or("Conectando à sessão selecionada…"),
                Bounds(48., 110., 1220., 200.),
            );
            return Ok(());
        };
        if !terminal_only {
            t.PushAxisAlignedClip(
                &rect(Bounds(16., 70., 440., height - 32.)),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            for card in &scene.cards {
                let b = card.bounds;
                let b = Bounds(b.0, b.1 - scroll, b.2, b.3 - scroll);
                if b.3 < 70. || b.1 > height - 32. {
                    continue;
                }
                let child = card.row.visual_kind
                    == crate::client::shell::organization::OrganizationVisualKind::Worker;
                if !child {
                    panel(t, 0xdadad5, Bounds(b.0, b.1 + 3., b.2, b.3 + 3.), 22.)?;
                }
                panel(
                    t,
                    if child { 0xf2f1ed } else { 0xfdfcfa },
                    b,
                    if child { 12. } else { 22. },
                )?;
                t.DrawLine(
                    windows_numerics::Vector2 {
                        X: b.0 + 20.,
                        Y: b.1 + 1.,
                    },
                    windows_numerics::Vector2 {
                        X: b.2 - 20.,
                        Y: b.1 + 1.,
                    },
                    &brush(t, 0xffffff)?,
                    1.,
                    None,
                );
                if card.row.selected {
                    t.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: rect(b),
                            radiusX: if child { 12. } else { 22. },
                            radiusY: if child { 12. } else { 22. },
                        },
                        &ink,
                        1.2,
                        None,
                    );
                }
                let marker = match card.own_status {
                    crate::api::schema::AgentStatus::Working => 0x496c65,
                    crate::api::schema::AgentStatus::Blocked => 0x8b754b,
                    _ => 0x878b86,
                };
                t.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 {
                            X: b.0 + 22.,
                            Y: b.1 + 28.,
                        },
                        radiusX: 4.,
                        radiusY: 4.,
                    },
                    &brush(t, marker)?,
                );
                // Name and state come from Herdr's existing localized organization projection.
                text(
                    t,
                    &title,
                    &ink,
                    &card.row.label,
                    Bounds(b.0 + 36., b.1 + 12., b.2 - 18., b.1 + 43.),
                );
                text(
                    t,
                    &body,
                    &muted,
                    &card.status_label,
                    Bounds(b.0 + 18., b.1 + 49., b.2 - 18., b.3 - 9.),
                );
            }
            if !scene.reserve_keys.is_empty() {
                let b = scene.toggle;
                let b = Bounds(b.0, b.1 - scroll, b.2, b.3 - scroll);
                panel(t, 0xe9e8e3, b, 10.)?;
                text(
                    t,
                    &body,
                    &ink,
                    if scene.reserve_open {
                        "− Reserva"
                    } else {
                        "+ Reserva"
                    },
                    Bounds(b.0 + 6., b.1 + 6., b.2, b.3),
                );
            }
            t.PopAxisAlignedClip();
        }
        let left = if terminal_only { 32. } else { 452. };
        // Entire observed grid fits locally. Resizing this view never resizes the server PTY.
        let f = &scene.frame;
        let available_w = 1248. - left - 48.;
        let available_h = (height - 260.).max(100.);
        let cw = (available_w / f.width.max(1) as f32)
            .min(available_h / f.height.max(1) as f32 / 1.85)
            .min(11.);
        let ch = cw * 1.85;
        let panel_bottom = (188. + ch * f.height as f32 + 74.).min(height - 34.);
        panel(
            t,
            0xdadad5,
            Bounds(left, 79., 1248., panel_bottom + 3.),
            22.,
        )?;
        panel(t, 0xfdfcfa, Bounds(left, 76., 1248., panel_bottom), 22.)?;
        text(
            t,
            &title,
            &ink,
            &scene.selected_label,
            Bounds(left + 24., 99., 1224., 139.),
        );
        text(
            t,
            &body,
            &muted,
            if operational {
                "Terminal Pro · entrada habilitada somente no ambiente isolado"
            } else {
                "Terminal real · somente leitura neste trecho"
            },
            Bounds(left + 24., 144., 1224., 169.),
        );
        let font_size = (ch * 0.9).max(1.);
        let fonts = [
            (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL),
            (DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL),
            (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_ITALIC),
            (DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_ITALIC),
        ]
        .into_iter()
        .map(|(weight, style)| {
            w.CreateTextFormat(
                w!("Consolas"),
                None,
                weight,
                style,
                DWRITE_FONT_STRETCH_NORMAL,
                font_size,
                w!("en-US"),
            )
        })
        .collect::<windows::core::Result<Vec<_>>>()?;
        for font in &fonts {
            font.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            font.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, ch, ch * 0.8)?;
        }
        let buffer = f.to_ratatui_buffer().ok_or("invalid cell frame")?;
        let mut brushes = std::collections::HashMap::new();
        t.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        for cell in &buffer.content {
            for c in [rgb(cell.fg, true), rgb(cell.bg, false)] {
                if let std::collections::hash_map::Entry::Vacant(e) = brushes.entry(c) {
                    e.insert(brush(t, c)?);
                }
            }
        }
        let x0 = left + 24.;
        let y0 = 188.;
        // Paint all backgrounds before text so a wide grapheme is not erased by its spacer.
        for (i, c) in buffer.content.iter().enumerate() {
            let x = x0 + (i % f.width as usize) as f32 * cw;
            let y = y0 + (i / f.width as usize) as f32 * ch;
            t.FillRectangle(
                &rect(Bounds(x, y, x + cw, y + ch)),
                &brushes[&rgb(c.bg, false)],
            );
        }
        for (i, c) in buffer.content.iter().enumerate() {
            if c.skip || c.symbol().is_empty() {
                continue;
            }
            let x = x0 + (i % f.width as usize) as f32 * cw;
            let y = y0 + (i / f.width as usize) as f32 * ch;
            let m = c.modifier;
            let fi = usize::from(m.contains(ratatui::style::Modifier::BOLD))
                + 2 * usize::from(m.contains(ratatui::style::Modifier::ITALIC));
            let width = unicode_width::UnicodeWidthStr::width(c.symbol()).max(1) as f32 * cw;
            let color = &brushes[&rgb(c.fg, true)];
            text(
                t,
                &fonts[fi],
                color,
                c.symbol(),
                Bounds(x, y, x + width, y + ch),
            );
            if m.contains(ratatui::style::Modifier::UNDERLINED) {
                t.DrawLine(
                    windows_numerics::Vector2 {
                        X: x,
                        Y: y + ch - 2.,
                    },
                    windows_numerics::Vector2 {
                        X: x + width,
                        Y: y + ch - 2.,
                    },
                    color,
                    1.,
                    None,
                );
            }
            if m.contains(ratatui::style::Modifier::CROSSED_OUT) {
                t.DrawLine(
                    windows_numerics::Vector2 {
                        X: x,
                        Y: y + ch * 0.55,
                    },
                    windows_numerics::Vector2 {
                        X: x + width,
                        Y: y + ch * 0.55,
                    },
                    color,
                    1.,
                    None,
                );
            }
        }
        if let Some(c) = f.cursor.as_ref().filter(|c| c.visible) {
            let x = x0 + c.x as f32 * cw;
            let y = y0 + c.y as f32 * ch;
            t.DrawRectangle(
                &rect(Bounds(x, y, x + cw, y + ch)),
                &brush(t, 0xbbc7c3)?,
                1.,
                None,
            );
        }
        t.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        text(
            t,
            &body,
            &muted,
            error.unwrap_or(if operational {
                "Teclado e Ctrl+V · terminal isolado · nenhuma saída escondida"
            } else {
                "Seleção local · sem envio de teclas · nenhuma saída foi escondida"
            }),
            Bounds(left + 24., panel_bottom - 58., 1224., panel_bottom - 10.),
        );
        text(
            t,
            &body,
            &muted,
            "Role para ver a equipe",
            Bounds(32., height - 27., 430., height - 4.),
        );
    }
    Ok(())
}
