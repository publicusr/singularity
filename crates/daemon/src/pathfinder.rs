//! The query port under the Catalogue (D78): one Pathfinder query, `op` and its variables in, Spotify's answer out.
//!
//! [`Fetch`] is the port. [`Pathfinder`] is the adapter that talks to Spotify: it looks the operation's hash up in
//! the [`QueryRegistry`], asks once more after a hang, refreshes the registry when a hash is retired, and doesn't
//! crawl the web player again for an operation the last crawl didn't have (C3). Tests of the Catalogue use a
//! fixtures adapter instead; tests of these rules use a stand-in [`Raw`] call.

use crate::queries::QueryRegistry;
use librespot_core::{error::ErrorKind, session::Session};
use serde_json::Value;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Runs one private-API query.
pub trait Fetch: Send {
    fn query(&mut self, op: &str, vars: Value) -> impl Future<Output = anyhow::Result<Value>> + Send;
}

/// The call under the adapter: one persisted query with its hash, as Spotify answers it.
pub trait Raw: Send {
    /// `Err` when no call can go out at all (logged out), before a hash is looked up.
    fn ready(&self) -> anyhow::Result<()>;
    fn call(&mut self, op: &str, hash: &str, vars: Value) -> impl Future<Output = Result<Value, librespot_core::Error>> + Send;
}

/// The adapter that talks to Spotify.
pub struct Pathfinder<R> {
    registry: QueryRegistry,
    raw: R,
}

impl<R: Raw> Pathfinder<R> {
    pub fn new(registry: QueryRegistry, raw: R) -> Pathfinder<R> {
        Pathfinder { registry, raw }
    }
}

impl<R: Raw> Fetch for Pathfinder<R> {
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        self.raw.ready()?;
        let hash = self.registry.hash(op).await?;
        // a request that hangs is asked once more: the stuck connection went with the abandoned request
        let first = match self.raw.call(op, &hash, vars.clone()).await {
            Err(e) if e.kind == ErrorKind::DeadlineExceeded => {
                tracing::warn!("`{op}`: {e}; asking again");
                self.raw.call(op, &hash, vars.clone()).await
            }
            r => r,
        };
        match first {
            Err(e) if e.kind == ErrorKind::FailedPrecondition => {
                tracing::warn!("`{op}` hash retired; refreshing the query registry");
                self.registry.refresh().await?;
                let hash = self.registry.hash(op).await?;
                Ok(self.raw.call(op, &hash, vars).await?)
            }
            r => Ok(r?),
        }
    }
}

/// The Catalogue's Spotify session, shared with the adapter: the Catalogue logs in and out, the adapter asks through
/// whichever session is current.
pub type SessionSlot = Arc<Mutex<Option<Session>>>;

/// [`Raw`] through the logged-in session, with [`crate::spotify`]'s deadline.
pub struct SessionRaw(pub SessionSlot);

impl Raw for SessionRaw {
    fn ready(&self) -> anyhow::Result<()> {
        match self.0.lock().unwrap().as_ref() {
            Some(_) => Ok(()),
            None => anyhow::bail!("not logged in to Spotify"),
        }
    }

    async fn call(&mut self, op: &str, hash: &str, vars: Value) -> Result<Value, librespot_core::Error> {
        let session = self.0.lock().unwrap().clone().ok_or_else(|| librespot_core::Error::unauthenticated("not logged in to Spotify"))?;
        crate::spotify(session.spclient().pathfinder_query(op, hash, vars)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Spotify as a script: each call takes the next answer and is written down with the hash it used.
    #[derive(Clone, Default)]
    struct Script {
        answers: Arc<Mutex<VecDeque<Result<Value, librespot_core::Error>>>>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Raw for Script {
        fn ready(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn call(&mut self, op: &str, hash: &str, _vars: Value) -> Result<Value, librespot_core::Error> {
            self.calls.lock().unwrap().push((op.into(), hash.into()));
            self.answers.lock().unwrap().pop_front().unwrap_or_else(|| Ok(Value::from("ok")))
        }
    }

    /// A registry knowing `home` as "h1", whose crawls find `found` and are counted.
    fn registry(found: &[(&str, &str)]) -> (QueryRegistry, Arc<AtomicUsize>) {
        let crawls = Arc::new(AtomicUsize::new(0));
        let found: HashMap<String, String> = found.iter().map(|(o, h)| (o.to_string(), h.to_string())).collect();
        let counted = crawls.clone();
        let crawl: crate::queries::Crawl = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let found = found.clone();
            Box::pin(async move { Ok(found) })
        });
        (QueryRegistry::with_crawl(HashMap::from([("home".to_string(), "h1".to_string())]), crawl), crawls)
    }

    #[tokio::test]
    async fn a_query_that_hangs_is_asked_once_more() {
        let (reg, _) = registry(&[]);
        let script = Script::default();
        script.answers.lock().unwrap().push_back(Err(librespot_core::Error::deadline_exceeded("no answer")));
        let mut p = Pathfinder::new(reg, script.clone());
        assert_eq!(p.query("home", Value::Null).await.unwrap(), Value::from("ok"));
        assert_eq!(script.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_retired_hash_refreshes_the_registry_and_is_asked_once_with_the_new_one() {
        let (reg, crawls) = registry(&[("home", "h2")]);
        let script = Script::default();
        script.answers.lock().unwrap().push_back(Err(librespot_core::Error::failed_precondition("412")));
        let mut p = Pathfinder::new(reg, script.clone());
        assert_eq!(p.query("home", Value::Null).await.unwrap(), Value::from("ok"));
        assert_eq!(crawls.load(Ordering::SeqCst), 1);
        let hashes: Vec<String> = script.calls.lock().unwrap().iter().map(|(_, h)| h.clone()).collect();
        assert_eq!(hashes, ["h1", "h2"]);
    }

    /// C3 (2026-11-20 bug hunt): an operation the web player stopped having started a whole crawl (≈160 requests,
    /// the Catalogue blocked meanwhile) on every call. Once a fresh crawl hasn't found it, it fails at once.
    #[tokio::test]
    async fn an_operation_a_fresh_crawl_did_not_find_is_not_crawled_for_again() {
        let (reg, crawls) = registry(&[("home", "h1")]);
        let mut p = Pathfinder::new(reg, Script::default());
        for _ in 0..3 {
            assert!(p.query("areEntitiesInLibrary", Value::Null).await.is_err());
        }
        assert_eq!(crawls.load(Ordering::SeqCst), 1, "one crawl, then the answer is remembered");
        assert_eq!(p.query("home", Value::Null).await.unwrap(), Value::from("ok"), "the rest still works");
    }

    #[tokio::test]
    async fn logged_out_nothing_is_looked_up() {
        let (reg, crawls) = registry(&[]);
        let mut p = Pathfinder::new(reg, SessionRaw(SessionSlot::default()));
        let e = p.query("someOpNoOneHas", Value::Null).await.unwrap_err().to_string();
        assert!(e.contains("not logged in"), "{e}");
        assert_eq!(crawls.load(Ordering::SeqCst), 0);
    }
}
