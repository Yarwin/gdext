/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::borrow_state::BorrowStateErr;

const EXCLUSIVE_REF_MASK: usize = !(usize::MAX >> 1);

const SHARED_REF_MASK: usize = usize::MAX >> 1;

struct RefState {
    has_exclusive: bool,
    shared_count: usize,
}

impl RefState {
    fn from_usize(refstate: usize) -> Self {
        Self {
            has_exclusive: refstate & EXCLUSIVE_REF_MASK != 0,
            shared_count: refstate & SHARED_REF_MASK,
        }
    }

    fn from_load(refstate: &AtomicUsize) -> Self {
        Self::from_usize(refstate.fetch_add(1, Ordering::Acquire))
    }

    fn from_increment(refstate: &AtomicUsize) -> Self {
        Self::from_usize(refstate.fetch_add(1, Ordering::AcqRel))
    }

    fn from_decrement(refstate: &AtomicUsize) -> Self {
        Self::from_usize(refstate.fetch_sub(1, Ordering::AcqRel))
    }

    fn ensure_can_ref(&self) -> Result<(), BorrowStateErr> {
        if self.has_exclusive {
            return Err("cannot borrow while accessible mutable borrow exists".into());
        }

        if self.shared_count == SHARED_REF_MASK {
            return Err("attempted to borrow with overflow".into());
        }

        Ok(())
    }

    fn ensure_can_mut_ref(&self) -> Result<(), BorrowStateErr> {
        self.ensure_can_ref()?;
        if self.shared_count != 0 {
            return Err("cannot borrow mutable while shared borrow exists".into());
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct AtomicBorrowState {
    /// The first `n-1` bits store count of `&T` references,
    /// while the very last bit informs if any exclusive `&mut T` reference exists.
    refstate: AtomicUsize,
    /// The number of `&mut T` references that are inaccessible.
    inaccessible_count: AtomicUsize,
    /// `true` if the borrow state has reached an erroneous or unreliable state.
    poisoned: AtomicBool,
}

impl AtomicBorrowState {
    pub fn new() -> Self {
        Self {
            refstate: AtomicUsize::new(0),
            inaccessible_count: AtomicUsize::new(0),
            poisoned: AtomicBool::new(false),
        }
    }

    pub(crate) fn may_unset_inaccessible(&self) -> bool {
        let refstate = RefState::from_usize(self.refstate.load(Ordering::Acquire));
        let innacessibe_count = self.inaccessible_count.load(Ordering::Acquire);
        !refstate.has_exclusive && refstate.shared_count == 0 && innacessibe_count > 0
    }

    /// Returns `true` if the state has reached an erroneous or unreliable state.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }

    /// Set self as having reached an erroneous or unreliable state.
    ///
    /// Always returns [`BorrowStateErr::Poisoned`].
    pub(crate) fn poison(&self, err: impl Into<String>) -> Result<(), BorrowStateErr> {
        self.poisoned.store(true, Ordering::Release);

        Err(BorrowStateErr::Poisoned(err.into()))
    }

    pub(crate) fn is_currently_bound(&self) -> bool {
        self.refstate.load(Ordering::Acquire) == 0
            && self.inaccessible_count.load(Ordering::Acquire) == 0
    }

    fn ensure_not_poisoned(&self) -> Result<(), BorrowStateErr> {
        if self.is_poisoned() {
            return Err(BorrowStateErr::IsPoisoned);
        }

        Ok(())
    }

    /// Track a new shared reference.
    ///
    /// Returns the new total number of shared references.
    ///
    /// This fails when:
    /// - There exists an accessible mutable reference.
    /// - There exist `usize::MAX >> 1` shared references.
    pub fn increment_shared(&self) -> Result<usize, BorrowStateErr> {
        self.ensure_not_poisoned()?;

        let prev_refstate = RefState::from_increment(&self.refstate);
        prev_refstate.ensure_can_ref()?;

        Ok(prev_refstate.shared_count + 1)
    }

    /// Untrack an existing shared reference.
    ///
    /// Returns the new total number of shared references.
    ///
    /// This fails when:
    /// - There are currently no tracked shared references.
    pub fn decrement_shared(&self) -> Result<usize, BorrowStateErr> {
        self.ensure_not_poisoned()?;

        let prev_refstate = RefState::from_decrement(&self.refstate);

        if prev_refstate.shared_count == 0 {
            self.poison("shared counter decremented while no shared reference exists")?
        }

        if prev_refstate.has_exclusive {
            self.poison("shared reference tracked while exclusive mutable reference exists")?;
        }

        Ok(prev_refstate.shared_count)
    }

    /// Track a new mutable reference.
    ///
    /// Returns the new total number of mutable references.
    ///
    /// This fails when:
    /// - There exists an accessible mutable reference.
    /// - There exists a shared reference.
    ///
    /// Any amount of shared references will prevent [`Self::increment_inaccessible`] from succeeding.
    pub fn increment_mut(&self) -> Result<(), BorrowStateErr> {
        self.ensure_not_poisoned()?;

        if let Some(prev_refstate) = self
            .refstate
            .compare_exchange(0, EXCLUSIVE_REF_MASK, Ordering::AcqRel, Ordering::Relaxed)
            .err()
            .map(RefState::from_usize)
        {
            if prev_refstate.shared_count != 0 {
                self.poison("tried to acquire mutable reference while shared references exists")?;
            } else {
                self.poison("tried to acquire mutable reference while other accessible mutable reference exists")?;
            }
        }

        Ok(())
    }

    /// Untrack an existing mutable reference.
    ///
    /// Returns the new total number of mutable references.
    ///
    /// This fails when:
    /// - There are currently no mutable references.
    pub fn decrement_mut(&self) -> Result<(), BorrowStateErr> {
        self.ensure_not_poisoned()?;

        if let Some(prev_refstate) = self
            .refstate
            .compare_exchange(EXCLUSIVE_REF_MASK, 0, Ordering::AcqRel, Ordering::Relaxed)
            .err()
            .map(RefState::from_usize)
        {
            if prev_refstate.shared_count != 0 {
                self.poison("tried to decrement mutable reference while shared references exists")?;
            } else {
                self.poison("tried to decrement mutable reference count while no accessible mutable reference exists")?;
            }
        }

        Ok(())
    }

    pub fn set_inaccessible(&self) -> Result<usize, BorrowStateErr> {
        if let Some(prev_refstate) = self
            .refstate
            .compare_exchange(EXCLUSIVE_REF_MASK, 0, Ordering::AcqRel, Ordering::Relaxed)
            .err()
            .map(RefState::from_usize)
        {
            if !prev_refstate.has_exclusive {
                self.poison("cannot set current reference as inaccessible when no accessible reference exists")?;
            } else if prev_refstate.shared_count != 0 {
                self.poison("mutable refrence tracked when shared references existed")?;
            }
        }
        let prev = self.inaccessible_count.fetch_add(1, Ordering::AcqRel);

        if prev == usize::MAX {
            self.poison("Attempted to set inaccessible count with an overflow")?
        }

        Ok(prev + 1)
    }

    pub fn unset_inaccessible(&self, stack_depth: usize) -> Result<usize, BorrowStateErr> {
        if let Err(prev_count) = self.inaccessible_count.compare_exchange(
            stack_depth,
            stack_depth - 1,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            self.poison(format!(
                "Attempted to drop inaccessible borrows in wrong order.
                Expected: {stack_depth}, actual: {prev_count}"
            ))?
        }
        self.increment_mut()?;

        Ok(stack_depth - 1)
    }
}
