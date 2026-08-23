use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use windows::Win32::Foundation::HWND;

use super::state::OverlayState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AccessError {
    Busy,
    Unavailable,
}

struct Runtime {
    state: RefCell<Option<OverlayState>>,
    busy: Cell<bool>,
    redraw_pending: RefCell<HashSet<isize>>,
    recovery_posted: Cell<bool>,
    servicing_recovery: Cell<bool>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            state: RefCell::new(None),
            busy: Cell::new(false),
            redraw_pending: RefCell::new(HashSet::new()),
            recovery_posted: Cell::new(false),
            servicing_recovery: Cell::new(false),
        }
    }

    fn try_enter(&self) -> Result<BusyGuard<'_>, AccessError> {
        if self.busy.replace(true) {
            return Err(AccessError::Busy);
        }
        Ok(BusyGuard { runtime: self })
    }
}

struct BusyGuard<'a> {
    runtime: &'a Runtime,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.runtime.busy.set(false);
    }
}

thread_local! {
    static RUNTIME: Runtime = Runtime::new();
}

pub(super) fn is_busy() -> bool {
    RUNTIME.with(|runtime| runtime.busy.get())
}

/// Run an immutable owner-thread transaction.
///
/// The busy gate is established before the state borrow so synchronous Win32
/// callbacks cannot acquire mutable state while `operation` is running.
pub(super) fn try_with_state<R>(
    operation: impl FnOnce(&OverlayState) -> R,
) -> Result<R, AccessError> {
    RUNTIME.with(|runtime| {
        let _busy = runtime.try_enter()?;
        let state = runtime.state.try_borrow().map_err(|_| AccessError::Busy)?;
        let state = state.as_ref().ok_or(AccessError::Unavailable)?;
        Ok(operation(state))
    })
}

/// Run a mutable owner-thread transaction.
///
/// State references cannot escape this closure, and the busy flag is restored
/// by RAII on every Rust return path.
pub(super) fn try_with_state_mut<R>(
    operation: impl FnOnce(&mut OverlayState) -> R,
) -> Result<R, AccessError> {
    RUNTIME.with(|runtime| {
        let _busy = runtime.try_enter()?;
        let mut state = runtime
            .state
            .try_borrow_mut()
            .map_err(|_| AccessError::Busy)?;
        let state = state.as_mut().ok_or(AccessError::Unavailable)?;
        Ok(operation(state))
    })
}

/// Exclusively construct and install the owner-thread state.
pub(super) fn initialize<R>(build: impl FnOnce() -> (OverlayState, R)) -> Result<R, AccessError> {
    RUNTIME.with(|runtime| {
        let _busy = runtime.try_enter()?;
        let mut slot = runtime
            .state
            .try_borrow_mut()
            .map_err(|_| AccessError::Busy)?;
        if slot.is_some() {
            return Err(AccessError::Busy);
        }
        let (state, result) = build();
        *slot = Some(state);
        Ok(result)
    })
}

/// Remove state before running owner-thread teardown while keeping callbacks
/// behind the busy gate for the full cleanup transaction.
pub(super) fn shutdown(cleanup: impl FnOnce(OverlayState)) -> Result<(), AccessError> {
    RUNTIME.with(|runtime| {
        let _busy = runtime.try_enter()?;
        let state = runtime
            .state
            .try_borrow_mut()
            .map_err(|_| AccessError::Busy)?
            .take()
            .ok_or(AccessError::Unavailable)?;
        cleanup(state);
        runtime.redraw_pending.borrow_mut().clear();
        runtime.recovery_posted.set(false);
        runtime.servicing_recovery.set(false);
        Ok(())
    })
}

pub(super) fn mark_redraw_requested(hwnd: HWND) -> bool {
    !hwnd.is_invalid()
        && RUNTIME.with(|runtime| runtime.redraw_pending.borrow_mut().insert(hwnd.0 as isize))
}

pub(super) fn retain_redraw_request(hwnd: HWND) {
    let _ = mark_redraw_requested(hwnd);
}

pub(super) fn has_redraw_request(hwnd: HWND) -> bool {
    !hwnd.is_invalid()
        && RUNTIME.with(|runtime| runtime.redraw_pending.borrow().contains(&(hwnd.0 as isize)))
}

pub(super) fn take_redraw_request(hwnd: HWND) -> bool {
    !hwnd.is_invalid()
        && RUNTIME.with(|runtime| {
            runtime
                .redraw_pending
                .borrow_mut()
                .remove(&(hwnd.0 as isize))
        })
}

pub(super) fn clear_redraw_request(hwnd: HWND) {
    RUNTIME.with(|runtime| {
        runtime
            .redraw_pending
            .borrow_mut()
            .remove(&(hwnd.0 as isize));
    });
}

pub(super) fn recovery_is_running() -> bool {
    RUNTIME.with(|runtime| runtime.servicing_recovery.get())
}

/// Mark one recovery message as pending. Returns true only to the caller that
/// should post the message.
pub(super) fn claim_recovery_post() -> bool {
    RUNTIME.with(|runtime| !runtime.recovery_posted.replace(true))
}

pub(super) fn clear_recovery_post() {
    RUNTIME.with(|runtime| runtime.recovery_posted.set(false));
}

/// Serialize compositor recovery and restore its flag through RAII.
pub(super) fn try_service_recovery<R>(operation: impl FnOnce() -> R) -> Option<R> {
    RUNTIME.with(|runtime| {
        if runtime.servicing_recovery.replace(true) {
            return None;
        }
        struct RecoveryGuard<'a>(&'a Cell<bool>);
        impl Drop for RecoveryGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _guard = RecoveryGuard(&runtime.servicing_recovery);
        runtime.recovery_posted.set(false);
        Some(operation())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_busy_gate_is_rejected_and_restored() {
        RUNTIME.with(|runtime| {
            let guard = runtime.try_enter().expect("first transaction enters");
            assert_eq!(runtime.try_enter().err(), Some(AccessError::Busy));
            drop(guard);
            assert!(runtime.try_enter().is_ok());
        });
    }

    #[test]
    fn busy_gate_is_restored_after_a_panic() {
        let result = std::panic::catch_unwind(|| {
            RUNTIME.with(|runtime| {
                let _guard = runtime.try_enter().expect("transaction enters");
                panic!("exercise RAII restoration");
            });
        });
        assert!(result.is_err());
        assert!(!is_busy());
    }

    #[test]
    fn uninitialized_runtime_is_unavailable() {
        assert_eq!(try_with_state(|_| ()).err(), Some(AccessError::Unavailable));
        assert_eq!(
            try_with_state_mut(|_| ()).err(),
            Some(AccessError::Unavailable)
        );
    }

    #[test]
    fn redraw_requests_coalesce_until_taken() {
        let hwnd = HWND(0x1234usize as *mut _);
        clear_redraw_request(hwnd);
        assert!(mark_redraw_requested(hwnd));
        assert!(!mark_redraw_requested(hwnd));
        assert!(has_redraw_request(hwnd));
        assert!(take_redraw_request(hwnd));
        assert!(!has_redraw_request(hwnd));
        assert!(!mark_redraw_requested(HWND::default()));
    }

    #[test]
    fn recovery_service_flag_is_restored_after_a_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = try_service_recovery(|| panic!("exercise recovery guard"));
        });
        assert!(result.is_err());
        assert!(!recovery_is_running());
        assert!(try_service_recovery(|| ()).is_some());
    }
}
