//! Group commit of reads' audit records. A read ends with a transaction
//! that checks its version is still current and records the read; the
//! record must be durable before the body is returned. Reads that reach
//! that step while another read's transaction holds the catalog wait in
//! one queue, and the next of them to run takes the whole queue into one
//! transaction: each read's check and record in a part of its own, then
//! one commit, so one sync of the log makes all of them durable. No read
//! is answered before the commit that holds its record has returned.

use std::sync::{Condvar, Mutex, MutexGuard};

use uuid::Uuid;

use super::{State, Store, StoreError, Tx, commit, state, version_row};
use crate::audit::{Action, Actor, Entry};
use crate::backend::Packed;
use crate::ids::{KeyId, VersionId};

/// A read's last step: what it read, and how.
pub(super) struct Confirm {
    pub(super) key: KeyId,
    pub(super) action: Action,
    pub(super) uuid: Uuid,
    pub(super) version: VersionId,
    /// Whether the version's body opened.
    pub(super) opened: bool,
    /// The version's place when it was read, if it is packed.
    pub(super) place: Option<Packed>,
}

/// What the last step found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Confirmed {
    /// The version is still current; the read is recorded with the status
    /// its body gave.
    Answer,
    /// The version changed, or its pack file moved: the read starts again.
    /// Nothing is recorded.
    Again,
    /// The vCon is gone or not found; the refusal is recorded.
    Refused(StoreError),
}

/// The reads waiting for their last step, and whether one of them is
/// running it.
#[derive(Default)]
struct Queue {
    /// Some read holds, or is waiting for, the catalog for a batch.
    leading: bool,
    /// The reads not yet taken into a batch, each with its ticket.
    waiting: Vec<(u64, Confirm)>,
    /// The outcomes of batches committed, by ticket, until each read takes
    /// its own.
    done: Vec<(u64, Result<Confirmed, StoreError>)>,
    next: u64,
}

/// The queue of one Store's reads.
#[derive(Default)]
pub(super) struct Group {
    queue: Mutex<Queue>,
    turn: Condvar,
    /// The size of each batch committed, for tests.
    #[cfg(test)]
    pub(super) batches: Mutex<Vec<usize>>,
    /// The parts batches have opened, for tests.
    #[cfg(test)]
    pub(super) parts: std::sync::atomic::AtomicUsize,
}

impl Group {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .expect("no thread panicked holding the queue")
    }

    /// Runs `read`'s last step, in a batch with every other read waiting
    /// for it, and returns its outcome once the batch is committed.
    pub(super) fn confirm(&self, store: &Store, read: Confirm) -> Result<Confirmed, StoreError> {
        let mut queue = self.lock();
        let ticket = queue.next;
        queue.next += 1;
        queue.waiting.push((ticket, read));
        loop {
            if let Some(at) = queue.done.iter().position(|(t, _)| *t == ticket) {
                return queue.done.swap_remove(at).1;
            }
            if queue.leading {
                queue = self
                    .turn
                    .wait(queue)
                    .expect("no thread panicked holding the queue");
                continue;
            }
            queue.leading = true;
            drop(queue);
            // The lead is dropped, handing the turn on, before the queue is
            // locked again.
            Lead { group: self }.run(store);
            queue = self.lock();
        }
    }

    /// The reads waiting for a batch, for tests.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        self.lock().waiting.len()
    }
}

/// The read that runs a batch. Dropped, also on a panic, it hands the turn
/// on, so no read waits for a batch that will not run.
struct Lead<'a> {
    group: &'a Group,
}

impl Lead<'_> {
    fn run(&self, store: &Store) {
        let tx = store.catalog.immediate();
        // Taken once the catalog is held: every read that came while this
        // one waited for it is in the batch.
        let batch = std::mem::take(&mut self.group.lock().waiting);
        let outcomes = match tx {
            Ok(tx) => batch_in(store, tx, &batch),
            Err(e) => vec![Err(e); batch.len()],
        };
        #[cfg(test)]
        self.group
            .batches
            .lock()
            .expect("batches")
            .push(batch.len());
        let mut queue = self.group.lock();
        queue
            .done
            .extend(batch.iter().map(|(t, _)| *t).zip(outcomes));
    }
}

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        let mut queue = self.group.lock();
        queue.leading = false;
        drop(queue);
        self.group.turn.notify_all();
    }
}

/// Runs each read's step in `tx`, then commits if any read recorded
/// something; with nothing recorded the transaction is rolled back. With
/// more than one read, each runs in a part of its own, so that one read's
/// failure undoes only its own work (PostgreSQL aborts a whole transaction
/// at a failed statement until its part is rolled back); a lone read needs
/// no part, since the whole transaction is its own. A failed commit fails
/// every read that recorded something; a read that starts again recorded
/// nothing, and still starts again.
fn batch_in(
    store: &Store,
    mut tx: Tx,
    batch: &[(u64, Confirm)],
) -> Vec<Result<Confirmed, StoreError>> {
    let lone = batch.len() == 1;
    let mut outcomes: Vec<_> = batch
        .iter()
        .map(|(_, read)| {
            if lone {
                one(store, &mut tx, read)
            } else {
                #[cfg(test)]
                store
                    .group
                    .parts
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                in_part(&mut tx, |tx| one(store, tx, read))
            }
        })
        .collect();
    let recorded = |outcome: &Result<Confirmed, StoreError>| {
        matches!(outcome, Ok(Confirmed::Answer | Confirmed::Refused(_)))
    };
    if !outcomes.iter().any(recorded) {
        return outcomes;
    }
    if let Err(e) = commit(tx) {
        for outcome in &mut outcomes {
            if recorded(outcome) {
                *outcome = Err(e);
            }
        }
    }
    outcomes
}

/// `step` in a part of `tx`: kept if it succeeds, undone if it fails.
fn in_part(
    tx: &mut Tx,
    step: impl FnOnce(&mut Tx) -> Result<Confirmed, StoreError>,
) -> Result<Confirmed, StoreError> {
    tx.begin_part()?;
    match step(tx) {
        Ok(outcome) => {
            if let Err(e) = tx.release_part() {
                let _ = tx.rollback_part();
                return Err(e);
            }
            Ok(outcome)
        }
        Err(e) => {
            tx.rollback_part()?;
            Err(e)
        }
    }
}

/// One read's step: the checks and the record the read made in its own
/// transaction before batches.
fn one(store: &Store, tx: &mut Tx, read: &Confirm) -> Result<Confirmed, StoreError> {
    let entry = |status, version| Entry {
        actor: Actor::Key(read.key),
        action: read.action,
        target: Some(read.uuid.to_string()),
        version,
        status,
    };
    match state(tx, read.uuid)? {
        // A packed version's file may have been rewritten and deleted
        // after its place was read: read again at the new place. The same
        // place still failing is the answer.
        State::Live(now)
            if now == read.version
                && !read.opened
                && read.place.is_some()
                && version_row(tx, read.version).ok().map(|r| r.place)
                    != Some(read.place.clone()) =>
        {
            Ok(Confirmed::Again)
        }
        State::Live(now) if now == read.version => {
            let status = if read.opened { 200 } else { 500 };
            store.record(tx, &entry(status, Some(read.version)))?;
            Ok(Confirmed::Answer)
        }
        State::Live(_) => Ok(Confirmed::Again),
        State::Gone => {
            store.record(tx, &entry(410, None))?;
            Ok(Confirmed::Refused(StoreError::Gone))
        }
        State::Absent => {
            store.record(tx, &entry(404, None))?;
            Ok(Confirmed::Refused(StoreError::NotFound))
        }
    }
}
