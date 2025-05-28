/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::builtin::{Callable, Variant};
use crate::classes::object::ConnectFlags;
use crate::meta;
use crate::obj::{bounds, Bounds, Gd, GodotClass, WithBaseField, WithSignals, WithUserSignals};
use crate::registry::signal::{
    make_callable_name, make_godot_fn, ConnectBuilder, SignalObject, SignalReceiver,
};
use std::borrow::Cow;
use std::marker::PhantomData;
use crate::registry::signal::variadic::{DeFuck};

/// Object part of the signal receiver (handler).
///
/// Functionality overlaps partly with [`meta::AsObjectArg`] and [`meta::AsArg<ObjectArg>`]. Can however not directly be replaced
/// with `AsObjectArg`, since that allows nullability and doesn't require `&mut T`. Maybe there's a way to reuse them though.
pub trait ToSignalObj<C: GodotClass> {
    fn to_signal_obj(&self) -> Gd<C>;
}

impl<C: GodotClass> ToSignalObj<C> for Gd<C> {
    fn to_signal_obj(&self) -> Gd<C> {
        self.clone()
    }
}

impl<C: WithBaseField> ToSignalObj<C> for C {
    fn to_signal_obj(&self) -> Gd<C> {
        WithBaseField::to_gd(self)
    }
}

// ----------------------------------------------------------------------------------------------------------------------------------------------

/// Type-safe version of a Godot signal.
///
/// Short-lived type, only valid in the scope of its surrounding object type `C`, for lifetime `'c`. The generic argument `Ps` represents
/// the parameters of the signal, thus ensuring the type safety.
///
/// The [`WithSignals::SignalCollection`] struct returns multiple signals with distinct, code-generated types, but they all implement
/// `Deref` and `DerefMut` to `TypedSignal`. This allows you to either use the concrete APIs of the generated types, or the more generic
/// ones of `TypedSignal`.
///
/// # Connecting a signal to a receiver
/// Receiver functions are functions that are called when a signal is emitted. You can connect a signal in many different ways:
/// - [`connect()`][Self::connect] for global functions, associated functions or closures.
/// - [`connect_self()`][Self::connect_self] for methods with `&mut self` as the first parameter.
/// - [`connect_obj()`][Self::connect_obj] for methods with any `Gd<T>` (not `self`) as the first parameter.
/// - [`connect_builder()`][Self::connect_builder] for more complex setups.
///
/// # Emitting a signal
/// Code-generated signal types provide a method `emit(...)`, which adopts the names and types of the `#[signal]` parameter list.
/// In most cases, that's the method you are looking for.
///
/// For generic use, you can also use [`emit_tuple()`][Self::emit_tuple], which does not provide parameter names.
///
/// # More information
/// See the [Signals](https://godot-rust.github.io/book/register/signals.html) chapter in the book for a detailed introduction and examples.
pub struct TypedSignal<'c, C: WithSignals, Ps> {
    /// In Godot, valid signals (unlike funcs) are _always_ declared in a class and become part of each instance. So there's always an object.
    object: C::__SignalObj<'c>,
    name: Cow<'static, str>,
    _signature: PhantomData<Ps>,
}

impl<'c, C: WithSignals, Ps: meta::ParamTuple> TypedSignal<'c, C, Ps> {
    #[doc(hidden)]
    pub fn extract(
        obj: &mut Option<C::__SignalObj<'c>>,
        signal_name: &'static str,
    ) -> TypedSignal<'c, C, Ps> {
        let obj = obj.take().unwrap_or_else(|| {
            panic!(
                "signals().{signal_name}() call failed; signals() allows only one signal configuration at a time \n\
                see https://godot-rust.github.io/book/register/signals.html#one-signal-at-a-time"
            )
        });

        Self::new(obj, signal_name)
    }

    // Currently only invoked from godot-core classes, or from UserSignalObject::into_typed_signal.
    // When making public, make also #[doc(hidden)].
    fn new(object: C::__SignalObj<'c>, name: &'static str) -> Self {
        Self {
            object,
            name: Cow::Borrowed(name),
            _signature: PhantomData,
        }
    }

    pub(crate) fn receiver_object(&self) -> Gd<C> {
        let object = self.object.to_owned_object();

        // Potential optimization: downcast could use a new private Gd::unchecked_cast().
        // try_cast().unwrap_unchecked() won't be that efficient due to internal code path.
        object.cast()
    }

    /// Emit the signal with the given parameters.
    ///
    /// This is intended for generic use. Typically, you'll want to use the more specific `emit()` method of the code-generated signal
    /// type, which also has named parameters.
    pub fn emit_tuple(&mut self, args: Ps)
    where
        Ps: meta::OutParamTuple,
    {
        let name = self.name.as_ref();

        self.object.with_object_mut(|obj| {
            obj.emit_signal(name, &args.to_variant_array());
        });
    }

    /// Connect a non-member function (global function, associated function or closure).
    ///
    /// Example usages:
    /// ```ignore
    /// sig.connect(Self::static_func);
    /// sig.connect(global_func);
    /// sig.connect(|arg| { /* closure */ });
    /// ```
    ///
    /// To connect to a method of the own object `self`, use [`connect_self()`][Self::connect_self].  \
    /// If you need cross-thread signals or connect flags, use [`connect_builder()`][Self::connect_builder].
    pub fn connect<F>(&mut self, mut function: F)
    where
        F: SignalReceiver<(), Ps>,
        Ps: meta::InParamTuple + 'static,
    {
        let godot_fn = make_godot_fn(move |args| {
            function.call((), args);
        });

        self.inner_connect_godot_fn::<F>(godot_fn);
    }

    /// Connect a method (member function) with any `Gd<T>` (not `self`) as the first parameter.
    ///
    /// To connect to methods on the same object that declares the `#[signal]`, use [`connect_self()`][Self::connect_self].  \
    /// If you need cross-thread signals or connect flags, use [`connect_builder()`][Self::connect_builder].
    pub fn connect_obj<F, OtherC>(&mut self, object: &impl ToSignalObj<OtherC>, mut method: F)
    where
        OtherC: GodotClass + Bounds<Declarer = bounds::DeclUser>,
        for<'c_rcv> F: SignalReceiver<&'c_rcv mut OtherC, Ps>,
        Ps: meta::InParamTuple + 'static,
    {
        let mut gd = object.to_signal_obj();
        // let mut gd = gd.to_owned_object();
        let godot_fn = make_godot_fn(move |args| {
            let mut instance = gd.bind_mut();
            let instance = &mut *instance;
            method.call(instance, args);
        });

        self.inner_connect_godot_fn::<F>(godot_fn);
    }

    /// Fully customizable connection setup.
    ///
    /// The returned builder provides several methods to configure how to connect the signal. It needs to be finalized with a call to
    /// [`ConnectBuilder::done()`].
    pub fn connect_builder(&mut self) -> ConnectBuilder<'_, 'c, C, (), Ps, ()> {
        ConnectBuilder::new(self)
    }

    /// Directly connect a Rust callable `godot_fn`, with a name based on `F`.
    ///
    /// This exists as a short-hand for the connect methods on [`TypedSignal`] and avoids the generic instantiation of the full-blown
    /// type state builder for simple + common connections, thus hopefully being a tiny bit lighter on compile times.
    fn inner_connect_godot_fn<F>(
        &mut self,
        godot_fn: impl FnMut(&[&Variant]) -> Result<Variant, ()> + 'static,
    ) {
        let callable_name = make_callable_name::<F>();
        let callable = Callable::from_local_fn(&callable_name, godot_fn);

        let signal_name = self.name.as_ref();
        self.object.with_object_mut(|obj| {
            obj.connect(signal_name, &callable);
        });
    }

    /// Connect an untyped callable, with optional flags.
    ///
    /// Used by [`ConnectBuilder::done()`]. Any other type-state (such as thread-local/sync, callable debug name, etc.) are baked into
    /// `callable` and thus type-erased into runtime logic.
    pub(super) fn inner_connect_untyped(
        &mut self,
        callable: &Callable,
        flags: Option<ConnectFlags>,
    ) {
        use crate::obj::EngineBitfield;

        let signal_name = self.name.as_ref();

        self.object.with_object_mut(|obj| {
            let mut c = obj.connect_ex(signal_name, callable);
            if let Some(flags) = flags {
                c = c.flags(flags.ord() as u32);
            }
            c.done();
        });
    }

    pub(crate) fn to_untyped(&self) -> crate::builtin::Signal {
        crate::builtin::Signal::from_object_signal(&self.receiver_object(), &*self.name)
    }
}

impl<C: WithUserSignals, Ps: meta::InParamTuple + 'static> TypedSignal<'_, C, Ps> {
    /// Connect a method (member function) with `&mut self` as the first parameter.
    ///
    /// To connect to methods on other objects, use [`connect_obj()`][Self::connect_obj].  \
    /// If you need a `&self` receiver, cross-thread signals or connect flags, use [`connect_builder()`][Self::connect_builder].
    pub fn connect_self<F>(&mut self, mut function: F)
    where
        for<'c_rcv> F: SignalReceiver<&'c_rcv mut C, Ps>,
        for<'c_rcv> DeFuck<'c_rcv, &'c_rcv mut C, Ps, F>: From<&'c_rcv mut F>
    {
        let mut gd = self.receiver_object();
        let godot_fn = make_godot_fn(move |args| {
            let mut instance = gd.bind_mut();
            let instance = &mut *instance;
            <&mut F as Into<DeFuck<&'_ mut C, Ps, F>>>::into(&mut function).inner.call(instance, args);
        });

        self.inner_connect_godot_fn::<F>(godot_fn);
    }
}
