/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! A mock implementation of our instance-binding pattern in pure rust for the atomic variant of GdCell.
//!
//! Used so we can run miri on this, which we cannot when we are running in itest against Godot.

use std::collections::HashMap;
use std::error::Error;
use std::marker::PhantomData;
use std::panic;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use godot_cell::atomic::{AtomicGdCell as GdCell, InaccessibleGuard, MutGuard, RefGuard};

super::setup_mock!(GdCell);

// ----------------------------------------------------------------------------------------------------------------------------------------------

/// `instance_id` must be the key of a `MyClass`.
unsafe fn assert_id_is(instance_id: usize, target: i64) {
    let storage = unsafe { get_instance::<MyClass>(instance_id) };
    let bind = storage.cell.borrow().unwrap();
    assert_eq!(bind.int, target);
}

unsafe fn assert_poisoned(instance_id: usize) {
    let storage = unsafe { get_instance::<MyClass>(instance_id) };
    assert!(storage.cell.is_poisoned())
}

super::setup_test_class!();

// ----------------------------------------------------------------------------------------------------------------------------------------------

// NOTE: We have to ignore each test individually, instead of using #[cfg(test)] on a module containing them, so that
// `cargo test` still lists those tests, while indicating the reason why they were ignored.

/// Run each method both from the main thread and a newly created thread.
#[test]
fn calls_different_thread() {
    use std::thread;

    let instance_id = MyClass::init();

    // We're not running in parallel, so it will never fail to increment completely.
    for (f, _, expected_increment) in CALLS {
        let start = unsafe { get_int(instance_id) };
        unsafe {
            f(instance_id).unwrap();

            assert_id_is(instance_id, start + expected_increment);
        }
        let start = start + expected_increment;
        thread::spawn(move || unsafe { f(instance_id).unwrap() })
            .join()
            .unwrap();
        unsafe {
            assert_id_is(instance_id, start + expected_increment);
        }
    }
}

///
#[test]
fn calls_sequential() {
    use std::sync::Arc;
    use std::thread;

    let instance_id = MyClass::init();
    let immut_cond = Arc::new((
        Mutex::new((None, true, false, false)),
        Condvar::new(),
        Condvar::new(),
    ));

    let cond1 = immut_cond.clone();

    let t1 = thread::spawn(move || unsafe {
        with_obj(instance_id, move |obj: &mut MyClass| {
            let (lock, cvar_producer, cvar_consumer) = &*cond1;
            for i in 1..=10 {
                let mut state = lock.lock().unwrap();
                while !state.1 {
                    state = cvar_producer.wait(state).unwrap();
                }
                state.0 = Some(i);
                state.1 = false;
                let _guard = obj.base();
                cvar_consumer.notify_one();
                while !state.2 {
                    state = cvar_producer.wait(state).unwrap();
                }
                state.2 = false;
            }

            // Notify other thread that the test is over.
            let mut state = lock.lock().unwrap();
            state.3 = true;
            state.1 = false;
            drop(state);
            cvar_consumer.notify_one();
        })
        .unwrap()
    });

    let cond2 = immut_cond.clone();
    let t2 = thread::spawn(move || unsafe {
        let (lock, cvar_producer, cvar_consumer) = &*cond2;

        loop {
            let mut state = lock.lock().unwrap();

            while state.1 {
                state = cvar_consumer.wait(state).unwrap();
            }

            if state.3 {
                break;
            }

            let v = state.0.take().unwrap();
            state.1 = true;
            with_obj(instance_id, move |obj: &mut MyClass| {
                obj.int += v;
            })
            .unwrap();
            state.2 = true;
            cvar_producer.notify_one();
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    unsafe { assert_id_is(instance_id, (1..=10).into_iter().sum()) };
}

#[test]
fn calls_sequential_panic() {
    use std::sync::Arc;
    use std::thread;

    let instance_id = MyClass::init();
    let immut_cond = Arc::new((
        Mutex::new((None, true, false, false)),
        Condvar::new(),
        Condvar::new(),
    ));

    let cond1 = immut_cond.clone();

    let t1 = thread::spawn(move || unsafe {
        let err = std::panic::catch_unwind(move || {
            let err = with_obj(instance_id, move |_obj: &mut MyClass| {
                let (lock, cvar_producer, cvar_consumer) = &*cond1;
                let mut state = lock.lock().unwrap();
                state.0 = Some(1);
                state.1 = false;
                cvar_consumer.notify_one();
                while !state.1 {
                    state = cvar_producer.wait(state).unwrap();
                }
            });
            println!("my err! {:?}", err);

            err.is_err()
        });

        println!("my err 2! {:?}", err);
        err
    });

    let cond2 = immut_cond.clone();
    let t2 = thread::spawn(move || unsafe {
        let (lock, cvar_producer, cvar_consumer) = &*cond2;
        let mut state = lock.lock().unwrap();

        while state.0.is_none() {
            state = cvar_consumer.wait(state).unwrap();
        }
        let v = state.0.take().unwrap();

        let err = std::panic::catch_unwind(move || {
            with_obj(instance_id, move |obj: &mut MyClass| {
                obj.int += v;
            })
            .is_err()
        });
        state.1 = true;
        cvar_producer.notify_one();
        err
    });

    println!("{:?}", t1.join().unwrap());
    println!("{:?}", t2.join().unwrap());

    // assert!(t1.join().unwrap().is_err());
    // assert!(t2.join().unwrap().is_err());

    unsafe {
        assert_poisoned(instance_id);
    }
}

// /// Call each method from different threads, allowing them to run in parallel.
// ///
// /// This may cause borrow failures, we do a best-effort attempt at estimating the value then. We can detect
// /// if the first call failed, so then we know the integer was incremented by 0. Otherwise, we at least know
// /// the range of values that it can be incremented by.
// #[test]
// fn calls_parallel() {
//     use std::thread;

//     let instance_id = MyClass::init();
//     let mut handles = Vec::new();

//     for (f, min_increment, max_increment) in CALLS {
//         let handle = thread::spawn(move || {
//             std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || unsafe {
//                 f(instance_id).map_or((0, 0), |_| (*min_increment, *max_increment))
//             }))
//         });
//         handles.push(handle);
//     }

//     let (min_expected, max_expected) = handles
//         .into_iter()
//         .filter_map(|handle| handle.join().unwrap().ok())
//         .reduce(|(curr_min, curr_max), (min, max)| (curr_min + min, curr_max + max))
//         .unwrap();

//     unsafe {
//         assert!(get_int(instance_id) >= min_expected);
//         assert!(get_int(instance_id) <= max_expected);
//     }
// }

// /// Call each method from different threads, allowing them to run in parallel.
// ///
// /// This may cause borrow failures, we do a best-effort attempt at estimating the value then. We can detect
// /// if the first call failed, so then we know the integer was incremented by 0. Otherwise, we at least know
// /// the range of values that it can be incremented by.
// ///
// /// Runs each method several times in a row. This should reduce the non-determinism that comes from
// /// scheduling of threads.
// #[test]
// fn calls_parallel_many_serial() {
//     use std::thread;

//     let instance_id = MyClass::init();
//     let mut handles = Vec::new();

//     for (f, min_increment, max_increment) in CALLS {
//         for _ in 0..10 {
//             let handle = thread::spawn(move || unsafe {
//                 f(instance_id).map_or((0, 0), |_| (*min_increment, *max_increment))
//             });
//             handles.push(handle);
//         }
//     }

//     let (min_expected, max_expected) = handles
//         .into_iter()
//         .map(|handle| handle.join().unwrap())
//         .reduce(|(curr_min, curr_max), (min, max)| (curr_min + min, curr_max + max))
//         .unwrap();

//     unsafe {
//         assert!(get_int(instance_id) >= min_expected);
//         assert!(get_int(instance_id) <= max_expected);
//     }
// }

// /// Call each method from different threads, allowing them to run in parallel.
// ///
// /// This may cause borrow failures, we do a best-effort attempt at estimating the value then. We can detect
// /// if the first call failed, so then we know the integer was incremented by 0. Otherwise, we at least know
// /// the range of values that it can be incremented by.
// ///
// /// Runs all the tests several times. This is different from [`calls_parallel_many_serial`] as that calls the
// /// methods like AAA...BBB...CCC..., whereas this interleaves the methods like ABC...ABC...ABC...
// #[test]
// fn calls_parallel_many_parallel() {
//     use std::thread;

//     let instance_id = MyClass::init();
//     let mut handles = Vec::new();

//     for _ in 0..10 {
//         for (f, min_increment, max_increment) in CALLS {
//             let handle = thread::spawn(move || unsafe {
//                 f(instance_id).map_or((0, 0), |_| (*min_increment, *max_increment))
//             });
//             handles.push(handle);
//         }
//     }

//     let (min_expected, max_expected) = handles
//         .into_iter()
//         .map(|handle| handle.join().unwrap())
//         .reduce(|(curr_min, curr_max), (min, max)| (curr_min + min, curr_max + max))
//         .unwrap();

//     unsafe {
//         assert!(get_int(instance_id) >= min_expected);
//         assert!(get_int(instance_id) <= max_expected);
//     }
// }
