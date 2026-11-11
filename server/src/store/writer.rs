//! Serializes write transactions that mutate Conversation state.
use std::sync::Arc;

use sqlx::{pool::PoolConnection, Connection, Sqlite, Transaction};
use tokio::sync::{Mutex, MutexGuard};

use crate::Result;

use super::Store;

/// An unsuccessful transaction must never return its connection to the pool:
/// failed SQLite commits can leave sqlx's transaction depth nonzero.
pub(crate) struct WriteSession<'a> {
    _write: MutexGuard<'a, ()>,
    connection: PoolConnection<Sqlite>,
    committed: bool,
}

impl Store {
    pub(crate) async fn write_session(&self) -> Result<WriteSession<'_>> {
        Ok(WriteSession {
            _write: self.writes.lock().await,
            connection: self.pool.acquire().await?,
            committed: false,
        })
    }
}

impl WriteSession<'_> {
    pub(crate) async fn begin(&mut self) -> Result<Transaction<'_, Sqlite>> {
        Ok(self.connection.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub(crate) fn committed(&mut self) {
        self.committed = true;
    }
}

impl Drop for WriteSession<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.connection.close_on_drop();
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct WriteCoordinator {
    lock: Arc<Mutex<()>>,
}

impl WriteCoordinator {
    pub(crate) async fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConversationId;

    #[tokio::test]
    async fn aborting_a_write_rolls_back_and_releases_the_coordinator() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::connect(&format!(
            "sqlite://{}",
            directory.path().join("test.db").display()
        ))
        .await
        .unwrap();
        let writer_store = store.clone();
        let (ready, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut session = writer_store.write_session().await.unwrap();
            let mut tx = session.begin().await.unwrap();
            sqlx::query("INSERT INTO blobs(blob_id, data, created_at_ms) VALUES (?, ?, 0)")
                .bind(vec![42_u8; 32])
                .bind(vec![1_u8])
                .execute(&mut *tx)
                .await
                .unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
            tx.commit().await.unwrap();
            session.committed();
        });
        received.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for _ in 0..16 {
                store
                    .ensure_conversation(&ConversationId::new("after-abort"))
                    .await
                    .unwrap();
            }
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs WHERE blob_id = ?")
                .bind(vec![42_u8; 32])
                .fetch_one(store.pool())
                .await
                .unwrap();
            assert_eq!(count, 0);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn failed_commit_and_abandoned_transaction_do_not_poison_pool() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::connect(&format!(
            "sqlite://{}",
            directory.path().join("test.db").display()
        ))
        .await
        .unwrap();
        for abandon in [false, true] {
            let mut session = store.write_session().await.unwrap();
            let mut tx = session.begin().await.unwrap();
            if abandon {
                std::mem::forget(tx);
            } else {
                sqlx::query("PRAGMA defer_foreign_keys = ON")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO checkpoint_messages(checkpoint_id, ordinal, conversation_id, message_id) VALUES (999999, 0, 'missing', 'missing')").execute(&mut *tx).await.unwrap();
                assert!(tx.commit().await.is_err());
            }
            drop(session);
            for _ in 0..16 {
                store
                    .ensure_conversation(&ConversationId::new("poison-check"))
                    .await
                    .unwrap();
            }
        }
    }
}
