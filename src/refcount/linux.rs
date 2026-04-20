use core::sync::atomic::AtomicU64;

#[inline]
fn wait32(ptr: *const u32, expected: u32) {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            ptr,
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            expected,
            core::ptr::null::<core::ffi::c_void>(),
        );
    }
}

#[inline]
fn wake32(ptr: *const u32) {
    unsafe {
        libc::syscall(libc::SYS_futex, ptr, libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG, 1);
    }
}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
