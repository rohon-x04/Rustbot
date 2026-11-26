//! Wait for SIGINT / SIGTERM without external crates.

use std::time::Duration;

#[cfg(unix)]
mod imp {
    use std::ffi::c_int;
    use std::sync::atomic::{AtomicBool, Ordering};

    static TERMINATED: AtomicBool = AtomicBool::new(false);

    extern "C" {
        fn signal(signum: c_int, handler: usize) -> usize;
    }

    extern "C" fn on_signal(_: c_int) {
        TERMINATED.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe {
            signal(2, on_signal as *const () as usize); // SIGINT
            signal(15, on_signal as *const () as usize); // SIGTERM
        }
    }

    pub fn triggered() -> bool {
        TERMINATED.load(Ordering::SeqCst)
    }
}

#[cfg(not(unix))]
mod imp {
    pub fn install() {}
    pub fn triggered() -> bool {
        false
    }
}

/// Block until a termination signal arrives or `done()` returns true.
pub fn wait_for_termination(done: impl Fn() -> bool) {
    imp::install();
    while !imp::triggered() && !done() {
        std::thread::sleep(Duration::from_millis(100));
    }
}
