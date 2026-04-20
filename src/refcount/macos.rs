// runtime fallback chain for apple futex support:
//
//   OsSync:  os_sync_wait_on_address  (macOS 14.4+)
//   Ulock64: __ulock_wait2 + COMPARE_AND_WAIT64  (macOS 11+)
//   Ulock32: __ulock_wait  + COMPARE_AND_WAIT    (macOS 10.12+, hi/lo split)
//
// LazyFutexImpl is basically a no_std LazyLock<FutexImpl>. calling
// get() resolves the best available impl via dlsym on first access,
// caching it atomically. dlsym is deterministic per-process, so
// racing callers resolve the same pointers and the stores are
// idempotent — we don't care who wins.
//
// see https://github.com/rust-lang/rust/pull/122408 for the std
// equivalent and app store analysis.

use core::ffi::{c_void, CStr};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU8, AtomicU64, Ordering};

const UL_COMPARE_AND_WAIT: u32 = 1;
const UL_COMPARE_AND_WAIT64: u32 = 5;
const ULF_NO_ERRNO: u32 = 0x0100_0000;

const UNINIT: u8 = 0;
const OS_SYNC: u8 = 1;
const ULOCK64: u8 = 2;
const ULOCK32: u8 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
enum FutexImpl {
    OsSync {
        os_sync_wait_on_address: unsafe extern "C" fn(*mut c_void, u64, usize, u32) -> i32,
        os_sync_wake_by_address_any: unsafe extern "C" fn(*mut c_void, usize, u32) -> i32,
    },
    Ulock64 {
        __ulock_wait2: unsafe extern "C" fn(u32, *mut c_void, u64, u64, u64) -> i32,
        __ulock_wake: unsafe extern "C" fn(u32, *mut c_void, u64) -> i32,
    },
    Ulock32 {
        __ulock_wait: unsafe extern "C" fn(u32, *mut c_void, u64, u32) -> i32,
        __ulock_wake: unsafe extern "C" fn(u32, *mut c_void, u64) -> i32,
    },
}

struct LazyFutexImpl {
    impt: AtomicU8,
    wait: AtomicPtr<c_void>,
    wake: AtomicPtr<c_void>,
}

impl LazyFutexImpl {
    #[inline]
    fn dlsym(&self, ty: u8, wait_name: &CStr, wake_name: &CStr) -> bool {
        let wait = unsafe { libc::dlsym(libc::RTLD_DEFAULT, wait_name.as_ptr()) };
        if wait.is_null() { return false; }
        self.wait.store(wait, Ordering::Relaxed);

        let wake = unsafe { libc::dlsym(libc::RTLD_DEFAULT, wake_name.as_ptr()) };
        if wake.is_null() { return false; }
        self.wake.store(wake, Ordering::Relaxed);
        
        self.impt.store(ty, Ordering::Release);

        true
    }

    #[inline]
    fn get(&self) -> FutexImpl {
        loop {
            match self.impt.load(Ordering::Acquire) {
                UNINIT => {
                    assert!(
                        self.dlsym(OS_SYNC, c"os_sync_wait_on_address", c"os_sync_wake_by_address_any")
                        || self.dlsym(ULOCK64, c"__ulock_wait2", c"__ulock_wake")
                        || self.dlsym(ULOCK32, c"__ulock_wait", c"__ulock_wake"),
                        "lien: no futex available (macOS < 10.12?)"
                    );
                }
                OS_SYNC => return FutexImpl::OsSync {
                    os_sync_wait_on_address: unsafe { mem::transmute(self.wait.load(Ordering::Relaxed)) },
                    os_sync_wake_by_address_any: unsafe { mem::transmute(self.wake.load(Ordering::Relaxed)) },
                },
                ULOCK64 => return FutexImpl::Ulock64 {
                    __ulock_wait2: unsafe { mem::transmute(self.wait.load(Ordering::Relaxed)) },
                    __ulock_wake: unsafe { mem::transmute(self.wake.load(Ordering::Relaxed)) },
                },
                ULOCK32 => return FutexImpl::Ulock32 {
                    __ulock_wait: unsafe { mem::transmute(self.wait.load(Ordering::Relaxed)) },
                    __ulock_wake: unsafe { mem::transmute(self.wake.load(Ordering::Relaxed)) },
                },
                _ => unreachable!(),
            }
        }
    }
}

static IMPL: LazyFutexImpl = LazyFutexImpl {
    impt: AtomicU8::new(UNINIT),
    wait: AtomicPtr::new(ptr::null_mut()),
    wake: AtomicPtr::new(ptr::null_mut()),
};

#[inline]
pub fn wait(atom: &AtomicU64) {
    match IMPL.get() {
        FutexImpl::OsSync { os_sync_wait_on_address, .. } => {
            loop {
                match atom.load(Ordering::Acquire) {
                    0 => break,
                    v => unsafe { os_sync_wait_on_address(atom.as_ptr().cast(), v, mem::size_of::<u64>(), 0); },
                }
            }
        }
        FutexImpl::Ulock64 { __ulock_wait2, .. } => {
            loop {
                match atom.load(Ordering::Acquire) {
                    0 => break,
                    v => unsafe { __ulock_wait2(UL_COMPARE_AND_WAIT64 | ULF_NO_ERRNO, atom.as_ptr().cast(), v, 0, 0); },
                }
            }
        }
        FutexImpl::Ulock32 { __ulock_wait, .. } => {
            super::wait_hi_lo(atom, |ptr, expected| unsafe {
                __ulock_wait(UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, ptr as *mut c_void, expected as u64, 0);
            });
        }
    }
}

#[inline]
pub fn wake(ptr: *const u64, old: u64) {
    match IMPL.get() {
        FutexImpl::OsSync { os_sync_wake_by_address_any, .. } => {
            if old != 1 { return; }
            unsafe { os_sync_wake_by_address_any(ptr as *mut c_void, mem::size_of::<u64>(), 0); }
        }
        FutexImpl::Ulock64 { __ulock_wake, .. } => {
            if old != 1 { return; }
            unsafe { __ulock_wake(UL_COMPARE_AND_WAIT64 | ULF_NO_ERRNO, ptr as *mut c_void, 0); }
        }
        FutexImpl::Ulock32 { __ulock_wake, .. } => {
            super::wake_hi_lo(ptr, old, |p| unsafe {
                __ulock_wake(UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, p as *mut c_void, 0);
            });
        }
    }
}
