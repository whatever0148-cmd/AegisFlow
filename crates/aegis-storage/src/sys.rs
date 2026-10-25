//! Raw Darwin system calls used by the storage engine.

use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// `<sys/clonefile.h>`: do not follow a symlink at `src`.
pub const CLONE_NOFOLLOW: u32 = 0x0001;
/// `<sys/clonefile.h>`: do not copy ownership; the clone is owned by the caller.
pub const CLONE_NOOWNERCOPY: u32 = 0x0002;
/// `<stdio.h>`: atomically exchange `from` and `to`.
pub const RENAME_SWAP: libc::c_uint = 0x0000_0002;

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
