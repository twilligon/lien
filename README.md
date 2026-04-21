`lien`
======

[![crates.io](https://img.shields.io/crates/v/lien.svg)](https://crates.io/crates/lien) [![docs.rs](https://docs.rs/lien/badge.svg)](https://docs.rs/lien/)

> **🚧 Under construction 🚧**
> This crate is somewhat experimental: the recommended API may change (in semver-compatible ways) in future versions.

Scoped lending of borrowed references as `Send`-able smart pointers. Like `thread::scope` without the thread!

A *[lien](https://en.wiktionary.org/wiki/lien#English:_legal_claim)* is "a right to take possession of a debtor's property as security until a debt or duty is discharged." Similarly, a [`Lien`] represents a borrow of something from a [`Scope`]. Like a static borrow with `&` or `&mut`, this forces the `Scope` to outlive any `Lien`s made from it, but like an `Arc`, `Lien` carries no lifetime (it's atomically reference-counted at runtime), is thread-safe, and can be freely cloned.

```rust
let mut greeting = String::from("hello ");

{
    let scope = lien::scope!();
    let mut g = scope.lend_mut(&mut greeting);

    thread::spawn(move || {
        g.push_str("beautiful ");
    });
}

greeting.push_str("world");
assert_eq!(greeting, "hello beautiful world");
```

## Smart pointers

- [`Lien`]: a bare scope token.
- [`Ref`]: a sendable shared reference (like `&T`).
- [`RefMut`]: a sendable exclusive reference (like `&mut T`).

Both `Ref` and `RefMut` support sub-borrowing through [`Ref::map`] / [`RefMut::map`], which lets you re-lend fields against the original scope.

[`Lien`]: https://docs.rs/lien/latest/lien/struct.Lien.html
[`Scope`]: https://docs.rs/lien/latest/lien/struct.Scope.html
[`Ref`]: https://docs.rs/lien/latest/lien/struct.Ref.html
[`RefMut`]: https://docs.rs/lien/latest/lien/struct.RefMut.html
[`Ref::map`]: https://docs.rs/lien/latest/lien/struct.Ref.html#method.map
[`RefMut::map`]: https://docs.rs/lien/latest/lien/struct.RefMut.html#method.map

## `#[no_std]`

Disable the `std` feature:

```toml
[dependencies]
lien = { version = "0.1", default-features = false }
```
