use core::sync::atomic::{AtomicU64, Ordering};

/// Macro-internal refcount backing [`Scope`](crate::Scope).
///
/// # Why is this `pub`?
///
/// The obvious API for `Scope` would be, say, `let scope = lien::Scope::new()`.
/// But this would defeat the purpose of `lien`: you could `mem::forget(scope)`
/// or `mem::replace(&scope, lien::Scope::new())` and all outstanding `Lien`s
/// would dangle.
///
/// Fine: we can define a macro `lien::scope!` which immediately borrows a new
/// `Scope` by reference. It's an immutable reference, so you can't safely take
/// the `Scope`. With [temporary lifetime extension][tle] (and very careful use
/// of [`PhantomData`](core::marker::PhantomData)), the underlying `Scope` will
/// live as long as it needs.
///
/// There's one more complication, which is that both a `Scope` and its `Lien`s
/// must access the `Scope`'s reference count. But the whole point of `lien` is
/// the `Scope` may be in the process of being dropped when waiting for `Lien`s
/// to be released, which means if the reference count were owned by `Scope` it
/// would be mutably borrowed by `Scope::drop` at the same time it is immutably
/// borrowed by `Lien`s. So we double down on temporary lifetime extension, and
/// have `Scope` hold a reference to a separate local for the reference count.
///
/// [tle]: https://blog.m-ou.se/super-let/
///
/// In the end, a use of [`scope!`](crate::scope!) like:
///
///
/// ```
/// let mut funny_number = 69;
///
/// {
///     let scope = lien::scope!();
///     let mut l = scope.lend_mut(&mut funny_number);
///     std::thread::spawn(move || *l = 67);
/// }
///
/// assert_eq!(funny_number, 67);
/// ```
///
/// expands to:
///
/// ```
/// let mut funny_number = 69;
///
/// {
///     let scope = &lien::Scope {
///         __rc: &lien::__Rc::new(),
///         __phantom: lien::__PhantomData,
///     };
///     let mut l = scope.lend_mut(&mut funny_number);
///     std::thread::spawn(move || *l = 67);
/// }
///
/// assert_eq!(funny_number, 67);
/// ```
///
/// Now [`Scope`](crate::Scope), `Scope`, [`__Rc`], and [`__Rc::new`] must
/// technically be `pub` so the macro can use them. They can't even be marked
/// `unsafe` because the macro's expansion can technically live in crates that
/// `#![forbid(unsafe_code)]`. But semantically, they're still private, outside
/// of the semver contract, and very unsafe: bypassing the macro and misusing a
/// `Scope` can cause undefined behavior from data races to use-after-frees!
///
/// ```no_run
/// # use core::marker::PhantomData;
/// # use std::{thread, mem};
/// let mut funny_number = 69;
///
/// {
///     let rc = lien::__Rc::new();
///     let scope = lien::Scope { __rc: &rc, __phantom: PhantomData };
///     let mut l = scope.lend_mut(&mut funny_number);
///
///     thread::spawn(move || *l = 67);
///
///     // yawn, waiting for that thread will take too long...
///     mem::forget(scope);
/// }
///
/// funny_number = 420;  // 💥 races with other thread!
/// ```
///
/// Theoretically, you *can* create a `Scope` directly, taking responsibility
/// for `Scope::drop` running:
///
/// ```
/// # use core::marker::PhantomData;
/// # use std::thread;
/// let mut funny_number = 69;
///
/// {
///     let rc = lien::__Rc::new();
///     let scope = lien::Scope { __rc: &rc, __phantom: PhantomData };
///     let mut l = scope.lend_mut(&mut funny_number);
///     thread::spawn(move || *l = 67);
/// }
///
/// assert_eq!(funny_number, 67);
/// ```
///
/// But you almost certainly want [`scope!`](crate::scope!) instead.
#[doc(hidden)]
#[repr(transparent)]
pub struct __Rc(AtomicU64);

impl __Rc {
    #[allow(clippy::new_without_default)]
    #[inline]
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    #[inline]
    pub(crate) fn inc(&self) {
        let _old_rc = self.0.fetch_add(1, Ordering::Relaxed);

        // we might want this always? but... come on, it's a u64
        #[cfg(any(debug_assertions, miri))]
        if _old_rc > u64::MAX / 2 {
            panic!("lien: refcount overflow");
        }
    }

    /// # Safety
    ///
    /// `this` must be live at the time of the call. It may dangle on return.
    #[inline]
    pub(crate) unsafe fn dec(this: *const Self) {
        // SAFETY: caller guarantees this is live for the fetch_sub.
        let old = unsafe { (*this).0.fetch_sub(1, Ordering::Release) };
        // this may dangle now if Scope woke spuriously. that's fine:
        // wake only uses the pointer as a futex address, never derefs
        imp::wake(this as *const u64, old);
    }

    #[inline]
    pub(crate) fn wait(&self) {
        imp::wait(&self.0);
    }
}

#[allow(dead_code)]
#[inline]
fn words(p: *const u64) -> [*const u32; 2] {
    [(p as *const u32), (p as *const u32).wrapping_add(1)]
}

crate::cfg_select! {
    target_endian = "little" => {
        #[allow(dead_code)]
        #[inline]
        fn hi_ptr(p: *const u64) -> *const u32 { words(p)[1] }

        #[allow(dead_code)]
        #[inline]
        fn lo_ptr(p: *const u64) -> *const u32 { words(p)[0] }
    }
    target_endian = "big" => {
        #[allow(dead_code)]
        #[inline]
        fn hi_ptr(p: *const u64) -> *const u32 { words(p)[0] }

        #[allow(dead_code)]
        #[inline]
        fn lo_ptr(p: *const u64) -> *const u32 { words(p)[1] }
    }
}

#[allow(dead_code)]
#[inline]
pub(super) fn wait_hi_lo(atom: &AtomicU64, wait32: impl Fn(*const u32, u32)) {
    let p = atom as *const AtomicU64 as *const u64;
    loop {
        let v = atom.load(Ordering::Acquire);
        match ((v >> 32) as u32, v as u32) {
            (0, 0) => break,
            // futex is only 32-bit (smh linus!), so watch the nonzero half
            (hi, 0) => wait32(hi_ptr(p), hi),
            (_, lo) => wait32(lo_ptr(p), lo),
        }
    }
}

#[allow(dead_code)]
#[inline]
pub(super) fn wake_hi_lo(ptr: *const u64, old: u64, wake32: impl Fn(*const u32)) {
    match old {
        0x0000_0001_0000_0000 => wake32(hi_ptr(ptr)), // "go wait on lo"
        0x0000_0000_0000_0001 => wake32(lo_ptr(ptr)), // "no more Liens"
        _ => {}
    }
}

crate::cfg_select! {
    miri => {
        #[path = "refcount/miri.rs"]
        mod imp;
    }
    any(target_os = "linux", target_os = "android") => {
        #[path = "refcount/linux.rs"]
        mod imp;
    }
    target_family = "wasm" => {
        #[path = "refcount/wasm.rs"]
        mod imp;
    }
    target_os = "dragonfly" => {
        #[path = "refcount/dragonfly.rs"]
        mod imp;
    }
    target_os = "freebsd" => {
        #[path = "refcount/freebsd.rs"]
        mod imp;
    }
    target_os = "fuchsia" => {
        #[path = "refcount/fuchsia.rs"]
        mod imp;
    }
    target_os = "netbsd" => {
        #[path = "refcount/netbsd.rs"]
        mod imp;
    }
    target_os = "openbsd" => {
        #[path = "refcount/openbsd.rs"]
        mod imp;
    }
    target_os = "redox" => {
        #[path = "refcount/redox.rs"]
        mod imp;
    }
    target_vendor = "apple" => {
        #[path = "refcount/macos.rs"]
        mod imp;
    }
    target_family = "windows" => {
        #[path = "refcount/windows.rs"]
        mod imp;
    }
    feature = "spin" => {
        #[path = "refcount/spin.rs"]
        mod imp;
    }
    _ => {
        compile_error!(
            "lien: no futex implementation for this platform; \
             enable the `spin` feature for a (costly) fallback"
        );
    }
}
