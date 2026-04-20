use core::sync::atomic::AtomicU64;

#[inline]
fn wait32(ptr: *const u32, expected: u32) {
    unsafe {
        libc::umtx_sleep(ptr.cast(), expected as _, 0);
    }
}

#[inline]
fn wake32(ptr: *const u32) {
    unsafe {
        libc::umtx_wakeup(ptr.cast(), 1);
    }
}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
