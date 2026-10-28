//! The dispatch gate's rules, each a plain function or type a test can
//! drive: the scope a tool needs, the per-key rate, the concurrency cap and
//! the refusal sent when one says no.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::ErrorData;
use rmcp::model::{ErrorCode, ToolAnnotations};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::Limits;
use crate::access::Scope;
use crate::api::Problem;
use crate::ids::KeyId;

/// The scope a tool needs, from its name's area and its hints: `vcon_`
/// tools need read when read-only, delete when destructive, else write;
/// `legal_` tools legal; `audit_` tools audit; `admin_` tools admin. None,
/// which refuses every call, for a tool that does not state both hints,
/// states both true, or has no known area.
#[must_use]
pub fn scope(name: &str, hints: Option<&ToolAnnotations>) -> Option<Scope> {
    let hints = hints?;
    let (read_only, destructive) = (hints.read_only_hint?, hints.destructive_hint?);
    if read_only && destructive {
        return None;
    }
    match name.split_once('_')?.0 {
        "vcon" if read_only => Some(Scope::Read),
        "vcon" if destructive => Some(Scope::Delete),
        "vcon" => Some(Scope::Write),
        "legal" => Some(Scope::Legal),
        "audit" => Some(Scope::Audit),
        "admin" => Some(Scope::Admin),
        _ => None,
    }
}

/// The length of one rate window.
const WINDOW: Duration = Duration::from_secs(1);

/// Past this many keys in the table, windows that have ended are dropped.
const KEPT: usize = 1024;

/// Each key's calls in its current one-second window.
#[derive(Debug, Default)]
pub(super) struct Windows(Mutex<HashMap<KeyId, (Instant, u32)>>);

impl Windows {
    /// Counts one call by `key` at `now` against `limit`, or says how long
    /// until its window ends when the window is full. A key's window starts
    /// at its first call and lasts one second.
    pub(super) fn take(&self, key: KeyId, limit: u32, now: Instant) -> Result<(), Duration> {
        let mut windows = self.0.lock().expect("rate windows");
        if windows.len() > KEPT {
            windows.retain(|_, (start, _)| now.saturating_duration_since(*start) < WINDOW);
        }
        let (start, calls) = windows.entry(key).or_insert((now, 0));
        if now.saturating_duration_since(*start) >= WINDOW {
            *start = now;
            *calls = 0;
        }
        if *calls >= limit {
            return Err((*start + WINDOW).saturating_duration_since(now));
        }
        *calls += 1;
        Ok(())
    }
}

/// The server's state the gate checks calls against.
#[derive(Debug)]
pub(super) struct Gate {
    pub(super) limits: Limits,
    pub(super) windows: Windows,
    slots: Arc<Semaphore>,
}

impl Gate {
    pub(super) fn new(limits: Limits) -> Gate {
        Gate {
            limits,
            windows: Windows::default(),
            slots: Arc::new(Semaphore::new(limits.concurrent as usize)),
        }
    }

    /// A slot for one call, held until the permit drops; None at once when
    /// every slot is taken, never a wait.
    pub(super) fn slot(&self) -> Option<OwnedSemaphorePermit> {
        self.slots.clone().try_acquire_owned().ok()
    }
}

/// The JSON-RPC error a refusal is sent as: its code by kind, the problem's
/// title as its message, and its problem document as its data, with
/// `retry_after_ms` when the wait is known.
pub(super) fn refused(problem: Problem, retry: Option<Duration>) -> ErrorData {
    let code = match problem {
        Problem::UnknownTool | Problem::InvalidArguments => ErrorCode::INVALID_PARAMS,
        Problem::Forbidden => ErrorCode(-32001),
        _ => ErrorCode(-32000),
    };
    let mut data = problem.document();
    if let Some(retry) = retry {
        // At least 1, so a wait shorter than a millisecond is not read as
        // none.
        let millis = u64::try_from(retry.as_millis()).unwrap_or(u64::MAX).max(1);
        data["retry_after_ms"] = millis.into();
    }
    ErrorData::new(code, problem.name().1, Some(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(read_only: Option<bool>, destructive: Option<bool>) -> ToolAnnotations {
        let mut hints = ToolAnnotations::new();
        hints.read_only_hint = read_only;
        hints.destructive_hint = destructive;
        hints
    }

    #[test]
    fn the_scope_comes_from_the_area_and_the_hints() {
        let (t, f) = (Some(true), Some(false));
        for (name, read_only, destructive, want) in [
            ("vcon_get", t, f, Some(Scope::Read)),
            ("vcon_delete", f, t, Some(Scope::Delete)),
            ("vcon_store", f, f, Some(Scope::Write)),
            ("legal_get", t, f, Some(Scope::Legal)),
            ("legal_restore", f, f, Some(Scope::Legal)),
            ("legal_hold", f, t, Some(Scope::Legal)),
            ("audit_read", t, f, Some(Scope::Audit)),
            ("admin_tenant_list", t, f, Some(Scope::Admin)),
            ("admin_tenant_remove", f, t, Some(Scope::Admin)),
            // Both hints, and only consistent ones, are needed.
            ("vcon_get", None, f, None),
            ("vcon_get", t, None, None),
            ("vcon_get", t, t, None),
            ("admin_tenant_list", None, None, None),
            // An area no scope belongs to.
            ("tenant_list", t, f, None),
            ("vcons", t, f, None),
            ("", t, f, None),
        ] {
            assert_eq!(
                scope(name, Some(&hints(read_only, destructive))),
                want,
                "{name} {read_only:?} {destructive:?}"
            );
        }
        assert_eq!(scope("vcon_get", None), None, "no hints at all");
    }

    #[test]
    fn a_key_has_its_calls_per_window_and_then_waits_for_its_end() {
        let windows = Windows::default();
        let (a, b) = (KeyId([1; 8]), KeyId([2; 8]));
        let start = Instant::now();
        assert_eq!(windows.take(a, 2, start), Ok(()));
        let later = start + Duration::from_millis(300);
        assert_eq!(windows.take(a, 2, later), Ok(()));
        assert_eq!(
            windows.take(a, 2, later),
            Err(Duration::from_millis(700)),
            "full until the window ends"
        );
        assert_eq!(
            windows.take(b, 2, later),
            Ok(()),
            "another key's own window"
        );
        let next = start + WINDOW;
        assert_eq!(windows.take(a, 2, next), Ok(()), "a new window");
        assert_eq!(windows.take(a, 2, next), Ok(()));
        assert!(windows.take(a, 2, next).is_err());
    }

    #[test]
    fn ended_windows_are_dropped_once_the_table_is_large() {
        let windows = Windows::default();
        let start = Instant::now();
        for n in 0..=KEPT as u64 {
            let key = KeyId(n.to_be_bytes());
            windows.take(key, 1, start).expect("a first call");
        }
        let later = start + WINDOW;
        windows.take(KeyId([0xff; 8]), 1, later).expect("a call");
        assert_eq!(windows.0.lock().expect("windows").len(), 1);
    }

    #[test]
    fn a_full_server_refuses_a_slot_at_once_and_frees_it_on_drop() {
        let gate = Gate::new(Limits {
            calls_per_second: 1,
            concurrent: 2,
            ..Limits::DEFAULT
        });
        let first = gate.slot().expect("a slot");
        let second = gate.slot().expect("a slot");
        assert!(gate.slot().is_none(), "no third slot");
        drop(first);
        assert!(gate.slot().is_some(), "freed");
        drop(second);
    }

    #[test]
    fn a_refusal_carries_its_problem_document_and_its_code() {
        for (problem, code) in [
            (Problem::UnknownTool, -32602),
            (Problem::InvalidArguments, -32602),
            (Problem::Forbidden, -32001),
            (Problem::RateLimited, -32000),
            (Problem::AtCapacity, -32000),
            (Problem::AuditFailed, -32000),
        ] {
            let error = refused(problem, None);
            assert_eq!(error.code.0, code, "{problem:?}");
            assert_eq!(error.data, Some(problem.document()), "{problem:?}");
            assert_eq!(error.message, problem.name().1);
        }
        let error = refused(Problem::RateLimited, Some(Duration::from_micros(10)));
        let data = error.data.expect("data");
        assert_eq!(data["retry_after_ms"], 1, "never 0");
        assert_eq!(data["status"], 429);
    }
}
