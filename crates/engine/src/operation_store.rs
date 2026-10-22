//! Durable operation claims, state transitions and native acceptance receipts.
//!
//! The source key deduplicates platform input within this database. It provides
//! no remote idempotency guarantee: an uncertain dispatch is never made sendable
//! again. SQL state predicates arbitrate competing claims without holding a
//! database transaction across provider I/O.
use super::{Store, StoreError};
use session::operation::OperationState;
use sqlx::Row;

impl Store {
    /// Atomically persist a native receipt and finish the current dispatch claim.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Transition`] if the operation is no longer dispatching,
    /// or [`StoreError::Database`] on persistence failure. A lost local receipt
    /// does not prove that the remote write failed and must not trigger a resend.
    pub async fn accept_receipt(
        &self,
        id: &str,
        receipt: &session::provider::Receipt,
    ) -> Result<(), StoreError> {
        let receipt = serde_json::to_string(receipt).map_err(|_| StoreError::Database)?;
        let changed = sqlx::query(
            "UPDATE operations SET receipt=?,state='accepted',updated_at=unixepoch() WHERE id=? AND state='dispatching'",
        )
            .bind(receipt)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?
            .rows_affected();
        if changed != 1 {
            return Err(StoreError::Transition);
        }
        Ok(())
    }

    pub async fn receipt(
        &self,
        id: &str,
    ) -> Result<Option<session::provider::Receipt>, StoreError> {
        let text: Option<String> = sqlx::query_scalar("SELECT receipt FROM operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        text.map(|text| serde_json::from_str(&text).map_err(|_| StoreError::Database))
            .transpose()
    }

    pub async fn probe_threads(
        &self,
        instance: &str,
    ) -> Result<std::collections::HashSet<String>, StoreError> {
        let receipts = sqlx::query(
            "SELECT receipt FROM operations WHERE instance_id=? AND source_key LIKE 'probe:%:create' AND state='accepted'",
        )
        .bind(instance)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        let mut threads = std::collections::HashSet::new();
        for row in receipts {
            let Some(raw) = row.get::<Option<String>, _>("receipt") else {
                continue;
            };
            let receipt: session::provider::Receipt =
                serde_json::from_str(&raw).map_err(|_| StoreError::Database)?;
            threads.insert(receipt.thread);
        }
        Ok(threads)
    }

    /// Reserve one operation per platform input identity, or return its existing ID.
    ///
    /// Reusing a source key for another provider/session is a conflict, even if
    /// the original operation failed. Callers must still claim the returned ID
    /// through [`Store::transition`] before dispatching.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::SourceConflict`] for a different target and
    /// [`StoreError::Database`] on persistence failure.
    pub async fn reserve_operation(
        &self,
        source_key: &str,
        instance: &str,
        thread: &str,
    ) -> Result<String, StoreError> {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO operations(id,source_key,instance_id,thread_id,state) VALUES(?,?,?,?,'received') ON CONFLICT(source_key) DO NOTHING",
        )
        .bind(&id)
        .bind(source_key)
        .bind(instance)
        .bind(thread)
        .execute(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        let row = sqlx::query("SELECT id,instance_id,thread_id FROM operations WHERE source_key=?")
            .bind(source_key)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        if row.get::<String, _>("instance_id") != instance
            || row.get::<String, _>("thread_id") != thread
        {
            return Err(StoreError::SourceConflict);
        }
        Ok(row.get("id"))
    }

    /// Claim a legal transition only if the persisted state still equals `from`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Transition`] for an illegal edge, a missing operation
    /// or a lost claim; returns [`StoreError::Database`] for SQLite failures.
    pub async fn transition(
        &self,
        id: &str,
        from: OperationState,
        to: OperationState,
    ) -> Result<(), StoreError> {
        if !from.can_transition(to) {
            return Err(StoreError::Transition);
        }
        // The predicate and update form one claim. A read followed by an
        // unconditional update would allow concurrent deliveries to dispatch twice.
        let count = sqlx::query(
            "UPDATE operations SET state=?,updated_at=unixepoch() WHERE id=? AND state=?",
        )
        .bind(state_name(to))
        .bind(id)
        .bind(state_name(from))
        .execute(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?
        .rows_affected();
        if count != 1 {
            return Err(StoreError::Transition);
        }
        Ok(())
    }

    pub async fn operation_state(&self, id: &str) -> Result<OperationState, StoreError> {
        let name: String = sqlx::query_scalar("SELECT state FROM operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        parse_state(&name)
    }
}

fn state_name(state: OperationState) -> &'static str {
    match state {
        OperationState::Received => "received",
        OperationState::Prepared => "prepared",
        OperationState::Dispatching => "dispatching",
        OperationState::Accepted => "accepted",
        OperationState::Rejected => "rejected",
        OperationState::OutcomeUnknown => "outcome_unknown",
    }
}

fn parse_state(value: &str) -> Result<OperationState, StoreError> {
    match value {
        "received" => Ok(OperationState::Received),
        "prepared" => Ok(OperationState::Prepared),
        "dispatching" => Ok(OperationState::Dispatching),
        "accepted" => Ok(OperationState::Accepted),
        "rejected" => Ok(OperationState::Rejected),
        "outcome_unknown" => Ok(OperationState::OutcomeUnknown),
        _ => Err(StoreError::Database),
    }
}
