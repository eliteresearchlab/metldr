//! The WASI calls PDFium's C library makes, answered inside the module under the symbol names
//! wasi-libc imports them by, so the linker resolves them here and the module needs no WASI host.
//! PDFium reads only from memory: files, directories and the environment do not exist, writes
//! to the standard streams are dropped, and the clock is the browser's.
//!
//! The only unsafe code in the crate: each write goes to an address PDFium passed for the
//! result, inside the module's own memory, as the WASI ABI defines.
#![allow(unsafe_code)]

const SUCCESS: i32 = 0;
const BADF: i32 = 8;
const NOENT: i32 = 44;
const NOSYS: i32 = 52;

/// Writes `value` at the module address `at`.
fn put<T>(at: i32, value: T) {
    // SAFETY: `at` is a result address PDFium owns in this module's memory, sized for `T`.
    unsafe { (at as usize as *mut T).write_unaligned(value) }
}

macro_rules! wasi {
    ($($name:ident($($arg:ident: $ty:ty),*) $body:block)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($($arg: $ty),*) -> i32 $body
    )*};
}

wasi! {
    __imported_wasi_snapshot_preview1_environ_get(_a: i32, _b: i32) { SUCCESS }
    __imported_wasi_snapshot_preview1_environ_sizes_get(count: i32, size: i32) {
        put(count, 0u32);
        put(size, 0u32);
        SUCCESS
    }
    __imported_wasi_snapshot_preview1_clock_time_get(_id: i32, _precision: i64, time: i32) {
        put(time, (js_sys::Date::now() * 1e6) as u64);
        SUCCESS
    }
    __imported_wasi_snapshot_preview1_fd_close(fd: i32) { if fd <= 2 { SUCCESS } else { BADF } }
    __imported_wasi_snapshot_preview1_fd_fdstat_get(fd: i32, stat: i32) {
        if fd > 2 {
            return BADF;
        }
        // A character device with no flags and no rights: 24 bytes.
        put(stat, [2u64, 0, 0]);
        SUCCESS
    }
    __imported_wasi_snapshot_preview1_fd_fdstat_set_flags(_fd: i32, _flags: i32) { NOSYS }
    __imported_wasi_snapshot_preview1_fd_filestat_get(_fd: i32, _stat: i32) { BADF }
    __imported_wasi_snapshot_preview1_fd_filestat_set_size(_fd: i32, _size: i64) { BADF }
    __imported_wasi_snapshot_preview1_fd_prestat_get(_fd: i32, _stat: i32) { BADF }
    __imported_wasi_snapshot_preview1_fd_prestat_dir_name(_fd: i32, _path: i32, _len: i32) { BADF }
    __imported_wasi_snapshot_preview1_fd_read(_fd: i32, _iovs: i32, _len: i32, _read: i32) { BADF }
    __imported_wasi_snapshot_preview1_fd_readdir(_fd: i32, _buf: i32, _len: i32, _cookie: i64, _used: i32) { BADF }
    __imported_wasi_snapshot_preview1_fd_seek(_fd: i32, _offset: i64, _whence: i32, _to: i32) { NOSYS }
    __imported_wasi_snapshot_preview1_fd_sync(_fd: i32) { NOSYS }
    __imported_wasi_snapshot_preview1_fd_write(fd: i32, iovs: i32, len: i32, written: i32) {
        if fd != 1 && fd != 2 {
            return BADF;
        }
        // Dropped, but reported written in full so PDFium carries on.
        let mut total = 0u32;
        for n in 0..len {
            // SAFETY: `iovs` holds `len` (pointer, length) pairs of u32 that PDFium wrote.
            total += unsafe { ((iovs + n * 8 + 4) as usize as *const u32).read_unaligned() };
        }
        put(written, total);
        SUCCESS
    }
    __imported_wasi_snapshot_preview1_path_filestat_get(_fd: i32, _flags: i32, _path: i32, _len: i32, _stat: i32) { NOENT }
    __imported_wasi_snapshot_preview1_path_open(_fd: i32, _dirflags: i32, _path: i32, _len: i32, _oflags: i32, _base: i64, _inheriting: i64, _fdflags: i32, _opened: i32) { NOENT }
    __imported_wasi_snapshot_preview1_path_remove_directory(_fd: i32, _path: i32, _len: i32) { NOSYS }
    __imported_wasi_snapshot_preview1_path_unlink_file(_fd: i32, _path: i32, _len: i32) { NOSYS }
}

#[unsafe(no_mangle)]
pub extern "C" fn __imported_wasi_snapshot_preview1_proc_exit(_code: i32) -> ! {
    core::arch::wasm32::unreachable()
}

// The C library PDFium links expects these; the module runs on one thread.
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_mutex_init(_mutex: i32, _attr: i32) -> i32 {
    0
}

macro_rules! no_lock {
    ($($name:ident),*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(_mutex: i32) -> i32 { 0 }
    )*};
}

no_lock!(pthread_mutex_lock, pthread_mutex_unlock, pthread_mutex_destroy);
