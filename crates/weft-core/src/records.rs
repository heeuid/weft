//! What weft keeps on disk about terminal use, and for how long.
//!
//! | item      | where                                  | what                                   |
//! |-----------|----------------------------------------|----------------------------------------|
//! | commands  | `records/<date>.jsonl`                 | each finished command: line, output, exit code, time, directory, host |
//! | history   | `history/<host>.txt`                   | command lines (`#<unix>` before each, as bash writes them) |
//! | app log   | `logs/weft-<date>.log`                 | weft's own events                       |
//! | agent log | `logs/agent-<host>-<date>.log`         | requests to and answers from each host's agent |
//!
//! Each item can be turned off and has a keep time and a size limit
//! (settings "records"). Keystrokes are never written, so what a password
//! prompt reads (not echoed) is in none of them; a command typed with a
//! leading space is left out of commands and history, as bash's
//! HISTCONTROL=ignorespace does.

use chrono::{Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// One item's switch and limits. 0 means no limit.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Rule {
    pub on: bool,
    pub keep_days: u32,
    pub max_mb: u32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Records {
    pub commands: Rule,
    /// Most output kept per command, in KB (0: all of it).
    pub output_kb: u32,
    /// The size limit is per host.
    pub history: Rule,
    pub app_log: Rule,
    pub agent_log: Rule,
}

impl Default for Records {
    fn default() -> Self {
        Records {
            commands: Rule { on: true, keep_days: 90, max_mb: 200 },
            output_kb: 256,
            history: Rule { on: true, keep_days: 365, max_mb: 20 },
            app_log: Rule { on: true, keep_days: 30, max_mb: 50 },
            agent_log: Rule { on: true, keep_days: 14, max_mb: 200 },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Item {
    Commands,
    History,
    AppLog,
    AgentLog,
}

impl Item {
    pub const ALL: [Item; 4] = [Item::Commands, Item::History, Item::AppLog, Item::AgentLog];

    pub fn label(self) -> &'static str {
        match self {
            Item::Commands => "Commands and output",
            Item::History => "Command history",
            Item::AppLog => "weft log",
            Item::AgentLog => "Agent logs",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Item::Commands => "each finished command with its output, exit code, time, directory and host (searchable in the History panel)",
            Item::History => "command lines for the composer's ↑↓ and suggestions; the size limit is per host",
            Item::AppLog => "weft's own events: start, shells started, toasts, errors",
            Item::AgentLog => "what the file panel and editor asked each host and what came back",
        }
    }

    pub fn rule(self, r: &Records) -> Rule {
        match self {
            Item::Commands => r.commands,
            Item::History => r.history,
            Item::AppLog => r.app_log,
            Item::AgentLog => r.agent_log,
        }
    }

    pub fn rule_mut(self, r: &mut Records) -> &mut Rule {
        match self {
            Item::Commands => &mut r.commands,
            Item::History => &mut r.history,
            Item::AppLog => &mut r.app_log,
            Item::AgentLog => &mut r.agent_log,
        }
    }

    /// This item's files under the data folder.
    pub fn files(self, root: &Path) -> Vec<PathBuf> {
        let (dir, keep): (&str, fn(&str) -> bool) = match self {
            Item::Commands => ("records", |n| n.ends_with(".jsonl")),
            Item::History => ("history", |n| n.ends_with(".txt")),
            Item::AppLog => ("logs", |n| n.starts_with("weft") && n.ends_with(".log")),
            Item::AgentLog => ("logs", |n| n.starts_with("agent-") && n.ends_with(".log")),
        };
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { return Vec::new() };
        rd.flatten().filter(|e| keep(&e.file_name().to_string_lossy())).map(|e| e.path()).collect()
    }

    /// Bytes on disk now.
    pub fn size(self, root: &Path) -> u64 {
        self.files(root).iter().filter_map(|p| p.metadata().ok()).map(|m| m.len()).sum()
    }
}

// Logging switches, read by DataDir::log and the agent connections (which
// run on their own threads).
static APP_LOG: AtomicBool = AtomicBool::new(true);
static AGENT_LOG: AtomicBool = AtomicBool::new(true);

pub fn set_switches(r: &Records) {
    APP_LOG.store(r.app_log.on, Ordering::Relaxed);
    AGENT_LOG.store(r.agent_log.on, Ordering::Relaxed);
}

pub fn app_log_on() -> bool {
    APP_LOG.load(Ordering::Relaxed)
}

pub fn agent_log_on() -> bool {
    AGENT_LOG.load(Ordering::Relaxed)
}

/// Today's local date, for file names.
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Now, for a file name: "20261115-143005".
pub fn stamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A unix time as local "2026-11-12 14:03".
pub fn local_time(t: f64) -> String {
    Local.timestamp_opt(t as i64, 0).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

/// One finished command.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Record {
    /// Start, unix seconds.
    pub t: f64,
    pub host: String,
    #[serde(default)]
    pub cwd: String,
    pub cmd: String,
    /// None when the shell doesn't report it (guessed commands).
    pub exit: Option<i32>,
    /// Seconds.
    pub dur: f64,
    pub out: String,
    /// Output bytes left out (over the output limit).
    #[serde(default)]
    pub cut: u64,
}

/// Cut output to `kb` (0: keep all): the start and the end are kept, the
/// middle is left out. Returns the bytes left out.
pub fn limit_output(out: &mut String, kb: u32) -> u64 {
    let max = kb as usize * 1024;
    if kb == 0 || out.len() <= max {
        return 0;
    }
    let mut head = max / 2;
    while !out.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = out.len() - max / 2;
    while !out.is_char_boundary(tail) {
        tail += 1;
    }
    let cut = (tail - head) as u64;
    *out = format!("{}\n… ({cut} bytes left out) …\n{}", &out[..head], &out[tail..]);
    cut
}

pub fn append(root: &Path, rec: &Record) -> std::io::Result<()> {
    use std::io::Write;
    let dir = root.join("records");
    std::fs::create_dir_all(&dir)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.jsonl", today())))?;
    writeln!(f, "{}", serde_json::to_string(rec).unwrap_or_default())
}

/// Every command kept, oldest first.
pub fn load(root: &Path) -> Vec<Record> {
    let mut files = Item::Commands.files(root);
    files.sort();
    let mut out: Vec<Record> = files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|s| s.lines().filter_map(|l| serde_json::from_str::<Record>(l).ok()).collect::<Vec<_>>())
        .collect();
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

/// A history file's line, unless it is bash's `#<unix>` time line.
pub fn history_command(line: &str) -> Option<&str> {
    let is_time = line.len() > 1 && line.starts_with('#') && line[1..].bytes().all(|b| b.is_ascii_digit());
    (!is_time && !line.is_empty()).then_some(line)
}

/// The date a dated file is for: from a name ending in -YYYY-MM-DD (or
/// named so), else its modification day (files from before dated names).
fn file_date(p: &Path) -> Option<NaiveDate> {
    let stem = p.file_stem()?.to_string_lossy().into_owned();
    if stem.len() >= 10 {
        if let Ok(d) = NaiveDate::parse_from_str(&stem[stem.len() - 10..], "%Y-%m-%d") {
            return Some(d);
        }
    }
    let m = p.metadata().ok()?.modified().ok()?;
    Some(chrono::DateTime::<Local>::from(m).date_naive())
}

/// Apply every item's limits: files of switched-off items stay until
/// deleted by hand; their limits still apply. Returns what was removed.
pub fn prune(root: &Path, r: &Records) -> Vec<String> {
    let mut done = Vec::new();
    let today = Local::now().date_naive();
    for item in [Item::Commands, Item::AppLog, Item::AgentLog] {
        let rule = item.rule(r);
        // Oldest first.
        let mut files: Vec<(NaiveDate, PathBuf, u64)> =
            item.files(root).into_iter().filter_map(|p| Some((file_date(&p)?, p.metadata().ok()?.len(), p)).map(|(d, n, p)| (d, p, n))).collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.2).sum();
        let max = rule.max_mb as u64 * 1024 * 1024;
        for (d, p, n) in &files {
            let old = rule.keep_days > 0 && (today - *d).num_days() >= rule.keep_days as i64;
            // Today's file stays even over the limit: it is being written.
            let over = rule.max_mb > 0 && total > max && *d < today;
            if (old || over) && std::fs::remove_file(p).is_ok() {
                total -= n;
                done.push(format!("{}: removed {}", item.label(), p.display()));
            }
        }
    }
    let rule = r.history;
    for p in Item::History.files(root) {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let kept = prune_history(&text, rule, now());
        if kept.len() != text.len() && std::fs::write(&p, &kept).is_ok() {
            done.push(format!("{}: trimmed {}", Item::History.label(), p.display()));
        }
    }
    done
}

/// A history file without entries older than the keep time and, over the
/// size limit, without its oldest entries. Lines without a time line before
/// them (written before weft added them) count as old only by size.
pub fn prune_history(text: &str, rule: Rule, now: f64) -> String {
    // Entries: (time, lines).
    let mut entries: Vec<(Option<f64>, String)> = Vec::new();
    let mut time = None;
    for line in text.lines() {
        if line.len() > 1 && line.starts_with('#') && line[1..].bytes().all(|b| b.is_ascii_digit()) {
            time = line[1..].parse::<f64>().ok();
        } else if !line.is_empty() {
            let head = time.map(|t| format!("#{}\n", t as i64)).unwrap_or_default();
            entries.push((time.take(), format!("{head}{line}\n")));
        }
    }
    if rule.keep_days > 0 {
        let cutoff = now - rule.keep_days as f64 * 86400.0;
        entries.retain(|e| e.0.map(|t| t >= cutoff).unwrap_or(true));
    }
    let max = rule.max_mb as usize * 1024 * 1024;
    let mut total: usize = entries.iter().map(|e| e.1.len()).sum();
    let mut skip = 0;
    while rule.max_mb > 0 && total > max && skip < entries.len() {
        total -= entries[skip].1.len();
        skip += 1;
    }
    entries[skip..].iter().map(|e| e.1.as_str()).collect()
}

/// Delete all of an item's files now.
pub fn delete_all(root: &Path, item: Item) -> usize {
    item.files(root).iter().filter(|p| std::fs::remove_file(p).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_keeps_start_and_end() {
        let mut s = "a".repeat(3000) + &"b".repeat(3000);
        let cut = limit_output(&mut s, 2);
        assert_eq!(cut, 6000 - 2048);
        assert!(s.starts_with(&"a".repeat(1024)) && s.ends_with(&"b".repeat(1024)));
        assert!(s.contains("3952 bytes left out"));
        let mut small = "한글".to_string();
        assert_eq!(limit_output(&mut small, 1), 0);
        assert_eq!(small, "한글");
        // Never splits a character.
        let mut k = "가".repeat(1000);
        limit_output(&mut k, 1);
        assert!(k.starts_with('가') && k.ends_with('가'));
    }

    #[test]
    fn history_keeps_recent_and_small() {
        let day = 86400.0;
        let now = 100.0 * day;
        let text = format!("legacy\n#{}\nold\n#{}\nnew\n#{}\nnewest\n", (now - 40.0 * day) as i64, (now - 2.0 * day) as i64, now as i64);
        let kept = prune_history(&text, Rule { on: true, keep_days: 30, max_mb: 0 }, now);
        assert_eq!(kept, format!("legacy\n#{}\nnew\n#{}\nnewest\n", (now - 2.0 * day) as i64, now as i64));
        let cmds: Vec<&str> = kept.lines().filter_map(history_command).collect();
        assert_eq!(cmds, ["legacy", "new", "newest"]);
        // Size: the oldest go first.
        let big = (0..20000).map(|i| format!("#{}\ncommand number {i}\n", now as i64)).collect::<String>();
        let kept = prune_history(&big, Rule { on: true, keep_days: 0, max_mb: 0 }, now);
        assert_eq!(kept, big);
        let kept = prune_history(&(big.clone() + &big + &big), Rule { on: true, keep_days: 0, max_mb: 1 }, now);
        assert!(kept.len() <= 1024 * 1024 && kept.ends_with("command number 19999\n") && kept.starts_with('#'));
    }

    #[test]
    fn dated_files_go_by_age_and_size() {
        let root = std::env::temp_dir().join(format!("weft-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("records")).unwrap();
        std::fs::create_dir_all(root.join("logs")).unwrap();
        let day = |n: i64| (Local::now().date_naive() - chrono::Duration::days(n)).format("%Y-%m-%d").to_string();
        for n in [0, 1, 5, 40] {
            std::fs::write(root.join(format!("records/{}.jsonl", day(n))), vec![b'x'; 600 * 1024]).unwrap();
        }
        std::fs::write(root.join(format!("logs/weft-{}.log", day(50))), "old").unwrap();
        std::fs::write(root.join(format!("logs/agent-h-{}.log", day(1))), "new").unwrap();
        let mut r = Records { commands: Rule { on: true, keep_days: 30, max_mb: 1 }, ..Default::default() };
        r.app_log.keep_days = 30;
        prune(&root, &r);
        let mut left: Vec<String> = Item::Commands.files(&root).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        left.sort();
        // Day 40 is too old. That leaves 1.8 MB over 1 MB: day 5 goes, then
        // day 1 (1.2 MB is still over); today's file stays while written.
        assert_eq!(left, [format!("{}.jsonl", day(0))]);
        assert!(Item::AppLog.files(&root).is_empty());
        assert_eq!(Item::AgentLog.files(&root).len(), 1);
        assert_eq!(delete_all(&root, Item::AgentLog), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn records_round_trip() {
        let root = std::env::temp_dir().join(format!("weft-records-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = Record { t: now(), host: "local".into(), cwd: "C:/x".into(), cmd: "echo 한글".into(), exit: Some(0), dur: 0.1, out: "한글\nline".into(), cut: 0 };
        append(&root, &r).unwrap();
        append(&root, &Record { exit: None, ..r.clone() }).unwrap();
        let got = load(&root);
        assert_eq!(got.len(), 2);
        // JSON keeps the time to well under a millisecond, not to the last bit.
        assert!((got[0].t - r.t).abs() < 1e-3);
        assert_eq!(got[0], Record { t: got[0].t, ..r.clone() });
        assert_eq!(got[1].exit, None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
