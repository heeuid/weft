//! Glyphs as wide as they are drawn, in any terminal font.
//!
//! "Mono" builds of Nerd Fonts (and some other terminal fonts) give every
//! glyph a one-cell advance, Hangul and CJK included, though those glyphs are
//! drawn two cells wide. The terminal grid places each cell itself and doesn't
//! care, but the editor and markdown lay text out as runs, where the next
//! glyph starts at the advance: there such text overlapped. This reads how far
//! each glyph's ink reaches in the font file and, where that is past its
//! advance, makes the advance that many whole cells (half a cell drawn: left
//! alone; 1.5 cells: 2; 2.5 cells: 3), so text stays on the column grid.

/// The font with every glyph's advance covering its ink, or None when nothing needs it
/// (or it isn't a single TrueType/OpenType font this can rewrite).
pub fn fit_advances_to_ink(data: &[u8]) -> Option<Vec<u8>> {
    if data.get(..4)? == b"ttcf" {
        return None;
    }
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let cell = face.glyph_hor_advance(face.glyph_index('M')?)?;
    let n = face.number_of_glyphs() as usize;
    let mut metrics: Vec<(u16, i16)> = (0..n as u16)
        .map(|g| {
            let g = ttf_parser::GlyphId(g);
            (face.glyph_hor_advance(g).unwrap_or(0), face.glyph_hor_side_bearing(g).unwrap_or(0))
        })
        .collect();
    let mut changed = false;
    for sub in face.tables().cmap?.subtables {
        if !sub.is_unicode() {
            continue;
        }
        sub.codepoints(|cp| {
            // ASCII is laid out in runs on the terminal grid too; box drawing
            // and powerline glyphs overhang on purpose, to join their neighbours.
            if cp < 0x80 || (0x2500..=0x259F).contains(&cp) || (0xE0A0..=0xE0D7).contains(&cp) {
                return;
            }
            let Some(g) = sub.glyph_index(cp) else { return };
            let Some(ink) = face.glyph_bounding_box(g) else { return };
            let m = &mut metrics[g.0 as usize];
            // A tenth of a cell over is just an overhang.
            if ink.x_max as i32 > m.0 as i32 + cell as i32 / 10 {
                let cells = (ink.x_max as u32).div_ceil(cell as u32).max(1);
                m.0 = (cells * cell as u32).min(u16::MAX as u32) as u16;
                changed = true;
            }
        });
    }
    if !changed {
        return None;
    }
    let mut hmtx = Vec::with_capacity(n * 4);
    for (adv, lsb) in metrics {
        hmtx.extend(adv.to_be_bytes());
        hmtx.extend(lsb.to_be_bytes());
    }
    let mut hhea = table(data, b"hhea")?.to_vec();
    hhea.get_mut(34..36)?.copy_from_slice(&(n as u16).to_be_bytes());
    rebuild(data, &[(*b"hhea", hhea), (*b"hmtx", hmtx)])
}

/// The bytes of table `tag`.
fn table<'a>(data: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    records(data)?.into_iter().find(|r| &r.0 == tag).and_then(|(_, off, len)| data.get(off..off + len))
}

/// (tag, offset, length) of each table.
fn records(data: &[u8]) -> Option<Vec<([u8; 4], usize, usize)>> {
    let num = u16::from_be_bytes(data.get(4..6)?.try_into().ok()?) as usize;
    (0..num)
        .map(|i| {
            let r = data.get(12 + i * 16..28 + i * 16)?;
            let be = |a: usize| u32::from_be_bytes(r[a..a + 4].try_into().unwrap()) as usize;
            Some((r[..4].try_into().ok()?, be(8), be(12)))
        })
        .collect()
}

/// The font again, with some tables replaced; each table 4-byte aligned,
/// with its checksum.
fn rebuild(data: &[u8], replace: &[([u8; 4], Vec<u8>)]) -> Option<Vec<u8>> {
    let recs = records(data)?;
    let mut out = data[..12].to_vec();
    out.resize(12 + recs.len() * 16, 0);
    for (i, (tag, off, len)) in recs.iter().enumerate() {
        let body = match replace.iter().find(|r| &r.0 == tag) {
            Some((_, b)) => b.as_slice(),
            None => data.get(*off..off + len)?,
        };
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let at = out.len();
        out.extend_from_slice(body);
        let mut padded = body.to_vec();
        padded.resize(body.len().div_ceil(4) * 4, 0);
        let sum = padded.chunks(4).fold(0u32, |s, c| s.wrapping_add(u32::from_be_bytes(c.try_into().unwrap())));
        let rec = 12 + i * 16;
        out[rec..rec + 4].copy_from_slice(tag);
        out[rec + 4..rec + 8].copy_from_slice(&sum.to_be_bytes());
        out[rec + 8..rec + 12].copy_from_slice(&(at as u32).to_be_bytes());
        out[rec + 12..rec + 16].copy_from_slice(&(body.len() as u32).to_be_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_build_gets_two_cell_hangul() {
        // The "Mono" D2Coding build, if downloaded (target/fontdl).
        let Ok(data) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/fontdl/d2/D2CodingLigatureNerdFontMono-Regular.ttf")) else { return };
        let fixed = fit_advances_to_ink(&data).expect("Mono build should be widened");
        let face = ttf_parser::Face::parse(&fixed, 0).unwrap();
        let adv = |c| face.glyph_hor_advance(face.glyph_index(c).unwrap()).unwrap();
        assert_eq!(adv('기'), 2 * adv('M'));
        assert_eq!(adv('a'), adv('M'));
        assert_eq!(adv('\u{e0b0}'), adv('M'), "powerline stays one cell");
    }

    #[test]
    fn advances_follow_the_ink() {
        // Hack draws everything inside its advances: left alone.
        let hack = crate::theme::BUILTIN_FONTS.iter().find(|b| b.0 == "builtin:hack").unwrap().2;
        assert!(fit_advances_to_ink(hack).is_none());
        // D2Coding's Nerd icons are drawn up to two cells on a one-cell advance.
        let d2 = crate::theme::BUILTIN_FONTS.iter().find(|b| b.0 == "builtin:d2coding").unwrap().2;
        let fixed = fit_advances_to_ink(d2).unwrap();
        let face = ttf_parser::Face::parse(&fixed, 0).unwrap();
        let adv = |c| face.glyph_hor_advance(face.glyph_index(c).unwrap()).unwrap();
        let cell = adv('M');
        assert_eq!(adv('a'), cell);
        assert_eq!(adv('기'), 2 * cell);
        assert_eq!(adv('\u{e000}'), 2 * cell, "icon drawn 1.5 cells wide");
        assert_eq!(adv('\u{e0b0}'), cell, "powerline overhangs on purpose");
        assert_eq!(adv('\u{2500}'), cell, "box drawing overhangs on purpose");
    }
}
