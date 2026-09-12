//! The Windows-only paint layer consumes the client projection and its cell grid.
use super::{Bounds, Scene};
use std::{error::Error, path::Path};
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
fn color(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}
fn text(
    target: &ID2D1RenderTarget,
    font: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
    value: &str,
    bounds: Bounds,
) {
    unsafe {
        target.DrawText(
            &value.encode_utf16().collect::<Vec<_>>(),
            font,
            &rect(bounds),
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        )
    }
}
fn panel(target: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, b: Bounds, radius: f32) {
    unsafe {
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(b),
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        )
    }
}

pub(in crate::client::shell) fn render(scene: &Scene, output: &Path) -> Result<(), Box<dyn Error>> {
    // Wire colors use the 0x02_RR_GG_BB tag, not an alpha byte.
    // This first grid adapter supports the fixture subset, not every VT feature.
    let frame = &scene.frame;
    if frame.width > 78
        || frame.height > 25
        || !frame.graphics.is_empty()
        || frame.cells.iter().any(|c| {
            !c.symbol.is_ascii()
                || c.modifier != 0
                || c.skip
                || c.hyperlink.is_some()
                || c.fg >> 24 != 0x02
                || c.bg >> 24 != 0x02
        })
    {
        return Err("terminal surface outside probe subset (ASCII/RGB/plain, 78x25)".into());
    }
    raster::render(output, 1280, scene.height, |target, write| {
        unsafe {
            let ink = target.CreateSolidColorBrush(&color(0.13, 0.18, 0.17), None)?;
            let muted = target.CreateSolidColorBrush(&color(0.34, 0.40, 0.38), None)?;
            let paper = target.CreateSolidColorBrush(&color(0.99, 0.99, 0.97), None)?;
            let child = target.CreateSolidColorBrush(&color(0.93, 0.95, 0.92), None)?;
            let accent = target.CreateSolidColorBrush(&color(0.19, 0.37, 0.32), None)?;
            let title = write.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                22.0,
                w!("pt-BR"),
            )?;
            let body = write.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                15.0,
                w!("pt-BR"),
            )?;
            let mono = write.CreateTextFormat(
                w!("Consolas"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                16.0,
                w!("en-US"),
            )?;
            text(
                target,
                &title,
                &ink,
                "Sistema Argos",
                Bounds(32.0, 22.0, 420.0, 58.0),
            );
            text(
                target,
                &body,
                &muted,
                "P1 experimental · dados sintéticos · fora de tela",
                Bounds(460.0, 26.0, 1240.0, 60.0),
            );
            for card in &scene.cards {
                let is_worker = card.row.visual_kind
                    == crate::client::shell::organization::OrganizationVisualKind::Worker;
                panel(
                    target,
                    if is_worker { &child } else { &paper },
                    card.bounds,
                    18.0,
                );
                if card.row.selected {
                    panel(
                        target,
                        &accent,
                        Bounds(
                            card.bounds.0,
                            card.bounds.1 + 16.0,
                            card.bounds.0 + 5.0,
                            card.bounds.3 - 16.0,
                        ),
                        2.0,
                    );
                }
                let b = card.bounds;
                text(
                    target,
                    &title,
                    &ink,
                    &card.row.label,
                    Bounds(b.0 + 18.0, b.1 + 12.0, b.2 - 18.0, b.1 + 43.0),
                );
                text(
                    target,
                    &body,
                    &muted,
                    &card.status_label,
                    Bounds(b.0 + 18.0, b.1 + 46.0, b.2 - 18.0, b.3 - 8.0),
                );
            }
            if !scene.reserve_keys.is_empty() {
                panel(target, &child, scene.toggle, 12.0);
                text(
                    target,
                    &body,
                    &ink,
                    if scene.reserve_open {
                        "− Reserva"
                    } else {
                        "+ Reserva"
                    },
                    Bounds(338.0, 96.0, 410.0, 122.0),
                );
            }
            panel(
                target,
                &paper,
                Bounds(452.0, 76.0, 1248.0, scene.height as f32 - 40.0),
                20.0,
            );
            text(
                target,
                &title,
                &ink,
                &format!("Sessão selecionada · {}", scene.selected_label),
                Bounds(478.0, 100.0, 1220.0, 140.0),
            );
            text(
                target,
                &body,
                &muted,
                "Fixture de PaneSurfaceFrame · nenhuma sessão viva",
                Bounds(478.0, 148.0, 1220.0, 174.0),
            );
            let x0 = 478.0;
            let y0 = 204.0;
            let cw = 9.5;
            let ch = 22.0;
            // Adjacent fractional cell rectangles must not blend their edges into seams.
            target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            let mut brushes = std::collections::HashMap::<u32, ID2D1SolidColorBrush>::new();
            for (i, cell) in frame.cells.iter().enumerate() {
                let x = x0 + (i % usize::from(frame.width)) as f32 * cw;
                let y = y0 + (i / usize::from(frame.width)) as f32 * ch;
                for packed in [cell.bg, cell.fg] {
                    if let std::collections::hash_map::Entry::Vacant(entry) = brushes.entry(packed)
                    {
                        entry.insert(target.CreateSolidColorBrush(
                            &color(
                                ((packed >> 16) & 255) as f32 / 255.0,
                                ((packed >> 8) & 255) as f32 / 255.0,
                                (packed & 255) as f32 / 255.0,
                            ),
                            None,
                        )?);
                    }
                }
                let b = Bounds(x, y, x + cw, y + ch);
                target.FillRectangle(&rect(b), &brushes[&cell.bg]);
                text(target, &mono, &brushes[&cell.fg], &cell.symbol, b);
            }
            if let Some(cursor) = frame.cursor.as_ref().filter(|c| c.visible) {
                let x = x0 + cursor.x as f32 * cw;
                let y = y0 + cursor.y as f32 * ch;
                target.FillRectangle(&rect(Bounds(x, y + ch - 2.0, x + cw, y + ch)), &accent);
            }
            text(
                target,
                &body,
                &muted,
                "Grade terminal preservada separadamente dos cards. Sem chat paralelo.",
                Bounds(
                    478.0,
                    scene.height as f32 - 100.0,
                    1220.0,
                    scene.height as f32 - 60.0,
                ),
            );
        }
        Ok(())
    })
}
