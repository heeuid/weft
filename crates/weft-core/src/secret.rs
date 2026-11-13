//! ssh passwords held for the session (to reconnect), never written to disk.
//!
//! In memory each one is encrypted (ChaCha20) under a random key that is
//! replaced every `rotate_secs`: everything is re-encrypted under the new key
//! and the old one is zeroed. On Windows the key itself is sealed with
//! CryptProtectMemory, so it isn't readable from a dump of this process
//! either; elsewhere it is only zeroed when dropped. A password is decrypted
//! only for the moment it is handed to ssh, into a buffer zeroed after use.
//!
//! The key's memory is locked out of swap (mlock / VirtualLock). Linux and
//! macOS also keep other processes of the same user from attaching to weft
//! (see `harden_process`); Windows has no such switch, so there code running
//! as the same user could still attach and watch a password being decrypted.
//! ssh gets a password through a one-time ticket, not its environment (see
//! `ticket_env`). Better still is no password at all: keys + ssh-agent.

use chacha20::cipher::{KeyIvInit, StreamCipher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct VaultSettings {
    /// Encrypt held passwords (else kept as plain bytes, still zeroed when dropped).
    pub encrypt: bool,
    /// Replace the key this often, in seconds (0: never).
    pub rotate_secs: u64,
    /// Forget a password unused for this long, in seconds (0: keep for the session).
    pub forget_secs: u64,
}

impl Default for VaultSettings {
    fn default() -> Self {
        VaultSettings { encrypt: true, rotate_secs: 60, forget_secs: 0 }
    }
}

/// A 32-byte key, sealed by the OS between uses where it can be.
struct Key(Box<[u8; 32]>);

impl Key {
    fn random() -> Key {
        let mut k = Box::new([0u8; 32]);
        lock_in_ram(k.as_ptr(), 32);
        getrandom::getrandom(&mut k[..]).expect("no OS random source");
        let mut key = Key(k);
        key.seal();
        key
    }

    #[cfg(windows)]
    fn seal(&mut self) {
        use windows_sys::Win32::Security::Cryptography::{CryptProtectMemory, CRYPTPROTECTMEMORY_SAME_PROCESS};
        // 32 is a multiple of CRYPTPROTECTMEMORY_BLOCK_SIZE (16).
        unsafe { CryptProtectMemory(self.0.as_mut_ptr().cast(), 32, CRYPTPROTECTMEMORY_SAME_PROCESS) };
    }

    #[cfg(windows)]
    fn unseal(&mut self) {
        use windows_sys::Win32::Security::Cryptography::{CryptUnprotectMemory, CRYPTPROTECTMEMORY_SAME_PROCESS};
        unsafe { CryptUnprotectMemory(self.0.as_mut_ptr().cast(), 32, CRYPTPROTECTMEMORY_SAME_PROCESS) };
    }

    #[cfg(not(windows))]
    fn seal(&mut self) {}

    #[cfg(not(windows))]
    fn unseal(&mut self) {}

    /// Run `f` with the key in the clear, sealed again right after.
    fn with<R>(&mut self, f: impl FnOnce(&[u8; 32]) -> R) -> R {
        self.unseal();
        let r = f(&self.0);
        self.seal();
        r
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

struct Held {
    nonce: [u8; 12],
    /// Ciphertext, or the plain bytes when encryption is off.
    bytes: Zeroizing<Vec<u8>>,
    encrypted: bool,
    used: Instant,
}

fn xor_stream(key: &[u8; 32], nonce: &[u8; 12], data: &mut [u8]) {
    chacha20::ChaCha20::new(key.into(), nonce.into()).apply_keystream(data);
}

pub struct Vault {
    items: HashMap<String, Held>,
    key: Key,
    born: Instant,
    cfg: VaultSettings,
}

impl Vault {
    pub fn new(cfg: VaultSettings) -> Vault {
        Vault { items: HashMap::new(), key: Key::random(), born: Instant::now(), cfg }
    }

    fn seal(&mut self, plain: &[u8], used: Instant) -> Held {
        let mut bytes = Zeroizing::new(plain.to_vec());
        let mut nonce = [0u8; 12];
        if self.cfg.encrypt {
            getrandom::getrandom(&mut nonce).expect("no OS random source");
            self.key.with(|k| xor_stream(k, &nonce, &mut bytes));
        }
        Held { nonce, bytes, encrypted: self.cfg.encrypt, used }
    }

    fn open(&mut self, h: &Held) -> Zeroizing<Vec<u8>> {
        let mut bytes = h.bytes.clone();
        if h.encrypted {
            self.key.with(|k| xor_stream(k, &h.nonce, &mut bytes));
        }
        bytes
    }

    pub fn insert(&mut self, host: &str, password: Zeroizing<String>) {
        let held = self.seal(password.as_bytes(), Instant::now());
        self.items.insert(host.to_string(), held);
    }

    /// The password in a buffer that is zeroed when dropped.
    pub fn get(&mut self, host: &str) -> Option<Zeroizing<String>> {
        let h = self.items.remove(host)?;
        let plain = self.open(&h);
        self.items.insert(host.to_string(), Held { used: Instant::now(), ..h });
        Some(Zeroizing::new(String::from_utf8_lossy(&plain).into_owned()))
    }

    #[cfg(test)]
    fn contains(&self, host: &str) -> bool {
        self.items.contains_key(host)
    }

    pub fn remove(&mut self, host: &str) -> bool {
        self.items.remove(host).is_some()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// New settings: held passwords are re-sealed to match (encrypted or not).
    pub fn configure(&mut self, cfg: VaultSettings) {
        if cfg == self.cfg {
            return;
        }
        self.cfg = cfg;
        self.reseal(Key::random());
    }

    /// A new key; everything re-encrypted under it; the old key zeroed.
    fn reseal(&mut self, new: Key) {
        let held: Vec<(String, Held)> = self.items.drain().collect();
        let plains: Vec<(String, Zeroizing<Vec<u8>>, Instant)> = held.into_iter().map(|(k, h)| (k, self.open(&h), h.used)).collect();
        self.key = new;
        self.born = Instant::now();
        for (host, plain, used) in plains {
            let h = self.seal(&plain, used);
            self.items.insert(host, h);
        }
    }

    /// Rotate the key and forget unused passwords when due. Returns the hosts
    /// forgotten and how long until something is due again.
    pub fn tick(&mut self) -> (Vec<String>, Option<Duration>) {
        let mut forgotten = Vec::new();
        if self.cfg.forget_secs > 0 {
            let limit = Duration::from_secs(self.cfg.forget_secs);
            self.items.retain(|host, h| {
                let keep = h.used.elapsed() < limit;
                if !keep {
                    forgotten.push(host.clone());
                }
                keep
            });
        }
        if self.cfg.encrypt && self.cfg.rotate_secs > 0 && self.born.elapsed() >= Duration::from_secs(self.cfg.rotate_secs) {
            self.reseal(Key::random());
        }
        let mut next: Option<Duration> = None;
        if !self.items.is_empty() {
            if self.cfg.encrypt && self.cfg.rotate_secs > 0 {
                next = Some(Duration::from_secs(self.cfg.rotate_secs).saturating_sub(self.born.elapsed()));
            }
            if self.cfg.forget_secs > 0 {
                let limit = Duration::from_secs(self.cfg.forget_secs);
                let soonest = self.items.values().map(|h| limit.saturating_sub(h.used.elapsed())).min();
                next = match (next, soonest) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        (forgotten, next)
    }

    #[cfg(test)]
    fn raw(&self, host: &str) -> Vec<u8> {
        self.items[host].bytes.to_vec()
    }
}

// ---------------------------------------------------------------------------
// Handing a password to ssh
// ---------------------------------------------------------------------------

/// ssh gets the password from `weft --askpass`. Putting it in that
/// process's environment would leave it there, readable by the same user,
/// for as long as ssh runs. Instead the environment carries a port and a
/// random ticket; askpass trades the ticket for the password over loopback.
/// A ticket works up to TICKET_USES times (ssh asks again after a wrong
/// password, and once per jump host) within TICKET_LIFE, then it is gone.
const TICKET_LIFE: Duration = Duration::from_secs(120);
const TICKET_USES: u8 = 3;

pub const PORT_VAR: &str = "WEFT_ASKPASS_PORT";
pub const TICKET_VAR: &str = "WEFT_ASKPASS_TICKET";

type Tickets = std::sync::Arc<std::sync::Mutex<HashMap<String, (Zeroizing<String>, Instant, u8)>>>;

struct Handoff {
    port: u16,
    tickets: Tickets,
}

fn handoff() -> Option<&'static Handoff> {
    static H: std::sync::OnceLock<Option<Handoff>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        let tickets: Tickets = Default::default();
        let t = tickets.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve_ticket(stream, &t);
            }
        });
        Some(Handoff { port, tickets })
    })
    .as_ref()
}

fn serve_ticket(mut stream: std::net::TcpStream, tickets: &Tickets) {
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 80];
    let mut n = 0;
    while n < buf.len() {
        match stream.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => {
                n += k;
                if buf[..n].contains(&b'\n') {
                    break;
                }
            }
        }
    }
    let ticket = String::from_utf8_lossy(&buf[..n]).trim().to_string();
    let mut map = tickets.lock().unwrap();
    map.retain(|_, v| v.1.elapsed() < TICKET_LIFE && v.2 > 0);
    if let Some(v) = map.get_mut(&ticket) {
        v.2 -= 1;
        let _ = stream.write_all(v.0.as_bytes());
    }
    map.retain(|_, v| v.2 > 0);
}

/// Environment for ssh's askpass: where to fetch the password, and a ticket.
pub fn ticket_env(password: &str) -> Vec<(String, String)> {
    let Some(h) = handoff() else { return Vec::new() };
    let mut raw = [0u8; 24];
    getrandom::getrandom(&mut raw).expect("no OS random source");
    let ticket: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let mut map = h.tickets.lock().unwrap();
    map.retain(|_, v| v.1.elapsed() < TICKET_LIFE && v.2 > 0);
    map.insert(ticket.clone(), (Zeroizing::new(password.to_string()), Instant::now(), TICKET_USES));
    vec![(PORT_VAR.into(), h.port.to_string()), (TICKET_VAR.into(), ticket)]
}

/// In `weft --askpass`: trade the ticket from the environment for the password.
pub fn fetch_password() -> Option<Zeroizing<String>> {
    let port: u16 = std::env::var(PORT_VAR).ok()?.parse().ok()?;
    fetch_from(port, &std::env::var(TICKET_VAR).ok()?)
}

fn fetch_from(port: u16, ticket: &str) -> Option<Zeroizing<String>> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(format!("{ticket}\n").as_bytes()).ok()?;
    let mut out = Zeroizing::new(Vec::new());
    s.read_to_end(&mut out).ok()?;
    (!out.is_empty()).then(|| Zeroizing::new(String::from_utf8_lossy(&out).into_owned()))
}

// ---------------------------------------------------------------------------
// Process hardening
// ---------------------------------------------------------------------------

/// Keep other processes of the same user out of weft's memory where the OS
/// allows it, and keep it out of core dumps. Linux: not dumpable (no ptrace
/// attach, no /proc/<pid>/mem, no core). macOS: no debugger attach, no core.
/// Windows has no per-process switch for this; passwords there rely on the
/// encryption above.
pub fn harden_process() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
        libc::ptrace(libc::PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0);
    }
}

/// Keep a buffer out of swap / the page file.
fn lock_in_ram(ptr: *const u8, len: usize) {
    #[cfg(unix)]
    unsafe {
        libc::mlock(ptr.cast(), len);
    }
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Memory::VirtualLock(ptr as *const _, len);
    }
}

/// A time in seconds as the largest unit that divides it: (value, unit index
/// into UNITS).
pub const UNITS: [(&str, u64); 3] = [("seconds", 1), ("minutes", 60), ("hours", 3600)];

pub fn split_secs(secs: u64) -> (u64, usize) {
    for (i, (_, n)) in UNITS.iter().enumerate().rev() {
        if secs > 0 && secs.is_multiple_of(*n) {
            return (secs / n, i);
        }
    }
    (secs, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    #[test]
    fn held_encrypted_and_given_back() {
        let mut v = Vault::new(VaultSettings::default());
        v.insert("srv", pw("hunter2-비밀"));
        assert_ne!(v.raw("srv"), "hunter2-비밀".as_bytes(), "not stored in the clear");
        assert_eq!(v.get("srv").as_deref().map(String::as_str), Some("hunter2-비밀"));
        assert!(v.get("other").is_none());
        assert!(v.remove("srv") && !v.contains("srv"));
    }

    #[test]
    fn rotation_reencrypts_under_a_new_key() {
        let mut v = Vault::new(VaultSettings { encrypt: true, rotate_secs: 1, forget_secs: 0 });
        v.insert("a", pw("one"));
        v.insert("b", pw("two"));
        let before = v.raw("a");
        v.born -= Duration::from_secs(2);
        let (gone, next) = v.tick();
        assert!(gone.is_empty());
        assert!(next.unwrap() <= Duration::from_secs(1));
        assert_ne!(v.raw("a"), before, "new key, new nonce: new bytes");
        assert_eq!(v.get("a").unwrap().as_str(), "one");
        assert_eq!(v.get("b").unwrap().as_str(), "two");
    }

    #[test]
    fn unused_passwords_are_forgotten() {
        let mut v = Vault::new(VaultSettings { encrypt: true, rotate_secs: 0, forget_secs: 60 });
        v.insert("old", pw("x"));
        v.insert("new", pw("y"));
        v.items.get_mut("old").unwrap().used -= Duration::from_secs(61);
        let (gone, next) = v.tick();
        assert_eq!(gone, ["old"]);
        assert!(v.contains("new") && next.is_some());
    }

    #[test]
    fn switching_encryption_off_and_on() {
        let mut v = Vault::new(VaultSettings::default());
        v.insert("h", pw("secret"));
        v.configure(VaultSettings { encrypt: false, ..VaultSettings::default() });
        assert_eq!(v.raw("h"), b"secret");
        v.configure(VaultSettings::default());
        assert_ne!(v.raw("h"), b"secret");
        assert_eq!(v.get("h").unwrap().as_str(), "secret");
    }

    #[cfg(windows)]
    #[test]
    fn key_is_sealed_by_the_os_between_uses() {
        let mut k = Key(Box::new([7u8; 32]));
        k.seal();
        assert_ne!(*k.0, [7u8; 32]);
        assert_eq!(k.with(|clear| *clear), [7u8; 32]);
        assert_ne!(*k.0, [7u8; 32], "sealed again after use");
    }

    #[test]
    fn tickets_hand_out_the_password_a_few_times() {
        let env = ticket_env("pass word 비밀");
        let get = |k: &str| env.iter().find(|e| e.0 == k).map(|e| e.1.clone()).unwrap();
        let (port, ticket): (u16, String) = (get(PORT_VAR).parse().unwrap(), get(TICKET_VAR));
        assert!(env.iter().all(|e| !e.1.contains("pass")));
        assert!(fetch_from(port, "not-a-ticket").is_none());
        for _ in 0..TICKET_USES {
            assert_eq!(fetch_from(port, &ticket).unwrap().as_str(), "pass word 비밀");
        }
        assert!(fetch_from(port, &ticket).is_none(), "used up");
    }

    #[test]
    fn times_split_into_units() {
        assert_eq!(split_secs(0), (0, 0));
        assert_eq!(split_secs(45), (45, 0));
        assert_eq!(split_secs(120), (2, 1));
        assert_eq!(split_secs(7200), (2, 2));
        assert_eq!(split_secs(90), (90, 0));
    }
}
