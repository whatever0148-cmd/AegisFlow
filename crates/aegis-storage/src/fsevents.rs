//! Minimal FSEvents (CoreServices) binding: one stream on one path, delivered on a private serial
//! dispatch queue.
//!
//! Hand-written rather than using `notify` or `fsevent-sys`: the pool needs exactly one call
//! sequence (create, set queue, start, flush, stop, invalidate, release), and writing it here keeps
//! `aegis-storage` free of new dependencies, so the shared `Cargo.lock` does not change and no
//! network fetch is needed. `notify` would also add its own event thread and debouncing on top of
//! FSEvents, while `fsevent-sys` only provides the same declarations written below. Delivery uses
//! `FSEventStreamSetDispatchQueue` (the run-loop scheduling API is deprecated since macOS 13), so
//! no thread has to sit in a run loop.

use std::ffi::{c_char, c_void, CStr, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

type CFIndex = isize;
type CFTypeRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type FSEventStreamRef = *mut c_void;
type ConstFSEventStreamRef = *const c_void;
type DispatchQueue = *mut c_void;

/// `kCFStringEncodingUTF8`.
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// `kFSEventStreamEventIdSinceNow`.
const EVENT_ID_SINCE_NOW: u64 = u64::MAX;
/// `kFSEventStreamCreateFlagNoDefer`: deliver the first event of a burst immediately.
const CREATE_NO_DEFER: u32 = 0x0000_0002;
/// `kFSEventStreamCreateFlagWatchRoot`: also report renames/removal of the watched path itself.
const CREATE_WATCH_ROOT: u32 = 0x0000_0004;
/// `kFSEventStreamCreateFlagFileEvents`: per-file rather than per-directory events.
const CREATE_FILE_EVENTS: u32 = 0x0000_0010;
/// `kFSEventStreamEventFlagMustScanSubDirs`, `UserDropped`, `KernelDropped`: events were coalesced
/// or lost; everything below the path may have changed.
pub(crate) const EVENT_MUST_SCAN_SUB_DIRS: u32 = 0x0000_0001;
pub(crate) const EVENT_USER_DROPPED: u32 = 0x0000_0002;
pub(crate) const EVENT_KERNEL_DROPPED: u32 = 0x0000_0004;
/// `kFSEventStreamEventFlagHistoryDone`: marker event, not a change.
pub(crate) const EVENT_HISTORY_DONE: u32 = 0x0000_0010;
/// `kFSEventStreamEventFlagMount`, `Unmount`: a volume was mounted or unmounted under the path.
pub(crate) const EVENT_MOUNT: u32 = 0x0000_0040;
pub(crate) const EVENT_UNMOUNT: u32 = 0x0000_0080;
// The item-level bits (`RootChanged` 0x20, `ItemCreated` 0x100, `ItemRemoved` 0x200,
// `ItemInodeMetaMod` 0x400, `ItemRenamed` 0x800, `ItemModified` 0x1000, `ItemFinderInfoMod`
// 0x2000, `ItemChangeOwner` 0x4000, `ItemXattrMod` 0x8000, `ItemIsDir` 0x20000, `ItemCloned`
// 0x400000) are not interpreted: FSEvents carries earlier bits of a path over into later events on
// it, so the pool decides by path and inode instead (see pool.rs). They stay visible in raw form in
// `PoolStats::recent_fs_changes`.

/// One delivered event.
#[derive(Debug, Clone)]
pub(crate) struct Event {
    pub(crate) path: PathBuf,
    pub(crate) flags: u32,
}

#[repr(C)]
struct CFArrayCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

#[repr(C)]
struct FSEventStreamContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
}

type StreamCallback = unsafe extern "C" fn(
    stream: ConstFSEventStreamRef,
    info: *mut c_void,
    num_events: usize,
    event_paths: *mut c_void,
    event_flags: *const u32,
    event_ids: *const u64,
);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeArrayCallBacks: CFArrayCallBacks;
    fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: u32,
        is_external_representation: u8,
    ) -> CFStringRef;
    fn CFArrayCreate(
        alloc: CFAllocatorRef,
        values: *const *const c_void,
        num_values: CFIndex,
        callbacks: *const CFArrayCallBacks,
    ) -> CFArrayRef;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn FSEventStreamCreate(
        alloc: CFAllocatorRef,
        callback: StreamCallback,
        context: *mut FSEventStreamContext,
        paths_to_watch: CFArrayRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueue);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamFlushSync(stream: FSEventStreamRef);
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

// libdispatch is part of libSystem, which every binary links.
extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> DispatchQueue;
    fn dispatch_release(object: *mut c_void);
    fn dispatch_sync_f(queue: DispatchQueue, context: *mut c_void, work: unsafe extern "C" fn(*mut c_void));
}

/// Called with every event of a delivered batch.
type Handler = Box<dyn Fn(&[Event]) + Send + Sync + 'static>;

/// A running FSEvents stream. Dropping it stops the stream and waits for any callback in flight.
pub(crate) struct Watcher {
    stream: FSEventStreamRef,
    queue: DispatchQueue,
    flush_lock: Mutex<()>,
}

// SAFETY: the stream and queue are opaque CoreServices/libdispatch objects whose functions may be
// called from any thread; `flush_lock` serialises the only `&self` operation and Drop has `&mut`.
unsafe impl Send for Watcher {}
// SAFETY: see above.
unsafe impl Sync for Watcher {}

impl Watcher {
    /// Watches `path` (and everything below it). `coalesce` is the FSEvents latency: after the
    /// first event of a burst is delivered, further events are batched for this long.
    pub(crate) fn start(path: &Path, coalesce: Duration, handler: impl Fn(&[Event]) + Send + Sync + 'static) -> io::Result<Self> {
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: `bytes` is valid for `bytes.len()` bytes during the call; a NULL allocator selects
        // the default allocator. The returned string is owned by us (Create rule) and released below.
        let cf_path = unsafe {
            CFStringCreateWithBytes(ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex, CF_STRING_ENCODING_UTF8, 0)
        };
        if cf_path.is_null() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "CFStringCreateWithBytes rejected the watch path"));
        }
        let values = [cf_path];
        // SAFETY: `values` holds one valid CFString; kCFTypeArrayCallBacks makes the array retain it,
        // so releasing our own reference right after is balanced.
        let paths = unsafe {
            let paths = CFArrayCreate(ptr::null(), values.as_ptr(), 1, ptr::addr_of!(kCFTypeArrayCallBacks));
            CFRelease(cf_path);
            paths
        };
        if paths.is_null() {
            return Err(io::Error::other("CFArrayCreate failed"));
        }

        let info = Box::into_raw(Box::new(Box::new(handler) as Handler));
        let mut context = FSEventStreamContext {
            version: 0,
            info: info.cast(),
            retain: None,
            release: Some(release_handler),
            copy_description: None,
        };
        // SAFETY: `context` is copied by FSEventStreamCreate; `info` stays valid until FSEvents calls
        // `release_handler` when the stream is deallocated. `paths` is a valid CFArray of CFStrings
        // that the stream retains, so we release our reference afterwards.
        let stream = unsafe {
            let stream = FSEventStreamCreate(
                ptr::null(),
                stream_callback,
                &mut context,
                paths,
                EVENT_ID_SINCE_NOW,
                coalesce.as_secs_f64(),
                CREATE_NO_DEFER | CREATE_WATCH_ROOT | CREATE_FILE_EVENTS,
            );
            CFRelease(paths);
            stream
        };
        if stream.is_null() {
            // `info` is deliberately leaked: whether FSEvents already invoked the release callback on
            // this failure path is undocumented, and a double free would be worse than a small leak.
            return Err(io::Error::other("FSEventStreamCreate failed"));
        }
        // SAFETY: the label is a NUL-terminated literal; NULL attributes create a serial queue.
        let queue = unsafe { dispatch_queue_create(c"dev.aegisflow.storage.fsevents".as_ptr(), ptr::null()) };
        if queue.is_null() {
            // SAFETY: `stream` was created above and never scheduled; releasing it frees `info`.
            unsafe { FSEventStreamRelease(stream) };
            return Err(io::Error::other("dispatch_queue_create failed"));
        }
        // SAFETY: `stream` and `queue` are valid; the stream retains the queue while scheduled.
        let started = unsafe {
            FSEventStreamSetDispatchQueue(stream, queue);
            FSEventStreamStart(stream)
        };
        if started == 0 {
            // SAFETY: tear down in the documented order for a scheduled but not started stream.
            unsafe {
                FSEventStreamInvalidate(stream);
                FSEventStreamRelease(stream);
                dispatch_release(queue);
            }
            return Err(io::Error::other("FSEventStreamStart failed"));
        }
        Ok(Self { stream, queue, flush_lock: Mutex::new(()) })
    }

    /// Delivers, before returning, every event that had already occurred when it was called
    /// (`FSEventStreamFlushSync`).
    pub(crate) fn flush(&self) {
        let _guard = self.flush_lock.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the stream is started and stays valid until Drop, which needs `&mut self`.
        unsafe { FSEventStreamFlushSync(self.stream) };
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: documented teardown order: stop, invalidate (no further callbacks are scheduled),
        // drain the serial queue so a callback already running has returned, then release. The
        // handler box is freed by `release_handler` when the stream is deallocated.
        unsafe {
            FSEventStreamStop(self.stream);
            FSEventStreamInvalidate(self.stream);
            dispatch_sync_f(self.queue, ptr::null_mut(), drain_queue);
            FSEventStreamRelease(self.stream);
            dispatch_release(self.queue);
        }
    }
}

unsafe extern "C" fn stream_callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    num_events: usize,
    event_paths: *mut c_void,
    event_flags: *const u32,
    _event_ids: *const u64,
) {
    if info.is_null() || event_flags.is_null() || event_paths.is_null() || num_events == 0 {
        return;
    }
    // SAFETY: `info` is the `Handler` box installed in `Watcher::start`, alive until
    // `release_handler`. Without kFSEventStreamCreateFlagUseCFTypes, `event_paths` is a C array
    // of `num_events` NUL-terminated strings and `event_flags` holds `num_events` flags; both
    // are valid for the duration of the callback.
    let (handler, flags, paths) = unsafe {
        (
            &*(info as *const Handler),
            std::slice::from_raw_parts(event_flags, num_events),
            std::slice::from_raw_parts(event_paths as *const *const c_char, num_events),
        )
    };
    let events: Vec<Event> = paths
        .iter()
        .zip(flags)
        .map(|(path, flags)| {
            let path = if path.is_null() {
                PathBuf::new()
            } else {
                // SAFETY: non-null entries are NUL-terminated strings owned by FSEvents (see above).
                PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(*path) }.to_bytes()))
            };
            Event { path, flags: *flags }
        })
        .collect();
    // A panic must not unwind into CoreServices; the handler only updates counters, so it is dropped.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&events)));
}

unsafe extern "C" fn release_handler(info: *const c_void) {
    if !info.is_null() {
        // SAFETY: `info` came from `Box::into_raw` in `Watcher::start` and FSEvents releases it once.
        drop(unsafe { Box::from_raw(info as *mut Handler) });
    }
}

unsafe extern "C" fn drain_queue(_context: *mut c_void) {}
