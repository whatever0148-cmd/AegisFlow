//! Small helpers: millisecond timestamps, duration conversion, the session token, and the
//! process-lifetime lock on the demo directory.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::raw::c_int;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand_core::{OsRng, RngCore};

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC, millisecond resolution).
pub fn iso_ms(t: SystemTime) -> String {
    let since = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs() as i64;
    let millis = since.subsec_millis();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

pub fn now_iso() -> String {
    iso_ms(SystemTime::now())
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Rounds to 6 decimals so JSON stays readable without hiding sub-microsecond detail in ms values.
pub fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

pub fn ms(d: Duration) -> f64 {
    round6(d.as_secs_f64() * 1e3)
}

pub fn us(d: Duration) -> f64 {
    round6(d.as_secs_f64() * 1e6)
}

pub fn opt_ms(d: Option<Duration>) -> Option<f64> {
    d.map(ms)
}

/// 32 random bytes from the OS CSPRNG, hex encoded.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Random ASCII letters and digits (used for the credential-shaped string of `secret_leak`).
pub fn random_alnum(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = vec![0u8; len];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char).collect()
}

/// Comparison whose running time does not depend on where the inputs differ.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

extern "C" {
    /// flock(2) from libSystem / libc.
    fn flock(fd: c_int, operation: c_int) -> c_int;
}
/// Same values on macOS (`<sys/fcntl.h>`) and Linux.
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;

/// An exclusive `flock` held for as long as the value lives (the kernel drops it when the
/// descriptor closes, including when the process dies).
#[derive(Debug)]
pub struct ProcessLock {
    _file: File,
}

impl ProcessLock {
    /// Takes the lock on `path` (created 0600 if missing) without waiting: `Ok(None)` when another
    /// process holds it.
    pub fn try_exclusive(path: &Path) -> io::Result<Option<Self>> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(path)?;
        loop {
            // SAFETY: `file` owns a valid open descriptor for the duration of the call; flock takes
            // an advisory lock and touches no memory. The return code is checked below.
            let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
            if rc == 0 {
                return Ok(Some(Self { _file: file }));
            }
            let err = io::Error::last_os_error();
            match err.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => return Ok(None),
                _ => return Err(err),
            }
        }
    }
}

/// Keeps printable characters only and caps the length (free-text fields from the console).
pub fn clean_text(text: &str, max_chars: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max_chars).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_milliseconds() {
        assert_eq!(iso_ms(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_ms(UNIX_EPOCH + Duration::from_millis(951_782_400_123)), "2000-02-29T00:00:00.123Z");
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(constant_time_eq(a.as_bytes(), a.as_bytes()));
        assert!(!constant_time_eq(a.as_bytes(), b.as_bytes()));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn process_lock_excludes_a_second_holder_until_dropped() {
        let dir = std::env::temp_dir().join(format!("aegis-daemon-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.lock");
        let first = ProcessLock::try_exclusive(&path).unwrap();
        assert!(first.is_some());
        // A second open file description conflicts even inside one process (flock semantics).
        assert!(ProcessLock::try_exclusive(&path).unwrap().is_none());
        drop(first);
        assert!(ProcessLock::try_exclusive(&path).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_text_strips_controls() {
        assert_eq!(clean_text(" ali\u{7}ce\n ", 64), "alice");
        assert_eq!(clean_text("abcdef", 3), "abc");
    }
}
