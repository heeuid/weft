//! weft without its window: the terminal grid and its parser, ptys, hosts
//! and the operations they serve (this PC directly, a remote one through
//! the agent), searching, the command records, and passwords in memory.
//! Nothing here draws; the app (the root package) and weft-draw do.

pub mod editfind;
pub mod find;
pub mod host;
pub mod pty;
pub mod records;
pub mod secret;
pub mod term;

/// Text as it is to be drawn: each combining mark put together with the
/// letter before it where Unicode has the two as one. macOS hands out file
/// names decomposed: Hangul as its separate letters ("응" as ㅇ ㅡ ㅇ), "が"
/// as か and a mark, an accent apart from its letter, which a font draws
/// one by one. Nothing else changes: unlike a full NFC, a character that is
/// already one (a CJK compatibility ideograph, an Ångström sign) stays the
/// character it is. Only for showing: the name itself (to open or rename
/// the file with) is left as the system gave it.
pub fn shown(s: &str) -> std::borrow::Cow<'_, str> {
    let mut out = String::with_capacity(s.len());
    let mut pending: Option<char> = None;
    let mut joined_any = false;
    for c in s.chars() {
        match pending.and_then(|p| unicode_normalization::char::compose(p, c)) {
            Some(j) => {
                pending = Some(j);
                joined_any = true;
            }
            None => {
                if let Some(p) = pending {
                    out.push(p);
                }
                pending = Some(c);
            }
        }
    }
    if !joined_any {
        return std::borrow::Cow::Borrowed(s);
    }
    if let Some(p) = pending {
        out.push(p);
    }
    std::borrow::Cow::Owned(out)
}

/// `shown` for text drawn a character at a time: each character as it is
/// to be drawn, and which of the given ones it stands for (a letter and the
/// marks put together with it), so marks that go with the given characters
/// (the matched ones of a search) still go with the right ones.
pub fn shown_chars(chars: &[char]) -> Vec<(char, std::ops::Range<usize>)> {
    let mut out: Vec<(char, std::ops::Range<usize>)> = Vec::with_capacity(chars.len());
    for (k, &c) in chars.iter().enumerate() {
        if let Some(joined) = out.last().and_then(|(prev, _)| unicode_normalization::char::compose(*prev, c)) {
            let last = out.last_mut().unwrap();
            last.0 = joined;
            last.1.end = k + 1;
        } else {
            out.push((c, k..k + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// Text that is already as it is to be drawn is left exactly as it is,
    /// whatever the script: only marks after their letters are put
    /// together. (A full NFC would also have turned compatibility
    /// characters, e.g. the hanja of Korean names at U+F900, into others.)
    #[test]
    fn composed_text_of_every_script_is_left_alone() {
        let samples = [
            "한글 파일 이름.txt",
            "日本語のファイル がぎぐ パピプ",
            "中文文件名 繁體字",
            "العربية ملف",
            "עברית קובץ",
            "हिन्दी फ़ाइल",
            "ไฟล์ภาษาไทย",
            "Tiếng Việt: tệp ổ đĩa",
            "Ελληνικά Кириллица Ññ Åå",
            "emoji 👍🏽 👨‍👩‍👧 ❤️ 🇰🇷",
            "\u{F900}\u{F92D} hanja compatibility, \u{212B} angstrom, \u{2126} ohm",
            "ascii only, tabs\tand\nnewlines",
        ];
        for s in samples {
            assert!(matches!(super::shown(s), std::borrow::Cow::Borrowed(t) if t == s), "{s:?} is left as it is: {:?}", super::shown(s));
            let chars: Vec<char> = s.chars().collect();
            let drawn = super::shown_chars(&chars);
            assert_eq!(drawn.iter().map(|d| d.0).collect::<String>(), s, "a character at a time too");
        }
    }

    #[test]
    fn decomposed_text_is_shown_composed() {
        // macOS's "응용프로그램": each syllable as its letters.
        let nfd: String = unicode_normalization::UnicodeNormalization::nfd("응용프로그램").collect();
        assert_eq!(nfd.chars().count(), 15);
        assert_eq!(super::shown(&nfd), "응용프로그램");
        assert_eq!(super::shown("e\u{301}t\u{e9}"), "\u{e9}t\u{e9}");
        // Already composed: the same string, not a copy.
        assert!(matches!(super::shown("응용 abc"), std::borrow::Cow::Borrowed(_)));
        // Japanese as a Mac writes it: が as か and its mark.
        let ja: String = unicode_normalization::UnicodeNormalization::nfd("がぎぐパピ").collect();
        assert_eq!(super::shown(&ja), "がぎぐパピ");
        // A character at a time: which given ones each drawn one stands for.
        let chars: Vec<char> = nfd.chars().collect();
        let drawn = super::shown_chars(&chars);
        assert_eq!(drawn.iter().map(|d| d.0).collect::<String>(), "응용프로그램");
        assert_eq!((drawn[0].1.clone(), drawn[1].1.clone(), drawn[5].1.clone()), (0..3, 3..6, 12..15));
        assert_eq!(super::shown_chars(&['a', 'b']), [('a', 0..1), ('b', 1..2)]);
    }
}
