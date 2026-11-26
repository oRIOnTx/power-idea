//! A live connection that heals itself: when an operation fails because the link
//! dropped, it reconnects (a few times, with backoff) and retries the operation once.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::error::{RemoteError, Result};
use crate::fs::RemoteFs;

/// Reconnect attempts after a drop before giving up.
pub const MAX_RECONNECTS: u32 = 3;

pub type Connector = Box<dyn Fn() -> Result<Arc<dyn RemoteFs>> + Send + Sync>;

pub struct Connection {
    connector: Connector,
    fs: RwLock<Arc<dyn RemoteFs>>,
    /// Serialises reconnects so parallel operations do not each open a session.
    reconnecting: Mutex<()>,
    reconnects: AtomicU32,
    backoff: Duration,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("reconnects", &self.reconnects())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Connects once; the connector is kept for reconnects.
    pub fn open(connector: Connector) -> Result<Self> {
        let fs = connector()?;
        Ok(Self {
            connector,
            fs: RwLock::new(fs),
            reconnecting: Mutex::new(()),
            reconnects: AtomicU32::new(0),
            backoff: Duration::from_secs(1),
        })
    }

    #[cfg(test)]
    fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// How many times this connection has been re-established.
    pub fn reconnects(&self) -> u32 {
        self.reconnects.load(Ordering::Relaxed)
    }

    pub fn is_alive(&self) -> bool {
        self.current().is_alive()
    }

    fn current(&self) -> Arc<dyn RemoteFs> {
        match self.fs.read() {
            Ok(fs) => fs.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Runs `op`, reconnecting first if the link is known dead, and retrying once if it
    /// drops during the operation.
    pub fn run<T>(&self, op: impl Fn(&dyn RemoteFs) -> Result<T>) -> Result<T> {
        let mut fs = self.current();
        if !fs.is_alive() {
            fs = self.reconnect(&fs)?;
        }
        match op(&*fs) {
            Err(err) if err.is_connection_lost() => {
                log::warn!("remote: connection lost ({}); reconnecting", err.detail);
                let fs = self.reconnect(&fs)?;
                op(&*fs)
            }
            result => result,
        }
    }

    /// Replaces `broken` with a fresh session (unless another thread already did).
    fn reconnect(&self, broken: &Arc<dyn RemoteFs>) -> Result<Arc<dyn RemoteFs>> {
        let _guard = self
            .reconnecting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = self.current();
        if !Arc::ptr_eq(&current, broken) && current.is_alive() {
            return Ok(current);
        }
        let mut last = RemoteError::network("connection lost");
        for attempt in 0..MAX_RECONNECTS {
            if attempt > 0 {
                std::thread::sleep(self.backoff * 2u32.pow(attempt - 1));
            }
            match (self.connector)() {
                Ok(fs) => {
                    if let Ok(mut slot) = self.fs.write() {
                        *slot = fs.clone();
                    }
                    self.reconnects.fetch_add(1, Ordering::Relaxed);
                    return Ok(fs);
                }
                // Credentials or host key problems will not fix themselves: stop right away.
                Err(err) if !err.is_connection_lost() => return Err(err),
                Err(err) => last = err,
            }
        }
        Err(RemoteError::network(format!(
            "connection lost; {MAX_RECONNECTS} reconnect attempts failed ({})",
            last.detail
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::error::ErrorKind;
    use crate::fs::memory::MemoryFs;

    fn connection(fail_after: usize) -> (Connection, Arc<AtomicUsize>, Arc<MemoryFs>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let fs = Arc::new(MemoryFs::new());
        fs.put("/a.txt", "hello");
        let (counter, shared) = (calls.clone(), fs.clone());
        let conn = Connection::open(Box::new(move || {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            if n > fail_after {
                return Err(RemoteError::network("refused"));
            }
            shared.dead.store(false, Ordering::Relaxed);
            Ok(shared.clone() as Arc<dyn RemoteFs>)
        }))
        .unwrap()
        .with_backoff(Duration::from_millis(1));
        (conn, calls, fs)
    }

    #[test]
    fn reconnects_after_a_drop() {
        let (conn, calls, fs) = connection(usize::MAX);
        fs.dead.store(true, Ordering::Relaxed);
        assert_eq!(conn.run(|fs| fs.read("/a.txt")).unwrap(), b"hello");
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(conn.reconnects(), 1);
    }

    #[test]
    fn gives_up_after_the_limit() {
        let (conn, calls, fs) = connection(0);
        fs.dead.store(true, Ordering::Relaxed);
        let err = conn.run(|fs| fs.read("/a.txt")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Network);
        assert!(err.detail.contains("3 reconnect attempts"));
        assert_eq!(calls.load(Ordering::Relaxed), 1 + MAX_RECONNECTS as usize);
    }

    #[test]
    fn other_errors_are_not_retried() {
        let (conn, calls, _) = connection(usize::MAX);
        let err = conn.run(|fs| fs.read("/missing")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
