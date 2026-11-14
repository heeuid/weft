//! What a frame looks like, in pixels, without a window or a GPU: egui's
//! shapes are tessellated and the triangles filled here, with the font
//! atlas and the images egui hands out as textures. For tests: they can
//! look at colours (a logo's eyes, a selection), and save a PNG to be
//! looked at.
//!
//! It blends as egui's own backends do (premultiplied colours, in gamma
//! space), without their finer points: no subpixel fill rule, so edges
//! shared by two translucent triangles are a shade darker.

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, ColorImage, ImageData, TextureId, TexturesDelta};
use std::collections::HashMap;

/// The textures egui has handed out so far (the font atlas, images).
#[derive(Default)]
pub struct Textures {
    images: HashMap<TextureId, ColorImage>,
}

impl Textures {
    /// Take what a frame changed (every frame's: the font atlas grows in
    /// pieces).
    pub fn update(&mut self, delta: &TexturesDelta) {
        for (id, change) in &delta.set {
            let new = match &change.image {
                ImageData::Color(c) => (**c).clone(),
                ImageData::Font(f) => ColorImage { size: f.size, pixels: f.srgba_pixels(None).collect() },
            };
            match (change.pos, self.images.get_mut(id)) {
                (Some([x, y]), Some(whole)) => {
                    for row in 0..new.size[1] {
                        for col in 0..new.size[0] {
                            if x + col < whole.size[0] && y + row < whole.size[1] {
                                whole.pixels[(y + row) * whole.size[0] + x + col] = new.pixels[row * new.size[0] + col];
                            }
                        }
                    }
                }
                _ => {
                    self.images.insert(*id, new);
                }
            }
        }
        for id in &delta.free {
            self.images.remove(id);
        }
    }

    /// The colour of a texture at `uv` (0..1), between its nearest pixels.
    fn sample(&self, id: TextureId, u: f32, v: f32) -> [f32; 4] {
        let Some(img) = self.images.get(&id) else { return [1.0; 4] };
        let (w, h) = (img.size[0], img.size[1]);
        if w == 0 || h == 0 {
            return [1.0; 4];
        }
        let (x, y) = ((u * w as f32 - 0.5).max(0.0), (v * h as f32 - 0.5).max(0.0));
        let (x0, y0) = ((x as usize).min(w - 1), (y as usize).min(h - 1));
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let px = |x: usize, y: usize| rgba(img.pixels[y * w + x]);
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t);
        mix(mix(px(x0, y0), px(x1, y0), fx), mix(px(x0, y1), px(x1, y1), fx), fy)
    }

    /// The frame's primitives as an image of `width` by `height` pixels.
    pub fn render(&self, primitives: &[ClippedPrimitive], pixels_per_point: f32, width: usize, height: usize) -> image::RgbaImage {
        let mut out = vec![[0.0_f32, 0.0, 0.0, 1.0]; width * height];
        for ClippedPrimitive { clip_rect, primitive } in primitives {
            let Primitive::Mesh(mesh) = primitive else { continue };
            let clip = [
                (clip_rect.min.x * pixels_per_point).round().max(0.0) as usize,
                (clip_rect.min.y * pixels_per_point).round().max(0.0) as usize,
                ((clip_rect.max.x * pixels_per_point).round().max(0.0) as usize).min(width),
                ((clip_rect.max.y * pixels_per_point).round().max(0.0) as usize).min(height),
            ];
            for tri in mesh.indices.chunks_exact(3) {
                let mut v = [0, 1, 2].map(|i| &mesh.vertices[tri[i] as usize]);
                let at = |v: &egui::epaint::Vertex| (v.pos.x * pixels_per_point, v.pos.y * pixels_per_point);
                // How far `q` is to one side of the edge from `a` to `b`.
                let side = |a: (f32, f32), b: (f32, f32), q: (f32, f32)| (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
                // All triangles the same way round, so an edge two of them
                // share runs opposite ways in each.
                if side(at(v[0]), at(v[1]), at(v[2])) < 0.0 {
                    v.swap(1, 2);
                }
                let p = v.map(at);
                let area = side(p[0], p[1], p[2]);
                if area < 1e-6 {
                    continue;
                }
                // A pixel exactly on an edge belongs to one of the two
                // triangles sharing it (else a see-through fill is laid on
                // twice along its diagonal).
                let owns = |a: (f32, f32), b: (f32, f32)| b.1 - a.1 > 0.0 || (b.1 == a.1 && b.0 < a.0);
                let inside = |a: (f32, f32), b: (f32, f32), q: (f32, f32)| {
                    let e = side(a, b, q);
                    e > 0.0 || (e == 0.0 && owns(a, b))
                };
                let x0 = (p.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize).max(clip[0]);
                let y0 = (p.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize).max(clip[1]);
                let x1 = (p.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(clip[2]);
                let y1 = (p.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(clip[3]);
                let colors = v.map(|v| rgba(v.color));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let q = (x as f32 + 0.5, y as f32 + 0.5);
                        if !(inside(p[1], p[2], q) && inside(p[2], p[0], q) && inside(p[0], p[1], q)) {
                            continue;
                        }
                        // How much of each corner the pixel is.
                        let w0 = side(p[1], p[2], q) / area;
                        let w1 = side(p[2], p[0], q) / area;
                        let w2 = 1.0 - w0 - w1;
                        let (u, tv) = (w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x, w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y);
                        let texel = self.sample(mesh.texture_id, u, tv);
                        let src = [0, 1, 2, 3].map(|i| (w0 * colors[0][i] + w1 * colors[1][i] + w2 * colors[2][i]) * texel[i]);
                        let dst = &mut out[y * width + x];
                        for i in 0..4 {
                            dst[i] = src[i] + dst[i] * (1.0 - src[3]);
                        }
                    }
                }
            }
        }
        let mut img = image::RgbaImage::new(width as u32, height as u32);
        for (i, px) in out.iter().enumerate() {
            let c = px.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            img.put_pixel((i % width) as u32, (i / width) as u32, image::Rgba([c[0], c[1], c[2], 255]));
        }
        img
    }
}

/// A premultiplied colour as four numbers from 0 to 1.
fn rgba(c: Color32) -> [f32; 4] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two filled rects and a line of text come out where they were put,
    /// in their colours: the renderer the screen tests look through.
    #[test]
    fn shapes_come_out_as_pixels() {
        let ctx = egui::Context::default();
        let mut tex = Textures::default();
        let mut shapes = Vec::new();
        let mut ppp = 1.0;
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(120.0, 60.0))), ..Default::default() };
            let out = ctx.run(input, |ctx| {
                let p = ctx.layer_painter(egui::LayerId::background());
                p.rect_filled(egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(20.0, 20.0)), egui::CornerRadius::ZERO, Color32::from_rgb(200, 0, 0));
                p.rect_filled(egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(20.0, 20.0)), egui::CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0, 0, 255, 128));
                p.text(egui::pos2(60.0, 10.0), egui::Align2::LEFT_TOP, "MMM", egui::FontId::monospace(20.0), Color32::WHITE);
            });
            tex.update(&out.textures_delta);
            shapes = out.shapes;
            ppp = out.pixels_per_point;
        }
        let img = tex.render(&ctx.tessellate(shapes, ppp), ppp, 120, 60);
        let at = |x: u32, y: u32| img.get_pixel(x, y).0;
        assert_eq!(at(15, 15), [200, 0, 0, 255], "the red rect");
        assert_eq!(at(5, 5), [0, 0, 0, 255], "nothing drawn there");
        let both = at(25, 25);
        // (egui premultiplies in linear light: half of 255 is 188, not 128.)
        assert!((95..=105).contains(&both[0]) && both[1] == 0 && (183..=193).contains(&both[2]), "blue, half see-through, over red, laid on once: {both:?}");
        let blue = at(35, 35);
        assert!(blue[0] == 0 && (183..=193).contains(&blue[2]), "and over nothing: {blue:?}");
        // The letters: white ink somewhere in their box, none beside it.
        let ink = |x0: u32, x1: u32| (x0..x1).flat_map(|x| (8..34).map(move |y| (x, y))).filter(|&(x, y)| at(x, y)[0] > 200).count();
        assert!(ink(60, 100) > 40, "the text is drawn: {} white pixels", ink(60, 100));
        assert_eq!(ink(104, 120), 0, "and only there");
    }
}
