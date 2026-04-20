use core::sync::atomic::{AtomicU64, Ordering};

cfg_select! {
    target_pointer_width = "64" => {
        #[inline]
        pub fn wait(atom: &AtomicU64) {
            loop {
                match atom.load(Ordering::Acquire) {
                    0 => break,
                    v => unsafe {
                        let _ = syscall::syscall5(
                            syscall::SYS_FUTEX,
                            atom as *const AtomicU64 as usize,
                            syscall::FUTEX_WAIT64,
                            v as usize,
                            0,
                            0,
                        );
                    },
                }
            }
        }

        #[inline]
        pub fn wake(ptr: *const u64, old: u64) {
            if old == 1 {
                unsafe {
                    let _ = syscall::syscall5(
                        syscall::SYS_FUTEX,
                        ptr as usize,
                        syscall::FUTEX_WAKE,
                        1,
                        0,
                        0,
                    );
                }
            }
        }
    }
    target_pointer_width = "32" => {
        #[inline]
        fn wait32(ptr: *const u32, expected: u32) {
            unsafe {
                let _ = syscall::futex(
                    ptr as *mut i32,
                    syscall::FUTEX_WAIT,
                    expected as i32,
                    0,
                    core::ptr::null_mut(),
                );
            }
        }

        #[inline]
        fn wake32(ptr: *const u32) {
            unsafe {
                let _ = syscall::futex(
                    ptr as *mut i32,
                    syscall::FUTEX_WAKE,
                    1,
                    0,
                    core::ptr::null_mut(),
                );
            }
        }

        #[inline]
        pub fn wait(atom: &AtomicU64) { super::wait_hi_lo(atom, wait32) }

        #[inline]
        pub fn wake(ptr: *const u64, old: u64) { super::wake_hi_lo(ptr, old, wake32) }
    }
}
