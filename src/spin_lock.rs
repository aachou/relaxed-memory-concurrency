#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(loom)]
use loom::hint::spin_loop;
#[cfg(not(loom))]
use std::hint::spin_loop;

pub struct SpinLock {
    inner: AtomicBool,
}

impl SpinLock {
    pub fn new() -> Self {
        SpinLock { inner: AtomicBool::new(false) }
    }

    pub fn lock(&self) {
        while self.inner.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spin_loop();
        }
    }

    pub fn unlock(&self, _token: ()) {
        self.inner.store(false, Ordering::Release);
    }
}

impl Default for SpinLock {
    fn default() -> Self {
        Self::new()
    }
}
