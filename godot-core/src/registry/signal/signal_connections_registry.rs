/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Signal connection registry
//!
//! Interacting with custom callables (used by typed signals) after hot reload in any way
//! is an instant UB. We prevent unsoundness by disconnecting all the signals before the hot reload.
//!
//! To achieve this, we store all connections in a global registry as long as the library remains loaded and given receiver object is alive.
//!
//! This feature is being compiled out with safeguards disengaged (storing, validating and disconnecting signals present non-trivial
//! amount of overhead).
//!
//! For upstream issue, see: <https://github.com/godotengine/godot/issues/105802>.

pub(crate) use __signal_connections_registry::{
    prune_stored_signal_connections, store_signal_connection,
};

#[cfg(not(safeguards_balanced))]
mod __signal_connections_registry {
    use crate::builtin::{Callable, CowStr};
    use crate::classes::Object;
    use crate::obj::Gd;

    #[allow(unused)]
    pub fn store_signal_connection(
        _receiver_object: &Gd<Object>,
        _signal_name: &CowStr,
        _callable: &Callable,
    ) {
    }

    #[allow(unused)]
    pub fn prune_stored_signal_connections() {}
}
#[cfg(safeguards_balanced)]
mod __signal_connections_registry {
    use crate::builtin::{Callable, CowStr};
    use crate::classes::{Engine, Object};
    use crate::godot_warn;
    use crate::obj::{Gd, Singleton};

    static IS_EDITOR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static __GODOT_SIGNAL_CONNECTIONS_REGISTRY: std::sync::Mutex<Vec<UnsafeConnectHandle>> =
        std::sync::Mutex::new(Vec::new());

    // Not a ConnectHandle because of slight differences + we can't reuse its methods anyway.
    struct UnsafeConnectHandle {
        // `Option<...>` so we can use `retain_mut` to prune old objects.
        receiver_object: Option<Gd<Object>>,
        signal_name: CowStr,
        callable: Callable,
    }

    // SAFETY: UnsafeConnectHandle is accessed only once, during library initialization, which must take place on main thread.
    unsafe impl Send for UnsafeConnectHandle {}
    unsafe impl Sync for UnsafeConnectHandle {}

    /// Prunes stale connections to objects that are no longer valid.
    ///
    /// This method does not check the validity of the signals themselves due to the overhead required.
    /// Additionally number of connections to alive objects is finite - unlike connections to freed objects,
    /// which can accumulate to a critical mass simply by opening and closing tabs in the editor.
    fn prune_stale_connections(registry: &mut Vec<UnsafeConnectHandle>) {
        registry.retain_mut(|connection| {
            if connection.receiver_object.is_none() {
                return false;
            }

            if connection
                .receiver_object
                .as_ref()
                .is_some_and(|obj| !obj.is_instance_valid())
            {
                let obj = connection.receiver_object.take().unwrap();
                obj.drop_weak();
                return false;
            }
            true
        });
    }

    /// Stores the given connection in a registry so it can be disconnected during library deinitialization,
    /// and prunes any existing connections to objects that are no longer valid.
    pub fn store_signal_connection(
        receiver_object: &Gd<Object>,
        signal_name: &CowStr,
        callable: &Callable,
    ) {
        if !IS_EDITOR.get_or_init(|| Engine::singleton().is_editor_hint()) {
            return;
        }

        let mut connection_registry = __GODOT_SIGNAL_CONNECTIONS_REGISTRY.lock().unwrap();
        prune_stale_connections(&mut connection_registry);

        // SAFETY: Given weak pointer to the Object is accessed only once in `prune_stored_signal_connections` or `prune_stale_connections`,
        // inaccessible outside this module, validated before use and properly disposed of by using `drop_weak`.
        let weak_object_ptr = unsafe { receiver_object.clone_weak() };
        connection_registry.push(UnsafeConnectHandle {
            receiver_object: Some(weak_object_ptr),
            signal_name: signal_name.clone(),
            callable: callable.clone(),
        });
    }

    /// Disconnects all the registered signals.
    ///
    /// Should be run only once during initialization of the library on [`InitLevel::Editor`].
    /// Running it multiple times is totally safe though; it is just silly.
    pub fn prune_stored_signal_connections() {
        let mut connection_registry = __GODOT_SIGNAL_CONNECTIONS_REGISTRY.lock().unwrap();

        if connection_registry.is_empty() {
            return;
        }

        // TODO! - shorten this message and point to newly-written chapter in the book.
        godot_warn!(
            "godot-rust: Automatically disconnecting all registered typed signal connections. \n
Custom callables used by godot-rust signals become invalid after a hot reload.\
All the registered connections will be automatically disconnected to prevent unsoundness.\
After hot reload connections must be recreated with `ObjectNotification::EXTENSION_RELOADED`.\
You may also consider using untyped signals in this scenario.\
For more information, see: https://godot-rust.github.io/book/register/signals.html#untyped-signals."
        );

        for UnsafeConnectHandle {
            receiver_object,
            signal_name,
            callable,
        } in connection_registry.drain(..)
        {
            let Some(mut receiver_object) = receiver_object else {
                continue;
            };

            if !receiver_object.is_instance_valid() {
                receiver_object.drop_weak();
                continue;
            }

            if receiver_object.is_connected(&*signal_name, &callable) {
                receiver_object.disconnect(&*signal_name, &callable);
            }

            receiver_object.drop_weak();
        }
    }
}
