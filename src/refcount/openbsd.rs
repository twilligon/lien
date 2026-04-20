use core::sync::atomic::AtomicU64;

#[inline]
fn wait32(ptr: *const u32, expected: u32) {
    unsafe {
        libc::futex(
            ptr as *mut u32,
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            expected as _,
            core::ptr::null(),
            core::ptr::null_mut(),
        );
    }
}

#[inline]
fn wake32(ptr: *const u32) {
    unsafe {
        libc::futex(
            ptr as *mut u32,
            libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
            1,
            core::ptr::null(),
            core::ptr::null_mut(),
        );
    }
}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
