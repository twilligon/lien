use core::sync::atomic::{AtomicU64, Ordering};

#[inline]
pub fn wait(atom: &AtomicU64) {
    loop {
        match atom.load(Ordering::Acquire) {
            0 => break,
            v => unsafe {
                windows_sys::Win32::System::Threading::WaitOnAddress(
                    (atom as *const AtomicU64).cast(),
                    (&v as *const u64).cast(),
                    core::mem::size_of::<u64>(),
                    u32::MAX,
                );
            },
        }
    }
}

#[inline]
pub fn wake(ptr: *const u64, old: u64) {
    if old == 1 {
        unsafe {
            windows_sys::Win32::System::Threading::WakeByAddressSingle(ptr.cast());
        }
    }
}
