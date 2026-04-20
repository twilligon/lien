// miri backend. why this exists instead of the real futex:
//
// the real backends pass *const u32 (from hi_ptr/lo_ptr) to kernel
// syscalls. the kernel operates outside the abstract machine, so it
// is the opinion of the authors that this is sound. but miri's futex
// shim simulates the wait as a rust-level u32 atomic read, which it
// flags as mixed-width UB against our AtomicU64. we never actually
// deref those pointers in rust, but we can't argue with miri.
//
// so we spin instead. wait32 returns immediately (a spurious wakeup)
// and wait_hi_lo re-checks via the u64 load. this is a valid sim:
// futexes permit spurious wakeups, and wake32 is a no-op because
// nobody is ever actually blocked. you can't miss a wake if nobody
// is sleeping. exercises the full hi/lo split logic without ever
// touching a u32 atomically.

use core::sync::atomic::AtomicU64;

#[inline]
fn wait32(_ptr: *const u32, _expected: u32) {
    core::hint::spin_loop();
}

#[inline]
fn wake32(_ptr: *const u32) {}

#[inline]
pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

#[inline]
pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
