//! Characters none of weft's fonts has (kana, hanzi, Thai, Arabic, …: any
//! text a terminal or a file shows): looked for in the fonts the system has
//! installed, wherever it keeps them (fontdb knows each system's folders,
//! and fontconfig's on Linux), the face that has one loaded then and put
//! after weft's own in every family. A character no installed font has
//! stays a box: no terminal can draw it.
//!
//! The search runs on a thread of its own (the system's fonts are read the
//! first time one is wanted, and a face's characters by its cmap only, the
//! file mapped, not read whole); what it finds is put in at the next frame
//! (`poll`).

use egui::{FontData, FontDefinitions, FontFamily, FontId};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// A face found for a character, its bytes read for egui.
struct Face {
    key: String,
    data: Vec<u8>,
    index: u32,
}

#[derive(Default)]
struct State {
    /// The families as installed, faces found so far added: set again as
    /// each comes.
    defs: Option<FontDefinitions>,
    /// Faces found so far, kept for an install of other fonts (a font set
    /// again in the settings).
    faces: Vec<Face>,
    /// Characters looked at already (drawable or asked for).
    seen: HashSet<char>,
    tx: Option<Sender<char>>,
    rx: Option<Receiver<Option<Face>>>,
}

/// Each egui context's own (a window has one; tests make many).
fn with<T>(ctx: &egui::Context, f: impl FnOnce(&mut State) -> T) -> T {
    let st: Arc<Mutex<State>> = ctx.data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("weft-font-fallback"), || Arc::new(Mutex::new(State::default()))).clone());
    let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// The fonts just installed (`theme::install_fonts`): those found before
/// go after them again, and every character is looked at anew.
pub(crate) fn installed(ctx: &egui::Context, defs: &FontDefinitions) {
    let defs = with(ctx, |s| {
        let mut defs = defs.clone();
        for f in &s.faces {
            add(&mut defs, f);
        }
        s.defs = Some(defs.clone());
        s.seen.clear();
        defs
    });
    ctx.set_fonts(defs);
}

/// The families text is drawn in, each to have a found face last.
fn families(defs: &FontDefinitions) -> Vec<FontFamily> {
    defs.families.keys().cloned().collect()
}

fn add(defs: &mut FontDefinitions, f: &Face) {
    let mut d = FontData::from_owned(f.data.clone());
    d.index = f.index;
    defs.font_data.insert(f.key.clone(), Arc::new(d));
    for fam in families(defs) {
        let list = defs.families.entry(fam).or_default();
        if !list.contains(&f.key) {
            list.push(f.key.clone());
        }
    }
}

/// Text about to be drawn in `font`: any character of it the fonts lack is
/// looked for (once: a frame looks again only at characters not seen).
pub fn want(ctx: &egui::Context, font: &FontId, text: &str) {
    let new: Vec<char> = with(ctx, |s| {
        if s.defs.is_none() {
            return Vec::new();
        }
        text.chars().filter(|c| !c.is_ascii() && !c.is_control() && !s.seen.contains(c)).collect::<HashSet<_>>().into_iter().collect()
    });
    if new.is_empty() {
        return;
    }
    let missing: Vec<char> = ctx.fonts(|f| new.iter().copied().filter(|c| !f.has_glyph(font, *c)).collect());
    with(ctx, |s| {
        s.seen.extend(new.iter().copied());
        if missing.is_empty() {
            return;
        }
        if s.tx.is_none() {
            let (tx, rx) = start();
            s.tx = Some(tx);
            s.rx = Some(rx);
        }
        if let Some(tx) = &s.tx {
            for c in missing {
                let _ = tx.send(c);
            }
        }
    });
}

/// Faces found since the last frame: put in, the next frame drawn with
/// them. Once a frame.
pub fn poll(ctx: &egui::Context) {
    let defs = with(ctx, |s| {
        let mut got = Vec::new();
        if let Some(rx) = &s.rx {
            while let Ok(r) = rx.try_recv() {
                got.extend(r);
            }
        }
        if got.is_empty() {
            return None;
        }
        let defs = s.defs.as_mut()?;
        for f in &got {
            add(defs, f);
        }
        let out = defs.clone();
        s.faces.extend(got);
        // Looked at again with them: some were asked for before any came.
        s.seen.clear();
        Some(out)
    });
    if let Some(defs) = defs {
        ctx.set_fonts(defs);
        ctx.request_repaint();
    }
}

/// The search: the system's fonts listed once, each character asked for
/// looked for in the faces found so far first, then in all of them.
fn start() -> (Sender<char>, Receiver<Option<Face>>) {
    let (tx, rx_c) = channel::<char>();
    let (tx_f, rx) = channel::<Option<Face>>();
    std::thread::Builder::new()
        .name("weft-font-fallback".into())
        .spawn(move || {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let order = order(&db);
            let mut found: Vec<fontdb::ID> = Vec::new();
            let mut none: HashSet<char> = HashSet::new();
            while let Ok(c) = rx_c.recv() {
                if none.contains(&c) || found.iter().any(|id| has(&db, *id, c)) {
                    continue;
                }
                match order.iter().find(|id| has(&db, **id, c)) {
                    Some(id) => {
                        found.push(*id);
                        let face = db.with_face_data(*id, |data, index| Face { key: format!("fallback-{}", found.len()), data: data.to_vec(), index });
                        if tx_f.send(face).is_err() {
                            return;
                        }
                    }
                    None => {
                        none.insert(c);
                    }
                }
            }
        })
        .expect("a thread");
    (tx, rx)
}

/// Whether any font the system has installed has the character (as the
/// search would take it: outlines). Reads the system's fonts: for tests.
pub fn system_has(c: char) -> bool {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    order(&db).iter().any(|id| has(&db, *id, c))
}

/// The faces in the order they are tried: upright, regular weight first,
/// then by name (the same face each run).
fn order(db: &fontdb::Database) -> Vec<fontdb::ID> {
    let mut faces: Vec<&fontdb::FaceInfo> = db.faces().collect();
    faces.sort_by_key(|f| {
        let name = f.families.first().map(|n| n.0.to_lowercase()).unwrap_or_default();
        (f.style != fontdb::Style::Normal, (f.weight.0 as i32 - 400).abs(), name, f.index)
    });
    faces.into_iter().map(|f| f.id).collect()
}

/// Whether a face has the character, drawn as outlines (glyf or CFF: egui
/// draws no colour bitmaps, Apple Color Emoji's).
fn has(db: &fontdb::Database, id: fontdb::ID, c: char) -> bool {
    db.with_face_data(id, |data, index| {
        ttf_parser::Face::parse(data, index).ok().is_some_and(|f| {
            let t = f.tables();
            (t.glyf.is_some() || t.cff.is_some()) && f.glyph_index(c).is_some_and(|g| g.0 != 0)
        })
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character weft's fonts lack is found in a font the system has and
    /// drawn after that; one no font has is asked for once and stays a box.
    #[test]
    fn a_character_the_fonts_lack_is_found_in_the_systems() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let _ = ctx.run(Default::default(), |_| {});
        let font = crate::theme::term_font(14.0);
        // The first installed face with a character none of weft's has.
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let candidates = ['あ', '中', 'ก', 'ע', 'ب', 'Ж', 'ሀ', 'ᚠ', '∰'];
        let lacking: Vec<char> = ctx.fonts(|f| candidates.iter().copied().filter(|c| !f.has_glyph(&font, *c)).collect());
        let Some(c) = lacking.into_iter().find(|c| order(&db).iter().any(|id| has(&db, *id, *c))) else {
            eprintln!("no character here that weft lacks and the system has");
            return;
        };
        want(&ctx, &font, &c.to_string());
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut drawn = false;
        while std::time::Instant::now() < end && !drawn {
            poll(&ctx);
            let _ = ctx.run(Default::default(), |_| {});
            drawn = ctx.fonts(|f| f.has_glyph(&font, c));
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(drawn, "{c:?} found in an installed font and drawn");
        // A private-use character no font has: asked once, a box.
        want(&ctx, &font, "\u{10FFFD}");
        assert!(with(&ctx, |s| s.seen.contains(&'\u{10FFFD}')));
    }
}
