//! Binding to `libtdjson` loaded at runtime with `dlopen` / `LoadLibrary`
//! (no C compiler, no build script, no linking against TDLib).
//!
//! Everything the rest of the crate needs from TDLib goes through the
//! [`Backend`] trait, so tests can substitute an in-process fake.

use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};

use crate::error::{Error, Result};

/// The four functions of TDLib's JSON interface.
pub trait Backend: Send + Sync {
    /// `td_create_client_id`: an opaque id for a new TDLib instance.
    fn create_client_id(&self) -> i32;
    /// `td_send`: queue a JSON request for the instance. Callable from any thread.
    fn send(&self, client_id: i32, request: &str);
    /// `td_receive`: next update or response, `None` when `timeout` (seconds) expires.
    /// Must only be called from one thread at a time.
    fn receive(&self, timeout: f64) -> Option<String>;
    /// `td_execute`: synchronously run a request that supports it.
    fn execute(&self, request: &str) -> Option<String>;
}

type CreateFn = unsafe extern "C" fn() -> c_int;
type SendFn = unsafe extern "C" fn(c_int, *const c_char);
type ReceiveFn = unsafe extern "C" fn(c_double) -> *const c_char;
type ExecuteFn = unsafe extern "C" fn(*const c_char) -> *const c_char;

/// A loaded `libtdjson`.
pub struct TdJson {
    create: CreateFn,
    send: SendFn,
    receive: ReceiveFn,
    execute: ExecuteFn,
}

#[cfg(unix)]
mod sys {
    use super::*;
    extern "C" {
        pub fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        pub fn dlerror() -> *const c_char;
    }
    #[cfg(target_os = "macos")]
    pub const RTLD_NOW_GLOBAL: c_int = 0x2 | 0x8;
    #[cfg(not(target_os = "macos"))]
    pub const RTLD_NOW_GLOBAL: c_int = 0x2 | 0x100;

    pub unsafe fn open(path: &CStr) -> std::result::Result<*mut c_void, String> {
        let h = dlopen(path.as_ptr(), RTLD_NOW_GLOBAL);
        if h.is_null() {
            Err(last_error())
        } else {
            Ok(h)
        }
    }
    pub unsafe fn symbol(h: *mut c_void, name: &CStr) -> std::result::Result<*mut c_void, String> {
        let p = dlsym(h, name.as_ptr());
        if p.is_null() {
            Err(last_error())
        } else {
            Ok(p)
        }
    }
    unsafe fn last_error() -> String {
        let e = dlerror();
        if e.is_null() {
            "unknown dlopen error".into()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::*;
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub unsafe fn open(path: &CStr) -> std::result::Result<*mut c_void, String> {
        let h = LoadLibraryA(path.as_ptr());
        if h.is_null() {
            Err(format!("LoadLibrary failed for {}", path.to_string_lossy()))
        } else {
            Ok(h)
        }
    }
    pub unsafe fn symbol(h: *mut c_void, name: &CStr) -> std::result::Result<*mut c_void, String> {
        let p = GetProcAddress(h, name.as_ptr());
        if p.is_null() {
            Err(format!("symbol {} not found", name.to_string_lossy()))
        } else {
            Ok(p)
        }
    }
}

/// Default shared-library file name for the current OS.
pub fn default_library_name() -> &'static str {
    if cfg!(windows) {
        "tdjson.dll"
    } else if cfg!(target_os = "macos") {
        "libtdjson.dylib"
    } else {
        "libtdjson.so"
    }
}

impl TdJson {
    /// Load the library from `path` (empty = the OS default name, resolved by
    /// the dynamic loader) and silence TDLib's own logging.
    pub fn load(path: &str) -> Result<TdJson> {
        let path = if path.is_empty() { default_library_name() } else { path };
        let cpath = CString::new(path).map_err(|_| Error::Library("library path contains NUL".into()))?;
        // SAFETY: the symbols are looked up by their documented names and cast to
        // their documented C signatures; the library handle is intentionally
        // leaked so the function pointers stay valid for the whole process.
        let lib = unsafe {
            let h = sys::open(&cpath).map_err(|e| Error::Library(format!("failed to load tdjson from {path}: {e}")))?;
            let get = |name: &str| -> Result<*mut c_void> {
                let n = CString::new(name).unwrap();
                sys::symbol(h, &n).map_err(|e| Error::Library(format!("{name}: {e}")))
            };
            TdJson {
                create: std::mem::transmute::<*mut c_void, CreateFn>(get("td_create_client_id")?),
                send: std::mem::transmute::<*mut c_void, SendFn>(get("td_send")?),
                receive: std::mem::transmute::<*mut c_void, ReceiveFn>(get("td_receive")?),
                execute: std::mem::transmute::<*mut c_void, ExecuteFn>(get("td_execute")?),
            }
        };
        lib.execute(r#"{"@type":"setLogStream","log_stream":{"@type":"logStreamEmpty"}}"#);
        Ok(lib)
    }
}

fn cstr_to_string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: TDLib returns a NUL-terminated string valid until the next call.
        // We copy it immediately.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

impl Backend for TdJson {
    fn create_client_id(&self) -> i32 {
        unsafe { (self.create)() }
    }

    fn send(&self, client_id: i32, request: &str) {
        if let Ok(c) = CString::new(request) {
            unsafe { (self.send)(client_id, c.as_ptr()) }
        }
    }

    fn receive(&self, timeout: f64) -> Option<String> {
        cstr_to_string(unsafe { (self.receive)(timeout) })
    }

    fn execute(&self, request: &str) -> Option<String> {
        let c = CString::new(request).ok()?;
        cstr_to_string(unsafe { (self.execute)(c.as_ptr()) })
    }
}
