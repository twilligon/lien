//! Scoped lending of borrowed references as [`Send`]-able smart pointers.
//!
//! ```
//! # use std::thread;
//! let mut greeting = String::from("hello ");
//!
//! {
//!     let scope = lien::scope!();
//!     let mut g = scope.lend_mut(&mut greeting);
//!
//!     thread::spawn(move || {
//!         g.push_str("beautiful ");
//!     });
//! }
//!
//! greeting.push_str("world");
//! assert_eq!(greeting, "hello beautiful world");
//! ```
//!
//! A *[lien][wikt]* is "a right to take possession of a debtor's property as
//! security until a debt or duty is discharged."
//!
//! Similarly, a [`Lien`] represents a borrow of something from a [`Scope`].
//! Like a static borrow with `&` or `&mut`, this forces the `Scope` to outlive
//! any `Lien`s made from it, but like an `Arc`, `Lien` carries no lifetime
//! (it's atomically reference-counted at runtime), is thread-safe, and can be
//! freely cloned.
//!
//! # Smart pointers
//!
//! - [`Lien`]: a bare scope token.
//! - [`Ref`]: a sendable shared reference (like `&T`).
//! - [`RefMut`]: a sendable exclusive reference (like `&mut T`).
//!
//! Both `Ref` and `RefMut` support sub-borrowing through [`Ref::map`] /
//! [`RefMut::map`], which lets you re-lend fields against the original scope.
//!
//! # `#[no_std]`
//!
//! Disable the `std` feature:
//!
//! ```toml
//! [dependencies]
//! lien = { version = "0.1", default-features = false }
//! ```
//!
//! [wikt]: https://en.wiktionary.org/wiki/lien#English:_legal_claim

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_family = "wasm", feature(stdarch_wasm_atomic_wait))]

use core::{
    borrow::{Borrow, BorrowMut},
    cmp,
    error::Error,
    fmt,
    future::Future,
    hash::{Hash, Hasher},
    iter::FusedIterator,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

macro_rules! cfg_select {
    (@apply [$cond:meta] { $($body:item)* }) => {
        $( #[cfg($cond)] $body )*
    };
    (@munch [$($prev:meta),*] _ => { $($body:item)* }) => {
        $crate::cfg_select!(@apply [not(any($($prev),*))] { $($body)* });
    };
    (@munch [$($prev:meta),*] $cond:meta => { $($body:item)* } $($rest:tt)*) => {
        $crate::cfg_select!(@apply [all($cond $(, not($prev))*)] { $($body)* });
        $crate::cfg_select!(@munch [$cond $(, $prev)*] $($rest)*);
    };
    (@munch [$($prev:meta),*]) => {};
    ($($tokens:tt)*) => { $crate::cfg_select!(@munch [] $($tokens)*); };
}
pub(crate) use cfg_select;

mod refcount;
/// Macro-internal and unsafe to touch; see [`__Rc`].
pub use refcount::__Rc;

/// A claim that holds a [`Scope`] open.
///
/// A *[lien][wikt]* is "a right to take possession of a debtor's property as
/// security until a debt or duty is discharged."
///
/// Similarly, a `Lien` represents a borrow of something from a `Scope`. Like a
/// static borrow with `&` or `&mut`, this forces the `Scope` to outlive any
/// `Lien`s made from it. But like an `Arc`, `Lien` carries no lifetime (it's
/// atomically reference-counted at runtime), is thread-safe, and can be freely
/// cloned.
///
/// You usually won't need `Lien` directly. A bare `Lien` isn't associated with
/// any specific resource from a `Scope`; it's most useful as a building block
/// for structured loan-backed products---sorry---for smart pointers such as
/// [`Ref`] and [`RefMut`] which hold `Lien`s internally.
///
/// See [`Scope::lien`] for details and examples.
///
/// [wikt]: https://en.wiktionary.org/wiki/lien#English:_legal_claim
#[must_use]
pub struct Lien {
    rc: NonNull<__Rc>,
}

impl Lien {
    #[inline]
    fn new(rc: &__Rc) -> Self {
        rc.inc();
        Self {
            rc: NonNull::from(rc),
        }
    }
}

// SAFETY: pointee is atomic and outlives any Lien
unsafe impl Send for Lien {}
unsafe impl Sync for Lien {}

impl fmt::Debug for Lien {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lien").finish_non_exhaustive()
    }
}

impl Clone for Lien {
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: self.rc is alive because Scope blocks until *self.rc == 0,
        // which can't happen until we are dropped at the earliest
        Self::new(unsafe { self.rc.as_ref() })
    }
}

impl Drop for Lien {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: self.rc is alive because Scope blocks until *self.rc == 0
        unsafe { __Rc::dec(self.rc.as_ptr()) }
        // self.rc may dangle now (if last Lien). that's fine, it has no Drop
    }
}

/// A guard for the current scope to lend resources as long-lived borrows.
///
/// [`Lien`]s, [`Ref`]s, and [`RefMut`]s carry no lifetime information, so it
/// can't be proven statically they will be returned by the end of any given
/// lexical scope (in direct contrast to a regular `&` or `&mut` borrow). But
/// [`Ref`] and [`RefMut`] point to data in the scope of the [`Scope::lend`] or
/// [`Scope::lend_mut`] that created them, so that scope must not end while such
/// references (or any other `Lien`s) are alive.
///
/// `Scope`'s `Drop` implementation blocks, keeping locals/stack data alive
/// until all `Lien`s derived from ones it minted via [`lien`](Scope::lien),
/// [`lend`](Scope::lend), or [`lend_mut`](Scope::lend_mut) have been dropped.
/// If the scope itself is long-lived, this may be a no-op. But especially if
/// `Lien`s escape onto other threads, `Scope` is your lender of last resort.
///
/// Use [`lien::scope!`](scope!) to create a scope, then use
/// [`scope.lend()`](Scope::lend) or [`scope.lend_mut()`](Scope::lend_mut) to
/// lend references.
///
/// ```
/// # use std::{thread, time::Duration};
/// let mut greeting = String::from("hello ");
///
/// {
///     let scope = lien::scope!();
///     let mut g = scope.lend_mut(&mut greeting);
///
///     thread::spawn(move || {
///         thread::sleep(Duration::from_secs(1));
///         g.push_str("beautiful ");
///     });
///
///     // `scope` drops here, blocking until the thread drops `g`
/// }
///
/// greeting.push_str("world");
/// assert_eq!(greeting, "hello beautiful world");
/// ```
///
/// Can't read a value while it's mutably lent:
/// ```compile_fail
/// # use lien::*;
/// let mut v = 42;
/// let scope = scope!();
/// let r = scope.lend_mut(&mut v);
/// println!("{v}"); // 💥 still borrowed
/// ```
///
/// Can't mutate a value while it's immutably lent:
/// ```compile_fail
/// # use lien::*;
/// let mut v = 42;
/// let scope = scope!();
/// let r = scope.lend(&v);
/// v = 0; // 💥 still borrowed
/// ```
///
/// Can't lend a value that doesn't live long enough:
/// ```compile_fail
/// # use lien::*;
/// let scope = scope!();
/// let r = {
///     let short = 42;
///     scope.lend(&short) // 💥 `short` dropped at end of block
/// };
/// ```
#[must_use]
pub struct Scope<'a> {
    /// Macro-internal and unsafe to touch; see [`__Rc`].
    ///
    // &'a __Rc and not __Rc so Drop::drop(&mut self) doesn't borrow
    // self.__rc as &mut just as outstanding Liens may borrow
    // self.rc.as_ref(), as that'd be aliasing between shared and
    // exclusive references, a big no-no under Stacked Borrows/Tree
    // Borrows/miri and all around obvious bad vibes
    #[doc(hidden)]
    pub __rc: &'a __Rc,

    /// Macro-internal and unsafe to touch; see [`__Rc`].
    // invariant lifetime holds onto any borrow the Scope lends via
    // lend{,_mut}

    #[doc(hidden)]
    pub __phantom: PhantomData<fn(&'a ()) -> &'a ()>,
}

impl<'a> Scope<'a> {
    /// Creates a [`Lien`] that holds this scope open.
    ///
    /// The scope's drop blocks until every outstanding `Lien` is dropped, so
    /// the scope is guaranteed to outlive the `Lien`.
    ///
    /// ```
    /// # use std::{thread, time::{Duration, Instant}};
    /// let t = Instant::now();
    ///
    /// {
    ///     let scope = lien::scope!();
    ///     let lien = scope.lien();
    ///     thread::spawn(move || {
    ///         thread::sleep(Duration::from_secs(1));
    ///         drop(lien);
    ///     });
    /// } // blocks here until the spawned thread drops its lien
    ///
    /// assert!(t.elapsed() >= Duration::from_secs(1));
    /// ```
    ///
    /// This lets you keep a raw pointer valid across a thread boundary:
    ///
    /// ```
    /// # use std::thread;
    /// let value = 42u32;
    ///
    /// {
    ///     let scope = lien::scope!();
    ///     let addr = &value as *const u32 as usize;
    ///     let lien = scope.lien();
    ///
    ///     thread::spawn(move || {
    ///         // SAFETY: the lien holds the scope open, keeping `value` alive.
    ///         assert_eq!(unsafe { *(addr as *const u32) }, 42);
    ///         drop(lien);
    ///     });
    /// } // blocks until the lien is dropped
    /// ```
    ///
    /// [`Ref`] and [`RefMut`] wrap this pattern safely. Prefer
    /// [`lend`](Scope::lend) and [`lend_mut`](Scope::lend_mut) unless you are
    /// building your own such wrapper.
    #[inline]
    pub fn lien(&self) -> Lien {
        Lien::new(self.__rc)
    }

    /// Lends a shared reference, returning a [`Ref`] that can be sent
    /// elsewhere. The borrow is anchored to this guard's lifetime.
    ///
    /// ```
    /// let v = vec![1, 2, 3];
    ///
    /// let scope = lien::scope!();
    /// let r = scope.lend(&v);
    ///
    /// std::thread::spawn(move || assert_eq!(&*r, &[1, 2, 3]));
    /// ```
    #[inline]
    pub fn lend<T: ?Sized>(&self, value: &'a T) -> Ref<T> {
        Ref {
            ptr: NonNull::from(value),
            _lien: self.lien(),
        }
    }

    /// Lends an exclusive reference as a [`RefMut`] that can be sent elsewhere.
    /// The borrow is anchored to this guard's lifetime.
    ///
    /// ```
    /// # use std::{thread, time::Duration};
    /// let mut value = String::from("hello");
    ///
    /// {
    ///     let scope = lien::scope!();
    ///     let mut r = scope.lend_mut(&mut value);
    ///
    ///     thread::spawn(move || {
    ///         thread::sleep(Duration::from_secs(1));
    ///         r.push_str(" world");
    ///     });
    /// }
    ///
    /// assert_eq!(value, "hello world");
    /// ```
    #[inline]
    pub fn lend_mut<T: ?Sized>(&self, value: &'a mut T) -> RefMut<T> {
        RefMut {
            ptr: NonNull::from(value),
            _lien: self.lien(),
            _phantom: PhantomData,
        }
    }

    // per rust reference items.fn.extern.unwind and RFC 2945, unwinding out of
    // extern "C" frames is defined to abort, and forced unwinding through such
    // a frame is UB on the unwinder's part---which is the best assignment-of-
    // blame-to-anyone-but-us we can achieve on stable. inlining doesn't change
    // this, so the end result is any unwind out of self.__rc.wait() becomes a
    // process abort regardless of panic strategy or std-ness, a property quite
    // needed as Scope::drop must NEVER be unwound past. now the ball is in the
    // hypothetical evil panic handler's court, as it's explicitly forbidden...
    //
    // the "real" argument is the above, not "it works on my machine", but fwiw
    // rustc DOES insert a .gcc_except_table entry to a landing pad to an abort
    // intrinsic (just ud2 or the like) even when this fn has been inlined away
    #[inline(always)]
    extern "C" fn wait(&self) {
        self.__rc.wait()
    }

    // TODO: an async fn wait could make lien usable in async contexts (w/ sync
    // Drop fallback) but would require a different data layout (storing waiter
    // future, etc.) so we'd need to make things generic and/or have a separate
    // async-lien crate
}

impl Drop for Scope<'_> {
    #[inline]
    fn drop(&mut self) {
        self.wait();
    }
}

/// A helper for [`Ref::map`] and [`RefMut::map`] that re-lends from existing
/// [`Ref`]s or [`RefMut`]s.
///
/// Use it to split a `Ref` or `RefMut` into smaller pieces that can be sent
/// independently:
///
/// ```
/// # use lien::RefMut;
/// # use std::thread;
/// struct User {
///     name: String,
///     bio: String,
/// }
///
/// fn question(mut s: impl AsMut<String>) {
///     s.as_mut().push_str("(?)");
/// }
///
/// fn question_everything(user: RefMut<User>) {
///     user.map(|u, rehyp| {
///         let name = rehyp.lend_mut(&mut u.name);
///         let bio = rehyp.lend_mut(&mut u.bio);
///         thread::spawn(move || question(name));
///         thread::spawn(move || question(bio));
///     });
/// }
///
/// let mut user = User {
///     name: "Elux Troxl".into(),
///     bio: "Guano salesman".into(),
/// };
///
/// {
///     let scope = lien::scope!();
///     question_everything(scope.lend_mut(&mut user));
/// }
///
/// assert_eq!(user.name, "Elux Troxl(?)");
/// assert_eq!(user.bio, "Guano salesman(?)");
/// ```
///
/// The invariant lifetime `'a` is tied via HRTB to the reference provided by
/// the `map` closure, preventing unrelated references from being lent:
///
/// ```compile_fail
/// # use lien::*;
/// let value = 42;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// r.map(|_v, rehyp| {
///     let local = 99;
///     rehyp.lend(&local); // 💥 `local` doesn't live long enough
/// });
/// ```
///
/// ```compile_fail
/// # use lien::*;
/// let value = 42;
/// let other = 99;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// r.map(|_v, rehyp| {
///     rehyp.lend(&other); // 💥 wrong lifetime
/// });
/// ```
///
/// ```compile_fail
/// # use lien::*;
/// let value = 42;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// let escaped = r.map(|_v, rehyp| rehyp); // 💥 can't escape
/// ```
///
/// ```compile_fail
/// # use lien::*;
/// let value = 42;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// let mut stash = None;
/// r.map(|_v, rehyp| {
///     stash = Some(rehyp); // 💥 can't escape
/// });
/// ```
///
/// ```compile_fail
/// # use lien::*;
/// let value = 42;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// let escaped: &i32 = r.map(|v, _rehyp| v); // 💥 can't escape &'a T
/// ```
pub struct Rehypothecator<'a> {
    lien: &'a Lien,
    _phantom: PhantomData<fn(&'a ()) -> &'a ()>,
}

impl<'a> Rehypothecator<'a> {
    /// Creates a [`Lien`] scope token against the original scope's rc.
    #[inline]
    pub fn lien(&self) -> Lien {
        self.lien.clone()
    }

    /// Lends a shared reference, returning a [`Ref`] backed by the original
    /// scope.
    #[inline]
    pub fn lend<T: ?Sized>(&self, value: &'a T) -> Ref<T> {
        Ref {
            ptr: NonNull::from(value),
            _lien: self.lien(),
        }
    }

    /// Lends an exclusive reference, returning a [`RefMut`] backed by the
    /// original scope.
    #[inline]
    pub fn lend_mut<T: ?Sized>(&self, value: &'a mut T) -> RefMut<T> {
        RefMut {
            ptr: NonNull::from(value),
            _lien: self.lien(),
            _phantom: PhantomData,
        }
    }
}

macro_rules! impl_deref_traits {
    ($ty:ident) => {
        impl<T: ?Sized> Deref for $ty<T> {
            type Target = T;
            #[inline]
            fn deref(&self) -> &T {
                // SAFETY: the Scope blocks until all Liens are dropped,
                // so *self.ptr is alive for the smart pointer's lifetime.
                unsafe { self.ptr.as_ref() }
            }
        }

        impl<T: ?Sized> AsRef<T> for $ty<T> {
            #[inline]
            fn as_ref(&self) -> &T {
                self
            }
        }

        impl<T: ?Sized> Borrow<T> for $ty<T> {
            #[inline]
            fn borrow(&self) -> &T {
                self
            }
        }

        impl<T: fmt::Debug + ?Sized> fmt::Debug for $ty<T> {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                (**self).fmt(f)
            }
        }

        impl<T: fmt::Display + ?Sized> fmt::Display for $ty<T> {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                (**self).fmt(f)
            }
        }

        impl<T: ?Sized> fmt::Pointer for $ty<T> {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Pointer::fmt(&self.ptr.as_ptr(), f)
            }
        }

        impl<T: Hash + ?Sized> Hash for $ty<T> {
            #[inline]
            fn hash<H: Hasher>(&self, state: &mut H) {
                (**self).hash(state)
            }
        }

        impl<T: PartialEq + ?Sized> PartialEq for $ty<T> {
            #[inline]
            fn eq(&self, other: &Self) -> bool {
                **self == **other
            }
        }

        impl<T: Eq + ?Sized> Eq for $ty<T> {}

        impl<T: PartialOrd + ?Sized> PartialOrd for $ty<T> {
            #[inline]
            fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
                (**self).partial_cmp(&**other)
            }
        }

        impl<T: Ord + ?Sized> Ord for $ty<T> {
            #[inline]
            fn cmp(&self, other: &Self) -> cmp::Ordering {
                (**self).cmp(&**other)
            }
        }

        impl<T: Error + ?Sized> Error for $ty<T> {
            #[inline]
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                (**self).source()
            }
        }
    };
}

/// A sendable shared reference backed by a [`Lien`].
///
/// Similarly, a `Ref<T>` represents a borrow of `T` from a [`Scope`]. Like a
/// static borrow with `&T`, this forces the `Scope` to outlive any `Ref`s made
/// from it. But like an `Arc`, `Ref<T>` carries no lifetime (it's atomically
/// reference-counted at runtime), is thread-safe, and can be freely cloned.
///
/// `Ref<T>` requires `T: Sync` to be `Send`, like `&T`:
/// ```compile_fail
/// # use lien::*;
/// use std::cell::Cell;
/// fn assert_send<T: Send>(_: T) {}
/// let v = Cell::new(42);
/// let scope = scope!();
/// let r = scope.lend(&v);
/// assert_send(r); // 💥 Cell is not Sync
/// ```
#[must_use]
pub struct Ref<T: ?Sized> {
    ptr: NonNull<T>,
    _lien: Lien,
}

// SAFETY: Ref<T> is morally &T, trivially Send but cross-thread needs T: Sync
unsafe impl<T: Sync + ?Sized> Send for Ref<T> {}
unsafe impl<T: Sync + ?Sized> Sync for Ref<T> {}

impl<T: ?Sized> Clone for Ref<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            _lien: self._lien.clone(),
        }
    }
}

impl<T: ?Sized> Ref<T> {
    /// Re-lends this `Ref`'s contents (or sub-fields) against the original
    /// scope's rc.
    #[inline]
    pub fn map<R>(&self, f: impl for<'a> FnOnce(&'a T, Rehypothecator<'a>) -> R) -> R {
        f(
            // SAFETY: ptr is valid because the lien keeps the scope alive.
            unsafe { self.ptr.as_ref() },
            Rehypothecator {
                lien: &self._lien,
                _phantom: PhantomData,
            },
        )
    }

    /// Returns a raw pointer to the `Ref`'s referent.
    ///
    /// The caller must ensure that the `Ref` outlives the pointer this
    /// function returns, or else it may end up dangling.
    #[inline]
    #[must_use]
    pub fn as_ptr(this: &Self) -> *const T {
        this.ptr.as_ptr()
    }
}

impl_deref_traits!(Ref);

impl<T: ?Sized> From<RefMut<T>> for Ref<T> {
    #[inline]
    fn from(r: RefMut<T>) -> Self {
        Self {
            ptr: r.ptr,
            _lien: r._lien,
        }
    }
}

/// A sendable exclusive reference backed by a [`Lien`].
///
/// Similarly, a `RefMut<T>` represents an exclusive borrow of `T` from a
/// [`Scope`]. Like a static borrow with `&mut T`, this forces the `Scope` to
/// outlive any `RefMut`s made from it. But like an `Arc`, `RefMut<T>` carries
/// no lifetime (it's atomically reference-counted at runtime) and is
/// thread-safe.
///
/// `RefMut<T>` requires `T: Send` to be `Send`, like `&mut T`:
/// ```compile_fail
/// # use lien::*;
/// use std::rc::Rc;
/// fn assert_send<T: Send>(_: T) {}
/// let mut v = Rc::new(42);
/// let scope = scope!();
/// let r = scope.lend_mut(&mut v);
/// assert_send(r); // 💥 Rc is not Send
/// ```
///
/// Can't clone a `RefMut`:
/// ```compile_fail
/// # use lien::*;
/// fn needs_clone<T: Clone>(_: &T) {}
/// let mut v = 42;
/// let scope = scope!();
/// let r = scope.lend_mut(&mut v);
/// needs_clone(&r); // 💥 RefMut is not Clone
/// ```
///
/// Can't have two `RefMut`s to the same value:
/// ```compile_fail
/// # use lien::*;
/// let mut v = 42;
/// let scope = scope!();
/// let r1 = scope.lend_mut(&mut v);
/// let r2 = scope.lend_mut(&mut v); // 💥 already borrowed mutably
/// drop(r1);
/// ```
#[must_use]
pub struct RefMut<T: ?Sized> {
    ptr: NonNull<T>,
    _lien: Lien,
    _phantom: PhantomData<(*mut T, &'static mut ())>,
}

// SAFETY: RefMut<T> is morally &mut T, which inherits T's Send and Sync-ness
unsafe impl<T: Send + ?Sized> Send for RefMut<T> {}
unsafe impl<T: Sync + ?Sized> Sync for RefMut<T> {}

impl<T: ?Sized> RefMut<T> {
    /// Consumes this `RefMut` and re-lends its contents (or sub-fields) against
    /// the original scope's rc.
    #[inline]
    pub fn map<R>(self, f: impl for<'a> FnOnce(&'a mut T, Rehypothecator<'a>) -> R) -> R {
        let lien = self._lien;
        f(
            // SAFETY: ptr is valid because the lien keeps the scope alive.
            unsafe { &mut *self.ptr.as_ptr() },
            Rehypothecator {
                lien: &lien,
                _phantom: PhantomData,
            },
        )
    }

    /// Returns a raw pointer to the `RefMut`'s referent.
    ///
    /// The caller must ensure that the `RefMut` outlives the pointer this
    /// function returns, or else it may end up dangling.
    #[inline]
    #[must_use]
    pub fn as_ptr(this: &Self) -> *const T {
        this.ptr.as_ptr()
    }

    /// Returns a raw mutable pointer to the `RefMut`'s referent.
    ///
    /// The caller must ensure that the `RefMut` outlives the pointer this
    /// function returns, or else it may end up dangling.
    #[inline]
    #[must_use]
    pub fn as_mut_ptr(this: &mut Self) -> *mut T {
        this.ptr.as_ptr()
    }
}

impl_deref_traits!(RefMut);

impl<T: ?Sized> DerefMut for RefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see Deref impl. RefMut has exclusive access to *self.ptr.
        unsafe { self.ptr.as_mut() }
    }
}

impl<T: ?Sized> AsMut<T> for RefMut<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: ?Sized> BorrowMut<T> for RefMut<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: Hasher + ?Sized> Hasher for RefMut<T> {
    #[inline]
    fn finish(&self) -> u64 {
        (**self).finish()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        (**self).write(bytes)
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        (**self).write_u8(i)
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        (**self).write_u16(i)
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        (**self).write_u32(i)
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        (**self).write_u64(i)
    }

    #[inline]
    fn write_u128(&mut self, i: u128) {
        (**self).write_u128(i)
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        (**self).write_usize(i)
    }

    #[inline]
    fn write_i8(&mut self, i: i8) {
        (**self).write_i8(i)
    }

    #[inline]
    fn write_i16(&mut self, i: i16) {
        (**self).write_i16(i)
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        (**self).write_i32(i)
    }

    #[inline]
    fn write_i64(&mut self, i: i64) {
        (**self).write_i64(i)
    }

    #[inline]
    fn write_i128(&mut self, i: i128) {
        (**self).write_i128(i)
    }

    #[inline]
    fn write_isize(&mut self, i: isize) {
        (**self).write_isize(i)
    }

    // TODO: nightly fn write_length_prefix (hasher_prefixfree_extras #96762)

    // TODO: nightly fn write_str (hasher_prefixfree_extras #96762)
}

impl<I: Iterator + ?Sized> Iterator for RefMut<I> {
    type Item = I::Item;
    #[inline]
    fn next(&mut self) -> Option<I::Item> {
        (**self).next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }

    // TODO: nightly fn advance_by (iter_advance_by #77404)

    #[inline]
    fn nth(&mut self, n: usize) -> Option<I::Item> {
        (**self).nth(n)
    }

    // TODO: nightly fn try_fold (requires I: Sized + try_trait_v2 #84277)
}

impl<I: DoubleEndedIterator + ?Sized> DoubleEndedIterator for RefMut<I> {
    #[inline]
    fn next_back(&mut self) -> Option<I::Item> {
        (**self).next_back()
    }

    // TODO: nightly fn advance_back_by (iter_advance_by #77404)

    #[inline]
    fn nth_back(&mut self, n: usize) -> Option<I::Item> {
        (**self).nth_back(n)
    }

    // TODO: nightly fn try_rfold (requires I: Sized + try_trait_v2 #84277)
}

impl<I: ExactSizeIterator + ?Sized> ExactSizeIterator for RefMut<I> {
    #[inline]
    fn len(&self) -> usize {
        (**self).len()
    }

    // TODO: nightly fn is_empty (exact_size_is_empty #35428)
}

impl<I: FusedIterator + ?Sized> FusedIterator for RefMut<I> {}

impl<F: Future + Unpin + ?Sized> Future for RefMut<F> {
    type Output = F::Output;
    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        F::poll(Pin::new(&mut **self), cx)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Read + ?Sized> std::io::Read for RefMut<T> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (**self).read(buf)
    }

    #[inline]
    fn read_vectored(&mut self, bufs: &mut [std::io::IoSliceMut<'_>]) -> std::io::Result<usize> {
        (**self).read_vectored(bufs)
    }

    // TODO: nightly fn is_read_vectored (can_vector #69941)

    #[inline]
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> std::io::Result<usize> {
        (**self).read_to_end(buf)
    }

    #[inline]
    fn read_to_string(&mut self, buf: &mut String) -> std::io::Result<usize> {
        (**self).read_to_string(buf)
    }

    #[inline]
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        (**self).read_exact(buf)
    }

    // TODO: nightly fn read_buf (read_buf #78485)

    // TODO: nightly fn read_buf_exact (read_buf #78485)
}

#[cfg(feature = "std")]
impl<T: std::io::Write + ?Sized> std::io::Write for RefMut<T> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (**self).write(buf)
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        (**self).write_vectored(bufs)
    }

    // TODO: nightly fn is_write_vectored (can_vector #69941)

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        (**self).flush()
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        (**self).write_all(buf)
    }

    // TODO: nightly fn write_all_vectored (write_all_vectored #70436)

    #[inline]
    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> std::io::Result<()> {
        (**self).write_fmt(fmt)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Seek + ?Sized> std::io::Seek for RefMut<T> {
    #[inline]
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        (**self).seek(pos)
    }

    #[inline]
    fn rewind(&mut self) -> std::io::Result<()> {
        (**self).rewind()
    }

    // TODO: nightly fn stream_len (seek_stream_len #59359)

    #[inline]
    fn stream_position(&mut self) -> std::io::Result<u64> {
        (**self).stream_position()
    }

    #[inline]
    fn seek_relative(&mut self, offset: i64) -> std::io::Result<()> {
        (**self).seek_relative(offset)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::BufRead + ?Sized> std::io::BufRead for RefMut<T> {
    #[inline]
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        (**self).fill_buf()
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }

    // TODO: nightly fn has_data_left (buf_read_has_data_left #86423)

    #[inline]
    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> std::io::Result<usize> {
        (**self).read_until(byte, buf)
    }

    #[inline]
    fn skip_until(&mut self, byte: u8) -> std::io::Result<usize> {
        (**self).skip_until(byte)
    }

    #[inline]
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        (**self).read_line(buf)
    }
}

/// Macro-internal and unsafe to touch; see [`__Rc`].
#[doc(hidden)]
pub use core::marker::PhantomData as __PhantomData;

/// Creates a [`Scope`].
///
/// ```
/// # use lien::*;
/// let value = 42;
/// let scope = scope!();
/// let r = scope.lend(&value);
/// assert_eq!(*r, 42);
/// ```
///
/// ```
/// # use lien::*;
/// let mut value = 0;
/// let scope = scope!();
/// let mut r = scope.lend_mut(&mut value);
/// *r = 99;
/// assert_eq!(*r, 99);
/// ```
///
/// A lien lasts until the end of its scope (and its `Scope`):
/// ```compile_fail
/// # use lien::*;
/// let mut v = 42;
/// let scope = scope!();
/// let _r = scope.lend_mut(&mut v);
/// v = 0; // 💥 scope still borrows v
/// ```
#[macro_export]
macro_rules! scope {
    () => {
        &$crate::Scope {
            __rc: &$crate::__Rc::new(),
            __phantom: $crate::__PhantomData,
        }
    };
}
