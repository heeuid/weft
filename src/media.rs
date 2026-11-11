//! Viewing non-text files in the editor: images (png, jpg, gif, webp, bmp,
//! ico) and PDF (hayro, pure Rust). The bytes come through the agent like any
//! file (nothing is stored on the remote side) and are rendered here.

use crate::theme;
use egui::{pos2, vec2, Color32, CornerRadius, RichText, ScrollArea, Ui};

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

fn ext(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_image(path: &str) -> bool {
    IMAGE_EXT.contains(&ext(path).as_str())
}

pub fn is_pdf(path: &str) -> bool {
    ext(path) == "pdf"
}

/// How much a read may bring back: text 8 MB, images and PDFs 64 MB.
pub fn read_limit(path: &str) -> u64 {
    if is_image(path) || is_pdf(path) {
        64 * 1024 * 1024
    } else {
        8 * 1024 * 1024
    }
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

type Rendered = (usize, u32, u32, Vec<u8>);

/// A PDF being viewed: the parsed document, its page sizes, and the pages
/// rendered so far (each at the pixel width it was last needed at).
#[derive(Default)]
pub struct PdfState {
    doc: Option<std::sync::Arc<hayro::hayro_syntax::Pdf>>,
    sizes: Vec<(f32, f32)>,
    error: Option<String>,
    from_len: usize,
    pages: std::collections::HashMap<usize, (u32, egui::TextureHandle)>,
    pending: std::collections::HashSet<usize>,
    done: std::sync::Arc<std::sync::Mutex<Vec<Rendered>>>,
    /// Relative to fit-width (1.0).
    zoom: f32,
}

/// Page `i` as premultiplied RGBA on white, `width` pixels wide.
fn render_page(pdf: &hayro::hayro_syntax::Pdf, i: usize, width: u32) -> Option<(u32, u32, Vec<u8>)> {
    let page = pdf.pages().get(i)?;
    let s = width as f32 / page.render_dimensions().0.max(1.0);
    // RenderCache is not Send: one per render, on the worker thread.
    let cache = hayro::RenderCache::new();
    let px = hayro::render(
        page,
        &cache,
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings { x_scale: s, y_scale: s, bg_color: hayro::vello_cpu::color::palette::css::WHITE },
    );
    Some((px.width() as u32, px.height() as u32, px.data_as_u8_slice().to_vec()))
}

/// All pages one under another; visible pages are rendered in the
/// background at the size they are shown, and again after zooming.
pub fn pdf_view(ui: &mut Ui, st: &mut PdfState, path: &str, bytes: &[u8]) {
    if st.from_len != bytes.len() || (st.doc.is_none() && st.error.is_none()) {
        *st = PdfState { from_len: bytes.len(), zoom: 1.0, ..Default::default() };
        match hayro::hayro_syntax::Pdf::new(bytes.to_vec()) {
            Ok(pdf) => {
                st.sizes = pdf.pages().iter().map(|p| p.render_dimensions()).collect();
                st.doc = Some(std::sync::Arc::new(pdf));
            }
            Err(e) => st.error = Some(format!("{e:?}")),
        }
    }
    if let Some(e) = &st.error {
        ui.label(RichText::new(format!("can't open this PDF: {e}")).color(theme::error()));
        return;
    }
    let Some(doc) = st.doc.clone() else { return };
    // Finished renders become textures.
    let done: Vec<Rendered> = std::mem::take(&mut *st.done.lock().unwrap());
    for (i, w, h, rgba) in done {
        st.pending.remove(&i);
        let img = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], &rgba);
        let tex = ui.ctx().load_texture(format!("pdf:{path}:{i}"), img, egui::TextureOptions::LINEAR);
        st.pages.insert(i, (w, tex));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} pages", st.sizes.len())).size(12.0).color(theme::text_dim()));
        if ui.selectable_label(st.zoom == 1.0, "Fit width").clicked() {
            st.zoom = 1.0;
        }
        if ui.button("\u{2212}").clicked() {
            st.zoom = (st.zoom / 1.2).max(0.2);
        }
        if ui.button("+").clicked() {
            st.zoom = (st.zoom * 1.2).min(6.0);
        }
        ui.label(RichText::new(format!("{:.0}% \u{00b7} {}", st.zoom * 100.0, crate::widgets::human_size(bytes.len() as u64))).size(11.5).color(theme::text_faint()));
    });
    let ppp = ui.ctx().pixels_per_point();
    let fit = (ui.available_width() - 24.0).max(100.0);
    let mut wanted: Vec<(usize, u32)> = Vec::new();
    ScrollArea::both().id_salt(("pdf", path)).auto_shrink([false, false]).show(ui, |ui| {
        for (i, &(pw, ph)) in st.sizes.iter().enumerate() {
            let w = fit * st.zoom;
            let size = vec2(w, w * ph / pw.max(1.0));
            let row_w = ui.available_width().max(size.x + 16.0);
            let (row, _) = ui.allocate_exact_size(vec2(row_w, size.y + 22.0), egui::Sense::hover());
            let page = egui::Rect::from_min_size(pos2(row.center().x - size.x / 2.0, row.top() + 4.0), size);
            if !ui.is_rect_visible(row) {
                continue;
            }
            let p = ui.painter();
            p.rect_filled(page.translate(vec2(2.0, 2.0)), CornerRadius::same(2), Color32::from_black_alpha(60));
            p.rect_filled(page, CornerRadius::same(2), Color32::WHITE);
            let want = ((size.x * ppp).round() as u32).clamp(64, 6000);
            match st.pages.get(&i) {
                Some((have, tex)) => {
                    p.image(tex.id(), page, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    // Sharpen after zooming (the old picture stays meanwhile).
                    if (*have as f32 - want as f32).abs() / want as f32 > 0.15 {
                        wanted.push((i, want));
                    }
                }
                None => {
                    p.text(page.center(), egui::Align2::CENTER_CENTER, format!("page {}\u{2026}", i + 1), egui::FontId::proportional(13.0), Color32::from_gray(140));
                    wanted.push((i, want));
                }
            }
            p.text(pos2(row.center().x, page.bottom() + 10.0), egui::Align2::CENTER_CENTER, format!("{} / {}", i + 1, st.sizes.len()), egui::FontId::proportional(11.0), theme::text_faint());
        }
    });
    // At most two pages render at a time.
    for (i, width) in wanted {
        if st.pending.len() >= 2 || st.pending.contains(&i) {
            continue;
        }
        st.pending.insert(i);
        let (doc, done, ctx) = (doc.clone(), st.done.clone(), ui.ctx().clone());
        std::thread::spawn(move || {
            if let Some((w, h, rgba)) = render_page(&doc, i, width) {
                done.lock().unwrap().push((i, w, h, rgba));
            }
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_pdf_page() {
        // A one-page PDF: a black square on A4, written by hand.
        let content = b"0 0 0 rg 100 600 200 200 re f";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), String::from_utf8_lossy(content)),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (n, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{}\nendobj\n", n + 1, o).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            pdf.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        let doc = hayro::hayro_syntax::Pdf::new(pdf).expect("parse");
        assert_eq!(doc.pages().len(), 1);
        let (w, h, rgba) = render_page(&doc, 0, 595).unwrap();
        assert_eq!((w, h), (595, 842));
        let px = |x: u32, y: u32| &rgba[((y * w + x) * 4) as usize..][..4];
        assert_eq!(px(10, 10), [255, 255, 255, 255], "white background");
        assert_eq!(px(200, 842 - 700), [0, 0, 0, 255], "the square, PDF y up");
        assert!(render_page(&doc, 5, 100).is_none());
    }

    #[test]
    fn image_extensions() {
        assert!(is_image("a/b/Photo.JPG"));
        assert!(is_image("x.webp"));
        assert!(!is_image("x.rs"));
        assert!(!is_image("noext"));
        assert!(is_pdf("Report.PDF"));
        assert_eq!(read_limit("a.pdf"), 64 * 1024 * 1024);
        assert_eq!(read_limit("a.rs"), 8 * 1024 * 1024);
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
