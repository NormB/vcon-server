//! A small pool of connections: at most `pool` open at once, each made by
//! the connector and checked as it was made, handed out one caller at a
//! time and kept for the next while it stays sound.
//!
//! - A caller waits at most `connect_timeout` for a free connection.
//! - A connection whose socket closed is never handed out again.
//! - A call that failed for any reason but a statement's own error
//!   (I/O, a protocol error, a timeout of the connection) drops its
//!   connection, and so does SQLSTATE 25006: the server went read-only
//!   under it, as a primary turned standby does.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_postgres::Client;
use tokio_postgres::error::SqlState;

use super::connect::{ConnectError, Connected, Connector, Server};

/// Why no connection was handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PoolError {
    /// Every connection stayed in use for the whole wait.
    Busy {
        /// The pool's size.
        size: u32,
        /// How long the caller waited.
        waited: Duration,
    },
    /// A new connection was needed and could not be made.
    Connect(ConnectError),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolError::Busy { size, waited } => write!(
                f,
                "all {size} connections (catalog.postgres.pool) stayed in use for {} s \
                 (catalog.postgres.connect_timeout)",
                waited.as_secs()
            ),
            PoolError::Connect(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PoolError {}

struct Shared {
    connector: Connector,
    idle: Mutex<Vec<Connected>>,
    slots: Arc<Semaphore>,
    size: u32,
    wait: Duration,
}

/// The pool; clones share it.
#[derive(Clone)]
pub(crate) struct Pool {
    shared: Arc<Shared>,
}

impl fmt::Debug for Pool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pool")
            .field("size", &self.shared.size)
            .field("idle", &self.idle())
            .finish_non_exhaustive()
    }
}

/// One connection, the caller's until dropped; then it goes back to the
/// pool unless it closed or a call on it failed in a way that drops it.
pub(crate) struct Lease {
    connected: Option<Connected>,
    keep: bool,
    shared: Arc<Shared>,
    _slot: OwnedSemaphorePermit,
}

impl fmt::Debug for Lease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lease")
            .field("server", &self.connected.as_ref().map(|c| &c.server))
            .field("keep", &self.keep)
            .finish_non_exhaustive()
    }
}

/// Whether a call that failed with `e` leaves its connection unfit for the
/// next caller: anything but an error the server answered, and the server's
/// SQLSTATE 25006 (read-only transaction), which a standby answers.
#[must_use]
pub(crate) fn must_drop(e: &tokio_postgres::Error) -> bool {
    match e.code() {
        None => true,
        Some(code) => *code == SqlState::READ_ONLY_SQL_TRANSACTION,
    }
}

impl Pool {
    /// A pool of at most `size` connections from `connector`, whose callers
    /// wait at most `wait` for one.
    pub(crate) fn new(connector: Connector, size: u32, wait: Duration) -> Pool {
        let permits = usize::try_from(size)
            .unwrap_or(usize::MAX)
            .min(Semaphore::MAX_PERMITS);
        Pool {
            shared: Arc::new(Shared {
                connector,
                idle: Mutex::new(Vec::new()),
                slots: Arc::new(Semaphore::new(permits)),
                size,
                wait,
            }),
        }
    }

    /// A connection: a kept one that is still open, or a new one.
    pub(crate) async fn get(&self) -> Result<Lease, PoolError> {
        let shared = &self.shared;
        let slot = tokio::time::timeout(shared.wait, shared.slots.clone().acquire_owned())
            .await
            .map_err(|_| PoolError::Busy {
                size: shared.size,
                waited: shared.wait,
            })?
            .expect("the pool never closes its semaphore");
        let kept = loop {
            let next = shared
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop();
            match next {
                Some(c) if c.client.is_closed() => {}
                other => break other,
            }
        };
        let connected = match kept {
            Some(connected) => connected,
            None => shared
                .connector
                .connect()
                .await
                .map_err(PoolError::Connect)?,
        };
        Ok(Lease {
            connected: Some(connected),
            keep: true,
            shared: shared.clone(),
            _slot: slot,
        })
    }

    /// [`Pool::get`] from a thread that is not running async code, such as
    /// one of `spawn_blocking`'s, on the runtime `handle` names, where the
    /// connection's task runs.
    #[cfg_attr(
        not(all(test, feature = "backend-tests")),
        expect(
            dead_code,
            reason = "the catalog's statements use it from the next change; the tests do now"
        )
    )]
    pub(crate) fn get_blocking(&self, handle: &tokio::runtime::Handle) -> Result<Lease, PoolError> {
        handle.block_on(self.get())
    }

    /// How many of the connections waiting unused have closed.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn idle_closed(&self) -> usize {
        self.shared
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|c| c.client.is_closed())
            .count()
    }

    /// How many connections wait unused.
    pub(crate) fn idle(&self) -> usize {
        self.shared
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Lease {
    /// The connection.
    pub(crate) fn client(&self) -> &Client {
        &self.connected().client
    }

    /// The server it reached.
    pub(crate) fn server(&self) -> &Server {
        &self.connected().server
    }

    /// Records that a call on this connection failed with `e`: when
    /// [`must_drop`] says so, the connection is closed when the lease ends
    /// instead of going back to the pool.
    pub(crate) fn failed(&mut self, e: &tokio_postgres::Error) {
        if must_drop(e) {
            self.keep = false;
        }
    }

    /// Closes the connection when the lease ends, whatever it did: for a
    /// connection left in a state that cannot be known, such as one whose
    /// COMMIT or ROLLBACK got no answer.
    pub(crate) fn discard(&mut self) {
        self.keep = false;
    }

    fn connected(&self) -> &Connected {
        self.connected
            .as_ref()
            .expect("a lease holds its connection until dropped")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(connected) = self.connected.take()
            && self.keep
            && !connected.client.is_closed()
        {
            self.shared
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(connected);
        }
    }
}
