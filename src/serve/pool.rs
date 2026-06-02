//! A fixed pool of `try_clone`'d `DbStore`s, one checked out per
//! in-flight query. Concurrency is bounded by the caller's semaphore
//! (permits == pool size), so `checkout` always finds a free store.

use std::sync::Mutex;

use anyhow::Result;

use crate::db::DbStore;

pub struct ConnectionPool {
    stores: Mutex<Vec<DbStore>>,
}

impl ConnectionPool {
    /// Build `size` sibling connections from `primary` via
    /// `try_clone_store`. All share the one already-opened database.
    pub fn build(primary: &DbStore, size: usize) -> Result<Self> {
        let mut stores = Vec::with_capacity(size);
        for _ in 0..size {
            stores.push(primary.try_clone_store()?);
        }
        Ok(Self {
            stores: Mutex::new(stores),
        })
    }

    /// Take a store out of the pool. Panics only if called more times
    /// than `size` without checking in — the semaphore prevents that.
    pub fn checkout(&self) -> DbStore {
        self.stores
            .lock()
            .unwrap()
            .pop()
            .expect("pool exhausted: concurrency exceeded pool size")
    }

    pub fn checkin(&self, store: DbStore) {
        self.stores.lock().unwrap().push(store);
    }
}
