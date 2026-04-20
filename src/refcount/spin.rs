use core::sync::atomic::{AtomicU64, Ordering};

#[inline]
pub fn wait(atom: &AtomicU64) {
    while atom.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
}

#[inline]
pub fn wake(_ptr: *const u64, _old: u64) {}
