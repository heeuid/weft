//! Where weft keeps what it keeps, in two files of its data folder:
//!
//! - settings.json: the settings a person sets (theme, fonts, hosts,
//!   keys…), written only when one changes, and then only the ones that
//!   changed, over the file as it is on disk; read again within a second
//!   when it changes on disk (another editor, weft's own), and what
//!   changed said. Not valid JSON (being edited, a typo): weft goes on
//!   with the settings it has and writes nothing to it until it is fixed.
//! - state.json: what weft keeps on its own as it runs (the tabs, files
//!   and folders open, the panels, the searches' modes), written by weft
//!   alone, as often as it changes.
//!
//! Both are written whole to a file beside and moved over the old one, so
//! a crash or a full disk never leaves half a file. settings.json from
//! before had both: its weft part moves to state.json as weft starts (the
//! old file kept as settings.json.bak).

use crate::app::{App, Settings, ToastKind};
use crate::host::DataDir;
use serde_json::{Map, Value};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

/// The names of Settings kept in state.json, not settings.json.
pub const STATE_KEYS: [&str; 6] = ["session", "open", "side_tabs", "layout", "search", "sidebar"];

/// Written whole beside it, then moved over it: never half a file.
/// (The file beside it is this weft's own, so that two wefts writing at
/// once never hand each other half a file; moving it over may fail for a
/// moment where another program has the file open, so it is tried again.)
pub fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("json.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    let mut tries = 0;
    loop {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(_) if tries < 5 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                // Held open by a program that lets others write but not
                // move it over (a C or Python program reading it): written
                // where it is, the file as it was kept beside first.
                return write_in_place(path, text).map_err(|_| e);
            }
        }
    }
}

/// The file written where it is (not moved over): its text before kept
/// as <name>.prev, since a crash in the middle leaves half of it.
fn write_in_place(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Ok(before) = std::fs::read(path) {
        std::fs::write(path.with_extension("json.prev"), before)?;
    }
    let mut f = std::fs::OpenOptions::new().write(true).truncate(true).create(true).open(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()
}

/// The names that differ between two sets of settings: changed, added,
/// or gone (an empty value is left out of the file, so gone is a change).
pub fn differing(a: &Map<String, Value>, b: Option<&Map<String, Value>>) -> Vec<String> {
    let Some(b) = b else { return a.keys().cloned().collect() };
    let mut keys: Vec<String> = a.iter().filter(|(k, v)| b.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
    keys.extend(b.keys().filter(|k| !a.contains_key(*k)).cloned());
    keys
}

/// `mine`'s value for each of `keys` put in `file` (gone from `mine`:
/// gone). The hosts and their shells go one by one against `base` (what
/// weft last saw): a host added or changed in weft is put, one it forgot
/// taken out, and the others are the file's as they are (one added
/// outside meanwhile, or by another weft, stays).
pub fn put(file: &mut Map<String, Value>, mine: &Map<String, Value>, base: Option<&Map<String, Value>>, keys: &[String]) {
    for k in keys {
        // (Left out when empty: an empty list of its kind then.)
        let empty = || if k == "profiles" { Value::Array(Vec::new()) } else { Value::Object(Map::new()) };
        match base {
            Some(b) if k == "profiles" || k == "remote_shells" => {
                let m = mine.get(k).cloned().unwrap_or_else(empty);
                let was = b.get(k).cloned().unwrap_or_else(empty);
                let f = file.get(k).cloned().unwrap_or_else(empty);
                let merged = merge_named(&m, &was, f);
                let none = merged.as_array().is_some_and(|a| a.is_empty()) || merged.as_object().is_some_and(|o| o.is_empty());
                if none && k == "remote_shells" {
                    file.remove(k);
                } else {
                    file.insert(k.clone(), merged);
                }
            }
            _ => match mine.get(k) {
                Some(v) => {
                    file.insert(k.clone(), v.clone());
                }
                None => {
                    file.remove(k);
                }
            },
        }
    }
}

/// A list of named things (an array of objects with "name", or an object
/// by name): `theirs` with what `mine` changed from `base`, name by name.
fn merge_named(mine: &Value, base: &Value, theirs: Value) -> Value {
    fn by_name(v: &Value) -> Vec<(String, Value)> {
        match v {
            Value::Array(a) => a.iter().map(|x| (x.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(), x.clone())).collect(),
            Value::Object(o) => o.iter().map(|(k, x)| (k.clone(), x.clone())).collect(),
            _ => Vec::new(),
        }
    }
    let (m, b, mut t) = (by_name(mine), by_name(base), by_name(&theirs));
    let find = |l: &[(String, Value)], n: &str| l.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    for (n, v) in &m {
        if find(&b, n).as_ref() != Some(v) {
            match t.iter_mut().find(|(k, _)| k == n) {
                Some(slot) => slot.1 = v.clone(),
                None => t.push((n.clone(), v.clone())),
            }
        }
    }
    for (n, _) in &b {
        if find(&m, n).is_none() {
            t.retain(|(k, _)| k != n);
        }
    }
    match theirs {
        Value::Object(_) => Value::Object(t.into_iter().collect()),
        _ => Value::Array(t.into_iter().map(|(_, v)| v).collect()),
    }
}

/// When a file was changed and its size: changed on disk when it differs.
pub fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn parse_object(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err("settings.json is one object: { … }".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// The settings a person sets, by name, as weft holds them.
pub fn prefs_of(s: &Settings) -> Map<String, Value> {
    let mut m = match serde_json::to_value(s) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    for k in STATE_KEYS {
        m.remove(k);
    }
    m
}

/// What weft keeps on its own, as state.json's text.
pub fn state_text(s: &Settings) -> String {
    let mut out = Map::new();
    if let Ok(Value::Object(mut m)) = serde_json::to_value(s) {
        for k in STATE_KEYS {
            if let Some(v) = m.remove(k) {
                out.insert(k.to_string(), v);
            }
        }
    }
    serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default()
}

/// The settings from settings.json's names over the defaults, weft's own
/// part from `state` (or from settings.json, as before state.json).
fn settings_from(prefs: &Map<String, Value>, state: Option<&Map<String, Value>>) -> Result<Settings, String> {
    let mut all = prefs.clone();
    if let Some(st) = state {
        for k in STATE_KEYS {
            all.remove(k);
            if let Some(v) = st.get(k) {
                all.insert(k.to_string(), v.clone());
            }
        }
    }
    serde_json::from_value(Value::Object(all)).map_err(|e| e.to_string())
}

/// What weft found as it started.
pub struct Loaded {
    /// The window this weft is (instances.rs).
    pub window: crate::instances::Claim,
    pub settings: Settings,
    /// settings.json was not valid: its text and what is wrong.
    pub bad: Option<(String, String)>,
    /// settings.json as read (empty: none, or not valid), and as weft reads it.
    pub prefs_text: String,
    pub prefs_seen: Option<Map<String, Value>>,
    /// settings.json still had weft's part (from before state.json).
    pub to_move: bool,
}

pub fn load(data: &DataDir, wanted: Option<&str>, reopen: bool) -> Loaded {
    let prefs_text = std::fs::read_to_string(data.path("settings.json")).unwrap_or_default();
    let prefs = if prefs_text.trim().is_empty() { Ok(Map::new()) } else { parse_object(&prefs_text) };
    // Which window this weft is, and which come back besides (by the
    // setting, read from the file as it is).
    let restore = prefs.as_ref().ok().and_then(|p| p.get("restore_windows")).and_then(|v| v.as_str()).unwrap_or("all").to_string();
    let window = crate::instances::claim(data, wanted, reopen, &restore);
    let state = window.state.clone();
    let state_raw: Option<()> = window.had_state.then_some(());
    let (settings, bad, prefs_seen, to_move) = match prefs.and_then(|p| settings_from(&p, state.as_ref()).map(|s| (s, p))) {
        Ok((s, p)) => {
            let to_move = state_raw.is_none() && STATE_KEYS.iter().any(|k| p.contains_key(*k));
            let seen = prefs_of(&s);
            (s, None, Some(seen), to_move)
        }
        Err(e) => {
            data.log(&format!("settings.json: {e}"));
            // weft's own part still comes back: from state.json, or (from
            // before it) from settings.json if it reads as JSON at all; the
            // file kept beside as it is (its tabs in it, if not).
            let lenient = if state.is_none() {
                let _ = std::fs::copy(data.path("settings.json"), data.path("settings.json.bak"));
                parse_object(&prefs_text).ok()
            } else {
                None
            };
            let s = settings_from(&Map::new(), state.as_ref().or(lenient.as_ref())).unwrap_or_default();
            (s, Some((prefs_text.clone(), e)), None, false)
        }
    };
    // What weft read as valid only: a file not valid is not "what weft
    // had" (undone to it, it would pass for fixed).
    let prefs_text = if bad.is_some() { String::new() } else { prefs_text };
    Loaded { window, settings, bad, prefs_text, prefs_seen, to_move }
}

/// The settings weft knows by these names, said in a few words.
fn names(keys: &[String]) -> String {
    let mut s = keys.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    if keys.len() > 4 {
        s.push_str(&format!(" and {} more", keys.len() - 4));
    }
    s
}

impl App {
    /// As weft starts on a settings.json from before state.json: its weft
    /// part moved to state.json, the old file kept as settings.json.bak.
    pub fn move_state_out(&mut self) {
        let path = self.data.path("settings.json");
        let _ = std::fs::copy(&path, self.data.path("settings.json.bak"));
        let mut prefs = parse_object(&self.prefs_written).unwrap_or_default();
        for k in STATE_KEYS {
            prefs.remove(k);
        }
        let text = serde_json::to_string_pretty(&Value::Object(prefs)).unwrap_or_default();
        // The tabs safe in state.json first; else settings.json as it was.
        if !self.save_state(true) {
            return;
        }
        if write_whole(&path, &text).is_ok() {
            self.prefs_written = text;
            self.prefs_stamp = stamp(&path);
        }
        self.data.log("settings: the session moved from settings.json to state.json (settings.json.bak kept)");
    }

    /// This window's place in state.json, when it changed (or `always`).
    /// Whether state.json has it now.
    pub fn save_state(&mut self, always: bool) -> bool {
        let text = state_text(&self.settings);
        if always || text != self.state_written {
            let place = parse_object(&text).unwrap_or_default();
            match crate::instances::save(&self.data, &self.window_id, place) {
                Ok(()) => self.state_written = text,
                Err(e) => {
                    self.data.log(&format!("state.json: {e}"));
                    return false;
                }
            }
        }
        true
    }

    /// The settings changed since weft last read or wrote settings.json,
    /// written over the file as it is now (what was changed in it outside
    /// meanwhile stays, and is taken). Not valid on disk: nothing written.
    pub fn save_prefs(&mut self) {
        if self.settings_bad.is_some() {
            return;
        }
        // (When each change here was made, up to now.)
        self.note_changed_here();
        let mine = prefs_of(&self.settings);
        let changed = differing(&mine, self.prefs_seen.as_ref());
        if changed.is_empty() {
            return;
        }
        let path = self.data.path("settings.json");
        let on_disk = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            // None (removed, or never made): made with what weft has.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.settings_take_over();
                return;
            }
            Err(_) => return,
        };
        // Changed outside since weft last saw it: taken too (below). Empty
        // or not valid there: maybe an editor saving it; left to the next
        // look (poll_prefs tells which), nothing written now.
        let outside = on_disk != self.prefs_written;
        let mut file = if !outside && on_disk.trim().is_empty() {
            Map::new()
        } else {
            match parse_object(&on_disk) {
                Ok(m) => m,
                Err(_) => {
                    self.prefs_stamp = None;
                    return;
                }
            }
        };
        // A setting changed in the file too since weft last saw it, and
        // later than here: the file's stays (the later one wins).
        let changed = if outside { self.later_here(&file, stamp(&path).map(|s| s.0), changed) } else { changed };
        put(&mut file, &mine, self.prefs_seen.as_ref(), &changed);
        // What would be written read as weft would: a value not of its
        // kind in the file said, and nothing written.
        let mut check = file.clone();
        for k in STATE_KEYS {
            check.remove(k);
        }
        if let Err(e) = settings_from(&check, None) {
            self.prefs_not_valid(on_disk, format!("a value is not of its kind: {e}"));
            return;
        }
        let text = serde_json::to_string_pretty(&Value::Object(file)).unwrap_or_default();
        if let Err(e) = write_whole(&path, &text) {
            // Said once (it is tried again at the next save).
            let said = format!("settings.json not saved: {e}");
            if self.prefs_failed.as_deref() != Some(said.as_str()) {
                self.toast(said.clone(), ToastKind::Error);
                self.prefs_failed = Some(said);
            }
            return;
        }
        self.prefs_failed = None;
        self.changed_here.clear();
        self.prefs_written = text.clone();
        self.prefs_stamp = stamp(&path);
        self.show_prefs_in_editor(&text);
        let mut kept = false;
        if outside {
            let old = self.settings.clone();
            if self.apply_settings_text(&text).is_ok() {
                self.say_changed(&old, false);
                // (A host a tab uses taken out outside: put back, below.)
                kept = self.keep_hosts_in_use(&old);
            }
        }
        // What weft saw is what it wrote (a host put back after: saved).
        let mut written = parse_object(&text).unwrap_or_default();
        for k in STATE_KEYS {
            written.remove(k);
        }
        self.prefs_seen = settings_from(&written, None).ok().map(|s| prefs_of(&s));
        if kept {
            self.save_prefs();
        }
    }

    /// settings.json as it is now, at most once a second: changed on disk
    /// (another editor, weft's own), read and applied, and what changed
    /// said; not valid, said once and left as it is.
    pub fn poll_prefs(&mut self) {
        if self.prefs_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.prefs_checked = Instant::now();
        self.note_changed_here();
        let path = self.data.path("settings.json");
        let now = stamp(&path);
        if now == self.prefs_stamp || now.is_none() {
            return;
        }
        self.prefs_stamp = now;
        let Ok(text) = std::fs::read_to_string(&path) else {
            // (Held by another program a moment: read at the next look.)
            self.prefs_stamp = None;
            return;
        };
        if text == self.prefs_written && !text.trim().is_empty() {
            // Undone back to what weft had: valid again.
            self.prefs_unsure = None;
            if self.settings_bad.take().is_some() {
                self.toast("settings.json is valid again", ToastKind::Info);
                if self.prefs_seen.is_none() {
                    self.prefs_seen = Some(prefs_of(&self.settings));
                }
            }
            return;
        }
        let file = if text.trim().is_empty() { Err("it is empty".to_string()) } else { parse_object(&text) };
        let file = match file {
            Ok(m) => m,
            // Empty or not valid: maybe an editor in the middle of saving
            // (many empty the file, then write it). Read again a second
            // later; the same then, it is so (an empty file: the defaults).
            Err(e) => {
                let settled = self.prefs_unsure.as_ref().is_some_and(|(t, at)| *t == text && at.elapsed() >= Duration::from_secs(1));
                if !settled {
                    if self.prefs_unsure.as_ref().is_none_or(|(t, _)| *t != text) {
                        self.prefs_unsure = Some((text, Instant::now()));
                    }
                    self.prefs_stamp = None;
                    return;
                }
                if text.trim().is_empty() {
                    Map::new()
                } else {
                    self.prefs_not_valid(text, e);
                    return;
                }
            }
        };
        self.prefs_unsure = None;
        let was_bad = self.settings_bad.take();
        let fixed = was_bad.is_some();
        // What weft changed and could not save yet (the file was not
        // valid): kept over the file's.
        let mine = prefs_of(&self.settings);
        // (Fixed after an error: the file's, as the window said.)
        let pending = match &self.prefs_seen {
            Some(seen) if !fixed => differing(&mine, Some(seen)),
            _ => Vec::new(),
        };
        // (A setting changed in the file later than here: the file's.)
        let pending = self.later_here(&file, now.map(|s| s.0), pending);
        let mut merged = file.clone();
        put(&mut merged, &mine, self.prefs_seen.as_ref(), &pending);
        let old = self.settings.clone();
        match self.apply_settings_text(&Value::Object(merged).to_string()) {
            Ok(notes) => {
                self.prefs_written = text.clone();
                let mut f = file;
                for k in STATE_KEYS {
                    f.remove(k);
                }
                self.prefs_seen = settings_from(&f, None).ok().map(|s| prefs_of(&s));
                // (The changes here the file's replaced: no longer pending.)
                self.note_changed_here();
                self.say_changed(&old, fixed);
                for n in notes {
                    self.toast(format!("settings.json: {n}"), ToastKind::Info);
                }
                self.show_prefs_in_editor(&text);
                let kept = self.keep_hosts_in_use(&old);
                if !pending.is_empty() || kept {
                    self.save_prefs();
                }
            }
            Err(e) => {
                // Still not valid: said only if it is a new error.
                self.settings_bad = was_bad;
                self.prefs_not_valid(text, e);
            }
        }
    }

    /// settings.json read now (before an Apply in the window: an edit
    /// outside is taken first).
    pub fn poll_prefs_now(&mut self) {
        self.prefs_checked = Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or(self.prefs_checked);
        self.prefs_stamp = None;
        self.poll_prefs();
    }

    /// When each setting weft changed and has not saved yet was changed
    /// (seen at most a second late): to tell which of it and the file's
    /// change to the same setting came later.
    fn note_changed_here(&mut self) {
        let Some(seen) = &self.prefs_seen else { return };
        let changed = differing(&prefs_of(&self.settings), Some(seen));
        self.changed_here.retain(|k, _| changed.contains(k));
        for k in changed {
            self.changed_here.entry(k).or_insert_with(SystemTime::now);
        }
    }

    /// Of `keys` (settings weft changed), those to keep over the file's:
    /// all but the ones the file changed too (from what weft last saw),
    /// later than weft did (the file's time, `file_at`). The hosts and
    /// their shells are merged one by one, not by time.
    fn later_here(&self, file: &Map<String, Value>, file_at: Option<SystemTime>, keys: Vec<String>) -> Vec<String> {
        let (Some(seen), Some(file_at)) = (&self.prefs_seen, file_at) else { return keys };
        let mut f = file.clone();
        for k in STATE_KEYS {
            f.remove(k);
        }
        let Some(theirs) = settings_from(&f, None).ok().map(|s| prefs_of(&s)) else { return keys };
        keys.into_iter()
            .filter(|k| {
                let named = k == "profiles" || k == "remote_shells";
                let file_changed = theirs.get(k) != seen.get(k);
                let here = self.changed_here.get(k).copied().unwrap_or_else(SystemTime::now);
                named || !file_changed || here >= file_at
            })
            .collect()
    }

    /// settings.json found not valid: said once (per text), left as it is.
    fn prefs_not_valid(&mut self, text: String, e: String) {
        if self.settings_bad.as_deref() != Some(text.as_str()) {
            self.toast(format!("settings.json has an error ({e}): weft goes on with the settings it has and saves none to the file until it is fixed (Settings… > Advanced)"), ToastKind::Error);
        }
        self.settings_bad = Some(text);
    }

    /// What changed in the settings, said.
    fn say_changed(&mut self, old: &Settings, fixed: bool) {
        let (a, b) = (prefs_of(old), prefs_of(&self.settings));
        let keys: Vec<String> = b.iter().filter(|(k, v)| a.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
        let what = if keys.is_empty() { String::new() } else { format!(": {}", names(&keys)) };
        let head = if fixed { "settings.json is valid again" } else { "settings.json changed" };
        if fixed || !keys.is_empty() {
            self.toast(format!("{head}{what}"), ToastKind::Info);
        }
    }

    /// settings.json as weft has it, written over the file on purpose
    /// (the file not valid, and the person chose weft's settings).
    pub fn settings_take_over(&mut self) {
        self.settings_bad = None;
        let path = self.data.path("settings.json");
        // The file as it was, kept beside.
        let _ = std::fs::copy(&path, self.data.path("settings.json.bak"));
        let text = serde_json::to_string_pretty(&Value::Object(prefs_of(&self.settings))).unwrap_or_default();
        match write_whole(&path, &text) {
            Ok(()) => {
                self.prefs_failed = None;
                self.prefs_written = text.clone();
                self.prefs_stamp = stamp(&path);
                self.prefs_seen = Some(prefs_of(&self.settings));
                self.show_prefs_in_editor(&text);
            }
            Err(e) => {
                let said = format!("settings.json not saved: {e}");
                if self.prefs_failed.as_deref() != Some(said.as_str()) {
                    self.toast(said.clone(), ToastKind::Error);
                    self.prefs_failed = Some(said);
                }
            }
        }
    }

    /// After settings were taken from settings.json: a host a tab uses
    /// that is no longer there (kept on connecting while the file was not
    /// valid, taken out outside or under Advanced) put back, with its
    /// shell, so the tab does not come back without it. Whether one was.
    pub fn keep_hosts_in_use(&mut self, old: &Settings) -> bool {
        let used: Vec<String> = self.panes.values().map(|p| p.host.clone()).collect();
        let mut kept = false;
        for p in &old.profiles {
            if used.contains(&p.name) && !self.settings.profiles.iter().any(|q| q.name == p.name) {
                self.settings.profiles.push(p.clone());
                if let Some(sh) = old.remote_shells.get(&p.name) {
                    self.settings.remote_shells.entry(p.name.clone()).or_insert_with(|| sh.clone());
                }
                kept = true;
            }
        }
        kept
    }

    /// weft's editor showing settings.json, unchanged there: what is on
    /// disk now shown (no conflict at its next save).
    fn show_prefs_in_editor(&mut self, text: &str) {
        let path = self.data.path("settings.json");
        let shown = path.display().to_string().replace('\\', "/");
        let mtime = std::fs::metadata(&path).ok().map(|m| crate::host::mtime_of(&m));
        for f in self.editor.files.iter_mut().filter(|f| f.host == "local" && f.path.replace('\\', "/") == shown && !f.dirty && !f.saving) {
            f.text = text.to_string();
            f.mtime = mtime;
        }
    }
}
