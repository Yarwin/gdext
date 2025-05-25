/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Emulates variadic argument lists (via tuples), related to functions and signals.

// https://geo-ant.github.io/blog/2021/rust-traits-and-variadic-functions
//
// Could be generalized with R return type, and not special-casing `self`. But keep simple until actually needed.

use std::marker::PhantomData;

/// Trait that is implemented for functions that can be connected to signals.
///
// Direct RustDoc link doesn't work, for whatever reason again...
/// This is used in [`ConnectBuilder`](struct.ConnectBuilder.html). There are three variations of the `I` (instance) parameter:
/// - `()` for global and associated ("static") functions.
/// - `&C` for `&self` methods.
/// - `&mut C` for `&mut self` methods.
///
/// See also [Signals](https://godot-rust.github.io/book/register/signals.html) in the book.
pub trait SignalReceiver<I, Ps>: 'static {
    /// Invoke the receiver on the given instance (possibly `()`) with `params`.
    fn call(&mut self, maybe_instance: I, params: Ps);
}

pub struct DeFuck<'c_rcv, I, Ps, S>
where
    S: SignalReceiver<I, Ps>
{
    pub inner: &'c_rcv mut S,
    identity: PhantomData<I>,
    params: PhantomData<Ps>,
}


// ----------------------------------------------------------------------------------------------------------------------------------------------
// Generated impls

macro_rules! impl_signal_recipient {
    ($( $args:ident : $Ps:ident ),*) => {
        // --------------------------------------------------------------------------------------------------------------------------------------
        // SignalReceiver


        impl<'c_rcv, F, R, C, $($Ps,)*> From<&'c_rcv mut F> for DeFuck<'c_rcv, &mut C, ($($Ps,)*), F>
            where F: FnMut( &mut C, $($Ps,)* ) -> R + 'static
        {
            fn from(value: &'c_rcv mut F) -> Self {
                DeFuck {
                    inner: value,
                    identity: PhantomData,
                    params: PhantomData
                }
            }
        }


        // impl<F, R, $($Ps,)*> CallHack<(), ( $($Ps,)* )> for F
        // where
        //     F: FnMut( $($Ps,)* ) -> R + 'static {
        //
        // }
        //
        // impl<F, R, C, $($Ps,)*> CallHack<&mut C, ( $($Ps,)* )> for F
        // where
        //     F: FnMut(&mut C, $($Ps,)* ) -> R + 'static {
        //
        // }
        //
        // impl<F, R, C, $($Ps,)*> CallHack<&C, ( $($Ps,)* )> for F
        // where
        //     F: FnMut(&C, $($Ps,)* ) -> R + 'static {
        //
        // }

        // impl<Closure, R> CallHack<(), ( $($Ps,)* ), Closure> for Closure
        // where
        //     Closure: FnMut( $($Ps,)* ) -> R + 'static + SignalReceiver<(), ( $($Ps,)* )>
        // {}

        // Global and associated functions.
        impl<F, R, $($Ps,)*> SignalReceiver<(), ( $($Ps,)* )> for F
            where F: FnMut( $($Ps,)* ) -> R + 'static
        {
            fn call(&mut self, _no_instance: (), ($($args,)*): ( $($Ps,)* )) {
                self($($args,)*);
            }
        }

        // Methods with mutable receiver - &mut self.
        impl<F, R, C, $($Ps,)*> SignalReceiver<&mut C, ( $($Ps,)* )> for F
            where F: FnMut( &mut C, $($Ps,)* ) -> R + 'static
        {
            fn call(&mut self, instance: &mut C, ($($args,)*): ( $($Ps,)* )) {
                self(instance, $($args,)*);
            }
        }

        // Methods with immutable receiver - &self.
        impl<F, R, C, $($Ps,)*> SignalReceiver<&C, ( $($Ps,)* )> for F
            where F: FnMut( &C, $($Ps,)* ) -> R + 'static
        {
            fn call(&mut self, instance: &C, ($($args,)*): ( $($Ps,)* )) {
                self(instance, $($args,)*);
            }
        }
    };
}

impl_signal_recipient!();
impl_signal_recipient!(arg0: P0);
impl_signal_recipient!(arg0: P0, arg1: P1);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4, arg5: P5);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4, arg5: P5, arg6: P6);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4, arg5: P5, arg6: P6, arg7: P7);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4, arg5: P5, arg6: P6, arg7: P7, arg8: P8);
impl_signal_recipient!(arg0: P0, arg1: P1, arg2: P2, arg3: P3, arg4: P4, arg5: P5, arg6: P6, arg7: P7, arg8: P8, arg9: P9);
