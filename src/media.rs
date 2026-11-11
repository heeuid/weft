//! Viewing non-text files in the editor: images (png, jpg, gif, webp, bmp,
//! ico). The bytes come through the agent like any file (nothing is stored on
//! the remote side) and are decoded here into a texture once.

use crate::theme;
use egui::{pos2, vec2, Color32, CornerRadius, RichText, ScrollArea, Ui};

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

pub fn is_image(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    IMAGE_EXT.contains(&ext.as_str())
}

#[derive(Default)]
pub struct ImageState {
    tex: Option<egui::TextureHandle>,
    /// Original size in pixels.
    size: [usize; 2],
    error: Option<String>,
    /// None: fit the panel; Some(z): z image pixels per point.
    zoom: Option<f32>,
    /// The bytes the texture was made from (reload replaces them).
    from_len: usize,
}

/// GPUs cap texture sides; bigger images are shown downscaled.
const MAX_SIDE: u32 = 8192;

fn decode(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Result<(egui::TextureHandle, [usize; 2]), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let size = [img.width() as usize, img.height() as usize];
    let img = if img.width() > MAX_SIDE || img.height() > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
    Ok((ctx.load_texture(name, color, egui::TextureOptions::LINEAR), size))
}

/// Toolbar (fit, 1:1, zoom) and the image, centered, scrollable when larger.
pub fn image_view(ui: &mut Ui, st: &mut ImageState, path: &str, bytes: &[u8]) {
    if st.tex.is_none() && st.error.is_none() || st.from_len != bytes.len() {
        st.from_len = bytes.len();
        match decode(ui.ctx(), &format!("image:{path}"), bytes) {
            Ok((tex, size)) => {
                st.tex = Some(tex);
                st.size = size;
                st.error = None;
            }
            Err(e) => {
                st.tex = None;
                st.error = Some(e);
            }
        }
    }
    if let Some(e) = &st.error {
        ui.label(RichText::new(format!("can't show this image: {e}")).color(theme::error()));
        return;
    }
    let Some(tex) = st.tex.clone() else { return };
    let [w, h] = st.size;
    let avail = ui.available_size() - vec2(0.0, 30.0);
    let fit = (avail.x / w as f32).min(avail.y / h as f32).min(1.0);
    let scale = st.zoom.unwrap_or(fit);
    ui.horizontal(|ui| {
        if ui.selectable_label(st.zoom.is_none(), "Fit").on_hover_text("fit the panel").clicked() {
            st.zoom = None;
        }
        if ui.selectable_label(st.zoom == Some(1.0), "1:1").on_hover_text("actual pixels").clicked() {
            st.zoom = Some(1.0);
        }
        if ui.button("\u{2212}").clicked() {
            st.zoom = Some((scale / 1.25).max(0.02));
        }
        if ui.button("+").clicked() {
            st.zoom = Some((scale * 1.25).min(32.0));
        }
        ui.label(RichText::new(format!("{w} \u{00d7} {h} \u{00b7} {:.0}% \u{00b7} {}", scale * 100.0, crate::widgets::human_size(bytes.len() as u64))).size(11.5).color(theme::text_faint()));
    });
    let shown = vec2(w as f32, h as f32) * scale;
    ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let area = vec2(ui.available_width().max(shown.x), ui.available_height().max(shown.y));
        let (rect, _) = ui.allocate_exact_size(area, egui::Sense::hover());
        let img = egui::Rect::from_center_size(rect.center(), shown);
        // A checkerboard under transparent pixels.
        let painter = ui.painter_at(rect);
        let cell = 10.0;
        let (light, dark) = if theme::is_light() { (Color32::from_gray(245), Color32::from_gray(225)) } else { (Color32::from_gray(58), Color32::from_gray(44)) };
        painter.rect_filled(img, CornerRadius::ZERO, light);
        let visible = img.intersect(ui.clip_rect());
        let (x0, y0) = (((visible.left() - img.left()) / cell).floor() as i32, ((visible.top() - img.top()) / cell).floor() as i32);
        let (x1, y1) = (((visible.right() - img.left()) / cell).ceil() as i32, ((visible.bottom() - img.top()) / cell).ceil() as i32);
        for cy in y0.max(0)..y1 {
            for cx in x0.max(0)..x1 {
                if (cx + cy) % 2 == 1 {
                    let r = egui::Rect::from_min_size(pos2(img.left() + cx as f32 * cell, img.top() + cy as f32 * cell), vec2(cell, cell)).intersect(img);
                    painter.rect_filled(r, CornerRadius::ZERO, dark);
                }
            }
        }
        painter.image(tex.id(), img, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extensions() {
        assert!(is_image("a/b/Photo.JPG"));
        assert!(is_image("x.webp"));
        assert!(!is_image("x.rs"));
        assert!(!is_image("noext"));
    }

    #[test]
    fn decodes_the_supported_formats() {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 128]));
        for fmt in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::Bmp, image::ImageFormat::Gif, image::ImageFormat::WebP] {
            let mut buf = std::io::Cursor::new(Vec::new());
            let dynimg = if fmt == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()) } else { image::DynamicImage::ImageRgba8(img.clone()) };
            dynimg.write_to(&mut buf, fmt).unwrap_or_else(|e| panic!("{fmt:?}: {e}"));
            let back = image::load_from_memory(buf.get_ref()).unwrap_or_else(|e| panic!("{fmt:?}: {e}"));
            assert_eq!((back.width(), back.height()), (3, 2), "{fmt:?}");
        }
    }
}
