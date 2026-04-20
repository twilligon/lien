// libc has the constants but not the __futex function.

use core::sync::atomic::AtomicU64;

unsafe extern "C" {
    fn __futex(
        uaddr: *const core::ffi::c_int,
        op: core::ffi::c_int,
        val: core::ffi::c_int,
        timeout: *const core::ffi::c_void,
        uaddr2: *const core::ffi::c_int,
        val2: core::ffi::c_int,
        val3: core::ffi::c_int,
    ) -> core::ffi::c_int;
}

#[inline]
fn wait32(ptr: *const u32, expected: u32) {
    unsafe {
        __futex(
            ptr.cast(),
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            expected as _,
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
        );
    }
}

#[inline]
fn wake32(ptr: *const u32) {
    unsafe {
        __futex(
            ptr.cast(),
            libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
            1,
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
        );
    }
}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
