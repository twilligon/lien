use core::sync::atomic::AtomicU64;

unsafe extern "C" {
    fn zx_futex_wait(
        value_ptr: *const i32,
        current_value: i32,
        new_futex_owner: u32,
        deadline: i64,
    ) -> i32;

    fn zx_futex_wake(value_ptr: *const i32, wake_count: u32) -> i32;
}

#[inline]
fn wait32(ptr: *const u32, expected: u32) {
    unsafe {
        zx_futex_wait(ptr.cast(), expected as _, 0, i64::MAX);
    }
}

#[inline]
fn wake32(ptr: *const u32) {
    unsafe {
        zx_futex_wake(ptr.cast(), 1);
    }
}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
