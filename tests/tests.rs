use lien::*;

#[test]
fn scope_lend_read() {
    let value = 42;
    let s = scope!();
    let r = s.lend(&value);
    assert_eq!(*r, 42);
}

#[test]
fn scope_lend_mut_write() {
    let mut value = 0;
    let s = scope!();
    let mut r = s.lend_mut(&mut value);
    *r = 99;
    assert_eq!(*r, 99);
}

#[test]
fn ref_clone() {
    let value = 42;
    let s = scope!();
    let r = s.lend(&value);
    let r2 = r.clone();
    assert_eq!(*r, *r2);
}

#[test]
fn ref_debug_display() {
    let value = "hello";
    let s = scope!();
    let r = s.lend(&value);
    assert_eq!(format!("{:?}", r), format!("{:?}", "hello"));
    assert_eq!(format!("{}", r), "hello");
}

#[test]
fn refmut_into_ref() {
    let mut value = 42;
    let s = scope!();
    let r: Ref<i32> = s.lend_mut(&mut value).into();
    assert_eq!(*r, 42);
}

#[test]
fn scope_wait() {
    let value = 42;
    let s = scope!();
    let r = s.lend(&value);
    drop(r);
    // s drops here, waits for all liens
}

#[test]
fn scope_wait_threaded() {
    let value = 42;
    let s = scope!();
    let r = s.lend(&value);
    std::thread::spawn(move || {
        assert_eq!(*r, 42);
    });
    // s drops here, waits for the spawned thread to drop r
}

#[test]
fn ref_map_keeps_original() {
    let value = (1, 2);
    let s = scope!();
    let r = s.lend(&value);
    r.map(|v, rehyp| {
        let _r2 = rehyp.lend(&v.0);
        assert_eq!(*_r2, 1);
    });
    // r is still alive
    assert_eq!(*r, (1, 2));
}

#[test]
fn refmut_map_split() {
    let mut value = (1u32, 2u32);
    let s = scope!();
    let r = s.lend_mut(&mut value);
    r.map(|v, rehyp| {
        let r1 = rehyp.lend(&v.0);
        let r2 = rehyp.lend(&v.1);
        assert_eq!(*r1, 1);
        assert_eq!(*r2, 2);
    });
}

#[test]
fn refmut_map_mut_split() {
    let mut value = (1u32, 2u32);
    let s = scope!();
    let r = s.lend_mut(&mut value);
    r.map(|v, rehyp| {
        let mut r1 = rehyp.lend_mut(&mut v.0);
        let mut r2 = rehyp.lend_mut(&mut v.1);
        *r1 = 10;
        *r2 = 20;
        assert_eq!(*r1, 10);
        assert_eq!(*r2, 20);
    });
}

#[test]
fn ref_map_threaded() {
    let value = (1, 2);
    let s = scope!();
    let r = s.lend(&value);
    r.map(|v, rehyp| {
        let r1 = rehyp.lend(&v.0);
        let r2 = rehyp.lend(&v.1);
        let h1 = std::thread::spawn(move || {
            assert_eq!(*r1, 1);
        });
        let h2 = std::thread::spawn(move || {
            assert_eq!(*r2, 2);
        });
        h1.join().unwrap();
        h2.join().unwrap();
    });
}

#[test]
fn lien_clone_and_drop() {
    let s = scope!();
    let l1 = s.lien();
    let l2 = l1.clone();
    let l3 = l2.clone();
    drop(l1);
    drop(l2);
    drop(l3);
    // s drops here, returns immediately — all liens already gone
}

#[test]
fn ref_hash_eq_ord() {
    use std::collections::HashSet;
    let a = 1;
    let b = 1;
    let s = scope!();
    let r1 = s.lend(&a);
    let r2 = s.lend(&b);
    assert_eq!(r1, r2);
    assert!(r1 <= r2);
    let mut set = HashSet::new();
    set.insert(r1.clone());
    assert!(set.contains(&r2));
}

#[test]
fn scope_multiple_lends() {
    let a = 1;
    let b = 2;
    let c = 3;
    let s = scope!();
    let r1 = s.lend(&a);
    let r2 = s.lend(&b);
    let r3 = s.lend(&c);
    assert_eq!(*r1 + *r2 + *r3, 6);
}

#[test]
fn scope_blocks_until_lien_dropped_by_thread() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let flag = Arc::new(AtomicBool::new(false));
    let flag2 = flag.clone();

    let mut value = 42;
    {
        let s = scope!();
        let r = s.lend_mut(&mut value);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(*r, 42);
            flag2.store(true, Ordering::Release);
            drop(r);
        });
        // s drops here — must block until the thread drops r
    }
    // If we get here, the scope waited for the thread.
    assert!(
        flag.load(Ordering::Acquire),
        "scope dropped before lien was released"
    );
    // value is usable again after scope exited
    value = 0;
    assert_eq!(value, 0);
}

#[test]
fn scope_survives_panicking_thread() {
    let value = 42;
    let s = scope!();
    let r = s.lend(&value);
    let h = std::thread::spawn(move || {
        let _keep = r;
        panic!("boom");
    });
    let _ = h.join();
    // s drops here — lien was dropped during unwinding
}
