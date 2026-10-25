//! Raw Darwin system calls used by the storage engine.
//!
//! Public so that examples and benchmarks measure exactly the wrappers the engine calls
//! (for instance `examples/clone_scaling.rs` times [`clone_tree`] directly).

use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Duration;

/// `<sys/clonefile.h>`: do not follow a symlink at `src`.
pub const CLONE_NOFOLLOW: u32 = 0x0001;
/// `<sys/clonefile.h>`: do not copy ownership; the clone is owned by the caller.
pub const CLONE_NOOWNERCOPY: u32 = 0x0002;
/// `<sys/stdio.h>`: atomically exchange `from` and `to`.
pub const RENAME_SWAP: libc::c_uint = 0x0000_0002;
/// `<sys/stdio.h>`: fail with `EEXIST` instead of replacing an existing `to`.
pub const RENAME_EXCL: libc::c_uint = 0x0000_0004;

extern "C" {
    fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
    fn renameatx_np(
        fromfd: libc::c_int,
        from: *const libc::c_char,
        tofd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
}

/// Clones `src` (file or whole directory hierarchy) to `dst` with
/// `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)`. `dst` must not exist.
pub fn clone_tree(src: &Path, dst: &Path) -> io::Result<()> {
    let (src, dst) = (c_path(src)?, c_path(dst)?);
    // SAFETY: both arguments are valid NUL-terminated strings that outlive the call.
    let rc = unsafe { clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW | CLONE_NOOWNERCOPY) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Exchanges the directory entries `a` and `b` in one syscall:
/// `renameatx_np(AT_FDCWD, a, AT_FDCWD, b, RENAME_SWAP)`.
pub fn swap(a: &Path, b: &Path) -> io::Result<()> {
    let (a, b) = (c_path(a)?, c_path(b)?);
    // SAFETY: valid C strings; AT_FDCWD makes the kernel resolve them like open(2) would.
    let rc = unsafe { renameatx_np(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), RENAME_SWAP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Moves `from` to `to` without ever replacing an existing `to`:
/// `renameatx_np(AT_FDCWD, from, AT_FDCWD, to, RENAME_EXCL)`.
///
/// Plain `rename(2)` silently replaces an empty directory at `to`; this fails with `EEXIST`
/// instead, so a mis-generated destination name can never clobber another tree.
pub fn rename_excl(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (c_path(from)?, c_path(to)?);
    // SAFETY: valid NUL-terminated strings that outlive the call; AT_FDCWD resolves them like open(2).
    let rc = unsafe { renameatx_np(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), RENAME_EXCL) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// True unless `kill(pid, 0)` reports `ESRCH` (no such process). `EPERM` means the process
/// exists but belongs to another user, which counts as alive.
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 performs only the existence and permission check; no signal is delivered.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// User and system CPU time consumed so far by this process (all threads), from
/// `getrusage(RUSAGE_SELF)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub user: Duration,
    pub system: Duration,
}

pub fn process_cpu_times() -> io::Result<CpuTimes> {
    // SAFETY: getrusage only writes into the zero-initialised struct we pass by pointer.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    let to_duration = |tv: libc::timeval| {
        Duration::from_secs(tv.tv_sec.max(0) as u64) + Duration::from_micros(tv.tv_usec.max(0) as u64)
    };
    Ok(CpuTimes { user: to_duration(usage.ru_utime), system: to_duration(usage.ru_stime) })
}

/// File system type name reported by `statfs(2)` (e.g. "apfs").
pub fn fs_type(path: &Path) -> io::Result<String> {
    let path = c_path(path)?;
    // SAFETY: statfs only writes into the zero-initialised struct we pass by pointer.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(path.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel NUL-terminates f_fstypename within its fixed-size buffer.
    let name = unsafe { CStr::from_ptr(st.f_fstypename.as_ptr()) };
    Ok(name.to_string_lossy().into_owned())
}

/// Exclusive advisory lock (`flock(LOCK_EX)`) released on drop.
pub struct FileLock {
    file: File,
}

impl FileLock {
    pub fn exclusive(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
        // SAFETY: the descriptor stays valid for as long as `file` lives.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // SAFETY: same descriptor as above; unlocking an fd we hold cannot violate memory safety.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
