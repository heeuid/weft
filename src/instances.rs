//! weft run more than once on one data folder: each weft is a window of
//! its own, with its own place in state.json ({"windows": {id: …}}),
//! written by it alone.
//!
//! A running weft holds its window's lock (instances/<id>.lock, the
//! system's lock on the file, let go when the process ends, crashed or
//! not), so a window whose lock can be taken is one no weft has open.
//!
//! - weft starts: the windows no weft has open come back (by the
//!   "restore_windows" setting: all of them, the last one, or none); it
//!   takes the most recent itself, and the others are opened by more
//!   wefts (main starts them).
//! - weft closes while another runs: its window is gone from state.json
//!   (as a window closed in a browser); the last one to close stays, to
//!   come back. A crash or a system shut down at once leaves them all.
//! - weft restarts: the new weft is told the window (WEFT_WINDOW) and
//!   waits for the old one to let go of it.
//!
//! state.json is read and written under a lock of its own
//! (state.json.lock), so two wefts never write over each other's window.

use crate::host::DataDir;
use serde_json::{Map, Value};
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// The windows' places in state.json, by window.
pub type Windows = Map<String, Value>;

fn lock_path(data: &DataDir, id: &str) -> PathBuf {
    data.path("instances").join(format!("{id}.lock"))
}

/// The window's lock, taken now if no weft holds it. (A file system with
/// no locks, some network ones: taken, as if weft ran once.)
fn take(data: &DataDir, id: &str) -> Option<File> {
    let _ = std::fs::create_dir_all(data.path("instances"));
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path(data, id)).ok()?;
    match f.try_lock() {
        Ok(()) => Some(f),
        Err(std::fs::TryLockError::WouldBlock) => None,
        Err(std::fs::TryLockError::Error(_)) => Some(f),
    }
}


/// The window's lock, waited for (a weft restarting lets go of it as it
/// closes).
fn take_waiting(data: &DataDir, id: &str, wait: Duration) -> Option<File> {
    let end = Instant::now() + wait;
    loop {
        if let Some(f) = take(data, id) {
            return Some(f);
        }
        if Instant::now() > end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// state.json read and written by one weft at a time while it is held.
struct StateLock(#[allow(dead_code)] Option<File>);

fn state_lock(data: &DataDir) -> StateLock {
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(data.path("state.json.lock")).ok();
    if let Some(f) = &f {
        let _ = f.lock();
    }
    StateLock(f)
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A new window's name: when it was made and by which process.
fn new_id() -> String {
    let n = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}-{:x}", n, std::process::id())
}

/// The windows in state.json's text, and whether it was from before
/// windows (0.12.2: one window, its tabs at the top; it is that window).
pub fn windows_of(text: &str) -> (Windows, bool) {
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return (Map::new(), false) };
    match m.get("windows") {
        Some(Value::Object(w)) => (w.clone(), false),
        _ if crate::store::STATE_KEYS.iter().any(|k| m.contains_key(*k)) => {
            let mut w = Map::new();
            w.insert(new_id(), Value::Object(m));
            (w, true)
        }
        _ => (Map::new(), false),
    }
}

/// state.json's windows (and whether there was a state.json). Held by
/// another program a moment (an antivirus, a sync): read again a few
/// times; still not to be read, Err (nothing is written over it then).
/// Read but not JSON: kept as state.json.bad, and no windows.
fn read(data: &DataDir) -> std::io::Result<(Windows, bool, bool)> {
    let path = data.path("state.json");
    let mut tries = 0;
    loop {
        match std::fs::read_to_string(&path) {
            Ok(t) => {
                let ok = matches!(serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')), Ok(Value::Object(_)));
                if !ok && !t.trim().is_empty() {
                    if tries < 3 {
                        // (Maybe being written: read again.)
                        tries += 1;
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    let _ = std::fs::copy(&path, data.path("state.json.bad"));
                    data.log("state.json is not JSON: kept as state.json.bad, the windows in it not opened");
                }
                let (w, old) = windows_of(&t);
                return Ok((w, true, old));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Map::new(), false, false)),
            Err(e) if tries < 5 => {
                let _ = e;
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

fn write(data: &DataDir, windows: &Windows) -> std::io::Result<()> {
    let mut top = Map::new();
    top.insert("windows".into(), Value::Object(windows.clone()));
    crate::store::write_whole(&data.path("state.json"), &serde_json::to_string_pretty(&Value::Object(top)).unwrap_or_default())
}

/// The window weft starts as.
pub struct Claim {
    pub id: String,
    /// Its lock, held as long as weft runs.
    pub lock: Option<File>,
    /// Its place in state.json (none: a new window).
    pub state: Option<Map<String, Value>>,
    /// The other windows to open (more wefts, started by main).
    pub others: Vec<String>,
    /// state.json was there.
    pub had_state: bool,
    /// The window wanted was not to be had (another weft took it).
    pub missed: bool,
}

/// How long a window being opened by another weft (started for it) is
/// left to it.
const OPENING: f64 = 30.0;

/// The window this weft is: `wanted` (a restart's, or one this weft was
/// started to open), else the windows no weft has open, by `restore`
/// ("all", "last" or "none"), else a new one. `missed`: the wanted one
/// was not to be had.
pub fn claim(data: &DataDir, wanted: Option<&str>, reopen: bool, restore: &str) -> Claim {
    claim_waiting(data, wanted, reopen, restore, HELD_A_MOMENT)
}

/// How long a window that looks held is asked again (a process being
/// started holds a copy of its lock for less).
const HELD_A_MOMENT: Duration = Duration::from_millis(150);

/// `claim`, a window that looks held asked again for `held_wait`.
fn claim_waiting(data: &DataDir, wanted: Option<&str>, reopen: bool, restore: &str, held_wait: Duration) -> Claim {
    // Waited for without holding state.json (others go on saving meanwhile).
    let wanted_lock = wanted.and_then(|id| take_waiting(data, id, Duration::from_secs(10)).map(|f| (id.to_string(), f)));
    let missed = wanted.is_some() && wanted_lock.is_none();
    // Started to open a window another weft took: nothing more (it ends).
    if missed && reopen {
        return Claim { id: new_id(), lock: None, state: None, others: Vec::new(), had_state: true, missed };
    }
    let _held = state_lock(data);
    // Not to be read just now: a new window, nothing written over it.
    let (mut windows, had_state, old_form, readable) = match read(data) {
        Ok((w, had, old)) => (w, had, old, true),
        Err(e) => {
            data.log(&format!("state.json not read ({e}): a new window, the others left as they are"));
            (Map::new(), true, false, false)
        }
    };
    let mut changed = old_form;
    if readable {
        clear_stale_locks(data, &windows);
    }
    // The wanted window, but state.json not to be read: what it had is not
    // known, so it is left as it is (to come back) and this is a new one.
    let wanted_lock = match wanted_lock {
        Some(w) if readable => Some(w),
        Some(_) if reopen => return Claim { id: new_id(), lock: None, state: None, others: Vec::new(), had_state: true, missed: true },
        _ => None,
    };
    if let Some((id, lock)) = wanted_lock {
        let state = windows.get(&id).and_then(|v| v.as_object().cloned());
        if changed && readable {
            let _ = write(data, &windows);
        }
        return Claim { id, lock: Some(lock), state, others: Vec::new(), had_state, missed: false };
    }
    // The windows no weft has open (nor is opening), the most recent first.
    let now = now_secs();
    // (Left to a weft being started for it: for a while, and only while
    // the process starting it runs, when that is known.)
    let opening = |v: &Value| {
        let until = v.get("opening_until").and_then(|t| t.as_f64()).is_some_and(|t| t > now);
        let by = v.get("opening_pid").and_then(|p| p.as_u64()).map(|p| weft_core::pty::process_alive(p as u32));
        until && by.unwrap_or(true)
    };
    let at = |id: &str| windows.get(id).and_then(|v| v.get("at")).and_then(|a| a.as_f64()).unwrap_or(0.0);
    let candidates: Vec<String> = windows.iter().filter(|(_, v)| !opening(v)).map(|(id, _)| id.clone()).collect();
    let mut free: Vec<(String, f64, File)> = candidates.iter().filter_map(|id| take(data, id).map(|f| (id.clone(), at(id), f))).collect();
    // One let go of a moment ago may still look held (a process being
    // started anywhere holds a copy of its lock until it runs its program):
    // those asked again for a moment (one another weft has open stays held:
    // this weft starts that much later, no more).
    let until = std::time::Instant::now() + held_wait;
    loop {
        let held: Vec<&String> = candidates.iter().filter(|id| !free.iter().any(|f| &f.0 == *id)).collect();
        if held.is_empty() || std::time::Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
        for id in held {
            if let Some(f) = take(data, id) {
                free.push((id.clone(), at(id), f));
            }
        }
    }
    free.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut mine: Option<(String, File)> = None;
    let mut others = Vec::new();
    for (i, (id, _, lock)) in free.into_iter().enumerate() {
        match (i, restore) {
            (_, "none") => {
                windows.remove(&id);
                changed = true;
            }
            (0, _) => mine = Some((id, lock)),
            (_, "last") => {
                windows.remove(&id);
                changed = true;
            }
            // Let go of for the weft started for it; left to it a while.
            _ => {
                if let Some(Value::Object(w)) = windows.get_mut(&id) {
                    w.insert("opening_until".into(), Value::from(now + OPENING));
                    changed = true;
                }
                others.push(id);
            }
        }
    }
    if changed && readable {
        let _ = write(data, &windows);
    }
    match mine {
        Some((id, lock)) => {
            let state = windows.get(&id).and_then(|v| v.as_object().cloned());
            Claim { id, lock: Some(lock), state, others, had_state, missed }
        }
        None => {
            let id = new_id();
            let lock = take(data, &id);
            Claim { id, lock, state: None, others, had_state, missed }
        }
    }
}

/// This window's place in state.json written (the others as they are).
pub fn save(data: &DataDir, id: &str, mut state: Map<String, Value>) -> std::io::Result<()> {
    let _held = state_lock(data);
    let (mut windows, _, _) = read(data)?;
    state.insert("at".into(), Value::from(now_secs()));
    windows.insert(id.to_string(), Value::Object(state));
    write(data, &windows)
}

/// Whether a weft other than this one is running on this data folder.
/// (Lock files are not taken away here: a weft may be about to take one.)
fn others_running(data: &DataDir, me: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(data.path("instances")) else { return false };
    rd.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.strip_suffix(".lock").is_some_and(|id| id != me && take(data, id).is_none())
    })
}

/// Lock files of windows state.json no longer has and no weft holds:
/// taken away (as weft starts, under state.json's lock; a window it has
/// may be about to be taken by a weft, so those stay).
fn clear_stale_locks(data: &DataDir, windows: &Windows) {
    let Ok(rd) = std::fs::read_dir(data.path("instances")) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".lock") else { continue };
        if windows.contains_key(id) {
            continue;
        }
        if let Some(f) = take(data, id) {
            drop(f);
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// How long a restart's window is left to the new weft: a launcher
/// building it first takes minutes.
pub const HANDING_OVER: f64 = 600.0;

/// weft closing: its window gone from state.json if another weft runs
/// (closed, as a window is), kept if this is the last one (to come back);
/// `handing_over` (a restart, the process id of the weft or launcher it
/// started): kept, and left to the new weft for a while, as long as that
/// process runs (a launcher whose build failed: free at once). Its window's `lock`
/// is let go of before state.json's, so of two wefts closing at once the
/// second sees the first gone: one window stays, never none.
pub fn leave(data: &DataDir, id: &str, handing_over: Option<u32>, lock: &mut Option<File>) {
    let _held = state_lock(data);
    decide(data, id, handing_over);
    *lock = None;
}

fn decide(data: &DataDir, id: &str, handing_over: Option<u32>) {
    let Ok((mut windows, _, _)) = read(data) else { return };
    if let Some(pid) = handing_over {
        if let Some(Value::Object(w)) = windows.get_mut(id) {
            w.insert("opening_until".into(), Value::from(now_secs() + HANDING_OVER));
            w.insert("opening_pid".into(), Value::from(pid));
            let _ = write(data, &windows);
        }
        return;
    }
    if !others_running(data, id) {
        return;
    }
    if windows.remove(id).is_some() {
        let _ = write(data, &windows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window whose lock looks held for a moment (a process being started
    /// anywhere holds a copy of it until it runs its program) is still taken:
    /// opened again, or with "none" let go of; not left behind as one
    /// another weft has open. (It happened only when another test started a
    /// program just then; here the lock is held on purpose.)
    #[test]
    fn a_window_held_for_a_moment_is_still_taken() {
        for restore in ["all", "none"] {
            let dir = std::env::temp_dir().join(format!("weft-claim-{}-{restore}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let data = DataDir::at(dir.clone());
            let mut w = Windows::new();
            for (id, at) in [("aaa", 1.0), ("bbb", 2.0)] {
                w.insert(id.into(), serde_json::json!({"at": at}));
            }
            write(&data, &w).unwrap();
            // The most recent one's lock, held 50 ms by someone else.
            let held = take(&data, "bbb").expect("its lock");
            let other = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                drop(held);
            });
            // (Asked again for long enough here that a slow machine's
            // thread lets go of it in time: what is tested is that it is
            // asked again, not how fast.)
            let c = claim_waiting(&data, None, false, restore, Duration::from_secs(5));
            other.join().unwrap();
            let (left, _, _) = read(&data).unwrap();
            if restore == "all" {
                assert_eq!(c.id, "bbb", "the most recent, though held a moment");
                assert_eq!(c.others, ["aaa"], "the other opened too");
            } else {
                assert!(left.is_empty(), "none: both let go of, not one left behind: {left:?}");
            }
            drop(c);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
