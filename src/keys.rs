//! Keyboard shortcuts: each command has default keys (ui_panels::COMMANDS);
//! settings "keys" replaces them per command, by its id:
//! `"keys": { "new_tab": ["Ctrl+T"], "zoom": [] }` (an empty list: no key).

use egui::{Key, Modifiers};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    /// "Ctrl+Shift+T", "Alt+Left", "F1", "Ctrl+=", "Ctrl+Plus" (any case).
    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut sc = Shortcut { ctrl: false, shift: false, alt: false, key: Key::A };
        let mut rest = s.trim();
        loop {
            let lower = rest.to_ascii_lowercase();
            // "Ctrl++" is Ctrl and the + key: stop when only the key is left.
            let take = |p: &str| lower.starts_with(p) && rest.len() > p.len();
            if take("ctrl+") {
                sc.ctrl = true;
                rest = &rest[5..];
            } else if take("shift+") {
                sc.shift = true;
                rest = &rest[6..];
            } else if take("alt+") {
                sc.alt = true;
                rest = &rest[4..];
            } else {
                break;
            }
        }
        // Names in any case ("pageup", "SPACE", "t"), or egui's own spellings ("=", "ArrowUp").
        sc.key = Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(rest)).or_else(|| Key::from_name(rest))?;
        Some(sc)
    }

    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(match self.key {
            Key::Equals => "=",
            Key::Minus => "-",
            Key::Plus => "Plus",
            k => k.name(),
        });
        s
    }

    fn modifiers(&self) -> u8 {
        self.ctrl as u8 + self.shift as u8 + self.alt as u8
    }

    /// A key press with these modifiers. Shift must match too, except for
    /// symbol keys that need Shift to type on some layouts (Ctrl+Plus is
    /// Ctrl+Shift+= on a US keyboard).
    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        let ctrl = m.ctrl || m.command;
        key == self.key && ctrl == self.ctrl && m.alt == self.alt && (m.shift == self.shift || (!self.shift && shifted_symbol(key)))
    }
}

fn shifted_symbol(key: Key) -> bool {
    matches!(
        key,
        Key::Plus | Key::Colon | Key::Pipe | Key::Questionmark | Key::Exclamationmark | Key::OpenCurlyBracket | Key::CloseCurlyBracket
    )
}

/// The command bound to a key press: of the shortcuts that match, the one
/// with the most modifiers (Ctrl+Shift+Tab before Ctrl+Tab).
pub fn lookup<T: Copy>(map: &[(T, Vec<Shortcut>)], key: Key, m: Modifiers, usable: impl Fn(T) -> bool) -> Option<T> {
    map.iter()
        .filter(|(c, _)| usable(*c))
        .flat_map(|(c, keys)| keys.iter().filter(|s| s.matches(key, m)).map(move |s| (s.modifiers(), *c)))
        .max_by_key(|(n, _)| *n)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_label() {
        let p = |s| Shortcut::parse(s).map(|k| k.label());
        assert_eq!(p("Ctrl+Shift+T").as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(p("ctrl+shift+t").as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(p("Alt+Left").as_deref(), Some("Alt+Left"));
        assert_eq!(p("F1").as_deref(), Some("F1"));
        assert_eq!(p("Ctrl+=").as_deref(), Some("Ctrl+="));
        assert_eq!(p("Ctrl++").as_deref(), Some("Ctrl+Plus"));
        assert_eq!(p("Ctrl+Space").as_deref(), Some("Ctrl+Space"));
        assert_eq!(p("ctrl+pageup").as_deref(), Some("Ctrl+PageUp"));
        assert_eq!(p("Ctrl+NoSuchKey"), None);
        assert_eq!(p(""), None);
    }

    #[test]
    fn exact_modifiers_most_specific_first() {
        let k = |s: &str| Shortcut::parse(s).unwrap();
        let map = [(1, vec![k("Ctrl+Tab")]), (2, vec![k("Ctrl+Shift+Tab")]), (3, vec![k("Ctrl+=") , k("Ctrl+Plus")])];
        let ctrl = Modifiers::CTRL;
        let cs = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(lookup(&map, Key::Tab, ctrl, |_| true), Some(1));
        assert_eq!(lookup(&map, Key::Tab, cs, |_| true), Some(2));
        assert_eq!(lookup(&map, Key::Tab, Modifiers::NONE, |_| true), None);
        // + typed with Shift still counts as Ctrl+Plus; = with Shift does not.
        assert_eq!(lookup(&map, Key::Plus, cs, |_| true), Some(3));
        assert_eq!(lookup(&map, Key::Equals, cs, |_| true), None);
        assert_eq!(lookup(&map, Key::Tab, ctrl, |c| c != 1), None);
    }
}
