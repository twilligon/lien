use core::sync::atomic::{AtomicU64, Ordering};

crate::cfg_select! {
    target_pointer_width = "64" => {
        #[inline]
        pub fn wait(atom: &AtomicU64) {
            loop {
                match atom.load(Ordering::Acquire) {
                    0 => break,
                    v => unsafe {
                        libc::_umtx_op(
                            (atom as *const AtomicU64) as *mut libc::c_void,
                            libc::UMTX_OP_WAIT,
                            v as libc::c_ulong,
                            core::ptr::null_mut(),
                            core::ptr::null_mut(),
                        );
                    },
                }
            }
        }

        #[inline]
        pub fn wake(ptr: *const u64, old: u64) {
            if old == 1 {
                unsafe {
                    libc::_umtx_op(
                        ptr as *mut libc::c_void,
                        libc::UMTX_OP_WAKE,
                        1 as libc::c_ulong,
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                    );
                }
            }
        }
    }
    target_pointer_width = "32" => {
        #[inline]
        fn wait32(ptr: *const u32, expected: u32) {
            unsafe {
                libc::_umtx_op(
                    ptr as *mut libc::c_void,
                    libc::UMTX_OP_WAIT_UINT_PRIVATE,
                    expected as libc::c_ulong,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
            }
        }

        #[inline]
        fn wake32(ptr: *const u32) {
            unsafe {
                libc::_umtx_op(
                    ptr as *mut libc::c_void,
                    libc::UMTX_OP_WAKE_PRIVATE,
                    1 as libc::c_ulong,
                    core::ptr::null_mut(),
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
