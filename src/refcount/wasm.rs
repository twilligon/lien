use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "wasm32")]
use core::arch::wasm32 as wasm;
#[cfg(target_arch = "wasm64")]
use core::arch::wasm64 as wasm;

#[inline]
pub fn wait(atom: &AtomicU64) {
    loop {
        match atom.load(Ordering::Acquire) {
            0 => break,
            v => unsafe {
                wasm::memory_atomic_wait64(
                    (atom as *const AtomicU64).cast_mut().cast(),
                    v as i64,
                    -1,
                );
            },
        }
    }
}

#[inline]
pub fn wake(ptr: *const u64, old: u64) {
    if old == 1 {
        unsafe {
            wasm::memory_atomic_notify(ptr.cast_mut().cast(), 1);
        }
    }
}
