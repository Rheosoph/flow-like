use crate::dialect::DbDialect;
use crate::retry::{RetryPolicy, retry_transaction};
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict, SimpleExpr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr,
    DeleteMany, EntityTrait, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter,
    QueryOrder, QuerySelect, Select, TryGetable, UpdateMany, Value,
};

/// Rows written per transaction by the chunked helpers.
///
/// Well under DSQL's 3,000-row cap so foreign-key cascades and the 10 MiB
/// transaction size still fit, and small enough that a lost commit race
/// re-runs cheaply. The same chunk is used on every engine so behaviour does
/// not depend on where the API happens to run.
pub const DEFAULT_WRITE_CHUNK: usize = 1_000;

/// Estimated bytes written per transaction by the size-aware helpers, leaving
/// headroom under DSQL's 10 MiB write-set and wire-message caps for the
/// statement text and the row overhead the estimate does not see.
pub const DEFAULT_WRITE_BYTES: usize = 8 * 1024 * 1024;

/// What a bounded sweep achieved before it returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchOutcome {
    pub rows: u64,
    /// The chunk budget ran out while rows still matched; call again.
    pub stopped_early: bool,
}

/// The wire size a row contributes to an insert, for rows whose JSONB
/// columns make a fixed row count a poor proxy for transaction size.
pub trait EstimateBytes {
    fn estimate_bytes(&self) -> usize;
}

fn clamp_chunk(chunk: usize) -> usize {
    chunk.clamp(1, DEFAULT_WRITE_CHUNK.max(1))
}

fn single_key_column<E: EntityTrait>() -> Result<E::Column, DbErr> {
    let mut keys = E::PrimaryKey::iter();
    match (keys.next(), keys.next()) {
        (Some(key), None) => Ok(key.into_column()),
        _ => Err(DbErr::Custom(format!(
            "{} has a composite primary key; use the tuple variant",
            E::default().table_name()
        ))),
    }
}

/// Split `models` into chunks of at most `max_rows` rows whose estimated
/// size stays under `max_bytes`. A single row larger than `max_bytes` gets a
/// chunk of its own; order is preserved.
pub fn chunk_by_rows_and_bytes<T: EstimateBytes>(
    models: Vec<T>,
    max_rows: usize,
    max_bytes: usize,
) -> Vec<Vec<T>> {
    chunk_by_rows_and_bytes_with(models, max_rows, max_bytes, T::estimate_bytes)
}

/// [`chunk_by_rows_and_bytes`] with the estimate as a function, for row types
/// from other crates that cannot implement [`EstimateBytes`] here.
pub fn chunk_by_rows_and_bytes_with<T>(
    models: Vec<T>,
    max_rows: usize,
    max_bytes: usize,
    estimate: impl Fn(&T) -> usize,
) -> Vec<Vec<T>> {
    let max_rows = clamp_chunk(max_rows);
    let max_bytes = max_bytes.max(1);
    let mut chunks = Vec::new();
    let mut current: Vec<T> = Vec::new();
    let mut current_bytes = 0usize;
    for model in models {
        let size = estimate(&model);
        let overflows = current.len() >= max_rows || current_bytes.saturating_add(size) > max_bytes;
        if overflows && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes = current_bytes.saturating_add(size);
        current.push(model);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Insert `models` in chunks of `chunk` rows, each chunk in its own retried
/// transaction.
///
/// Callers that need all-or-nothing semantics must provide it themselves
/// (a resumable marker row, or deterministic ids so a re-run is an upsert);
/// a failure part-way leaves the earlier chunks committed. `on_conflict` is
/// applied to every chunk, which is also what makes a repeated chunk after an
/// ambiguous commit harmless.
pub async fn insert_in_chunks<A>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    models: Vec<A>,
    chunk: usize,
    on_conflict: Option<OnConflict>,
) -> Result<u64, DbErr>
where
    A: ActiveModelTrait + Clone + Send + Sync + 'static,
{
    let chunks = models
        .chunks(clamp_chunk(chunk))
        .map(<[A]>::to_vec)
        .collect();
    insert_chunks(db, dialect, chunks, on_conflict).await
}

/// [`insert_in_chunks`] for rows with JSONB payloads: chunks close at
/// `max_rows` rows or `max_bytes` estimated bytes, whichever comes first.
pub async fn insert_in_sized_chunks<A>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    models: Vec<A>,
    max_rows: usize,
    max_bytes: usize,
    on_conflict: Option<OnConflict>,
) -> Result<u64, DbErr>
where
    A: ActiveModelTrait + EstimateBytes + Clone + Send + Sync + 'static,
{
    let chunks = chunk_by_rows_and_bytes(models, max_rows, max_bytes);
    insert_chunks(db, dialect, chunks, on_conflict).await
}

/// Insert already-split `chunks`, each in its own retried transaction; the
/// caller guarantees every chunk fits the row and byte budgets.
pub async fn insert_chunks<A>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    chunks: Vec<Vec<A>>,
    on_conflict: Option<OnConflict>,
) -> Result<u64, DbErr>
where
    A: ActiveModelTrait + Clone + Send + Sync + 'static,
{
    let mut inserted = 0u64;
    for rows in chunks {
        if rows.is_empty() {
            continue;
        }
        let on_conflict = on_conflict.clone();
        inserted += retry_transaction::<_, u64, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let rows = rows.clone();
                let on_conflict = on_conflict.clone();
                Box::pin(async move { insert_rows::<A, _>(txn, rows, on_conflict).await })
            },
        )
        .await?;
    }
    Ok(inserted)
}

/// Insert `models` inside an open transaction, one statement per `chunk`
/// rows so no single wire message exceeds the 10 MiB cap.
///
/// The caller keeps atomicity and therefore owns the row budget: everything
/// written through one transaction must stay under
/// [`crate::DSQL_MAX_ROWS_PER_TRANSACTION`].
pub async fn insert_chunked_in_txn<A, C>(
    txn: &C,
    models: Vec<A>,
    chunk: usize,
) -> Result<u64, DbErr>
where
    A: ActiveModelTrait,
    C: ConnectionTrait,
{
    let chunk = clamp_chunk(chunk);
    let mut inserted = 0u64;
    let mut models = models.into_iter().peekable();
    while models.peek().is_some() {
        let rows: Vec<A> = models.by_ref().take(chunk).collect();
        inserted += insert_rows::<A, C>(txn, rows, None).await?;
    }
    Ok(inserted)
}

async fn insert_rows<A, C>(
    txn: &C,
    rows: Vec<A>,
    on_conflict: Option<OnConflict>,
) -> Result<u64, DbErr>
where
    A: ActiveModelTrait,
    C: ConnectionTrait,
{
    let mut insert = <A::Entity as EntityTrait>::insert_many(rows);
    if let Some(on_conflict) = on_conflict {
        insert = insert.on_conflict(on_conflict);
    }
    insert.exec_without_returning(txn).await
}

/// The next page of single-column primary keys matching `condition`, in key
/// order, starting after `after`.
fn key_page<E>(
    condition: &Condition,
    key_column: E::Column,
    after: Option<SimpleExpr>,
    chunk: usize,
) -> Select<E>
where
    E: EntityTrait,
{
    let mut query = E::find().filter(condition.clone());
    if let Some(after) = after {
        query = query.filter(Expr::col(key_column).gt(after));
    }
    query
        .select_only()
        .column(key_column)
        .order_by_asc(key_column)
        .limit(chunk as u64)
}

/// `UPDATE … WHERE pk IN (ids) AND condition`: the condition is repeated so
/// a row that stopped matching between the page select and the write is
/// left alone, exactly as a single conditional statement would.
fn update_keys<E>(
    condition: &Condition,
    key_column: E::Column,
    ids: Vec<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    set: &[(E::Column, SimpleExpr)],
) -> UpdateMany<E>
where
    E: EntityTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: Into<Value>,
{
    let mut update = E::update_many()
        .filter(key_column.is_in(ids))
        .filter(condition.clone());
    for (column, expr) in set {
        update = update.col_expr(*column, expr.clone());
    }
    update
}

/// `DELETE … WHERE pk IN (ids) AND condition`; see [`update_keys`].
fn delete_keys<E>(
    condition: &Condition,
    key_column: E::Column,
    ids: Vec<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
) -> DeleteMany<E>
where
    E: EntityTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: Into<Value>,
{
    E::delete_many()
        .filter(key_column.is_in(ids))
        .filter(condition.clone())
}

/// Apply `set` to every row of `E` matching `condition`, `chunk` rows per
/// transaction, walking the single-column primary key so an update that
/// leaves the row still matching cannot loop forever.
pub async fn update_in_batches<E>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    condition: Condition,
    set: Vec<(E::Column, SimpleExpr)>,
    chunk: usize,
) -> Result<u64, DbErr>
where
    E: EntityTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Into<Value> + TryGetable + Clone + Send + Sync + 'static,
{
    let key_column = single_key_column::<E>()?;
    let chunk = clamp_chunk(chunk);
    let mut updated = 0u64;
    let mut after: Option<<E::PrimaryKey as PrimaryKeyTrait>::ValueType> = None;
    loop {
        let ids: Vec<<E::PrimaryKey as PrimaryKeyTrait>::ValueType> = key_page::<E>(
            &condition,
            key_column,
            after.clone().map(|last| Expr::val(last).into()),
            chunk,
        )
        .into_tuple()
        .all(db)
        .await?;
        let Some(last) = ids.last().cloned() else {
            return Ok(updated);
        };
        let fetched = ids.len();
        let condition = condition.clone();
        let set = set.clone();
        updated += retry_transaction::<_, u64, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let update = update_keys::<E>(&condition, key_column, ids.clone(), &set);
                Box::pin(async move { update.exec(txn).await.map(|result| result.rows_affected) })
            },
        )
        .await?;
        if fetched < chunk {
            return Ok(updated);
        }
        after = Some(last);
    }
}

/// Delete every row of `E` matching `condition`, `chunk` rows per transaction,
/// until none are left or `max_chunks` transactions have run.
///
/// Rows are selected by single-column primary key in ascending order so two
/// concurrent sweepers make progress instead of repeatedly colliding on the
/// same batch. Entities with composite keys use
/// [`delete_in_batches_by_tuple`].
pub async fn delete_in_batches<E>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    condition: Condition,
    chunk: usize,
    max_chunks: Option<usize>,
) -> Result<BatchOutcome, DbErr>
where
    E: EntityTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Into<Value> + TryGetable + Clone + Send + Sync + 'static,
{
    let key_column = single_key_column::<E>()?;
    let chunk = clamp_chunk(chunk);
    let mut outcome = BatchOutcome::default();
    let mut chunks = 0usize;
    loop {
        if max_chunks.is_some_and(|budget| chunks >= budget) {
            outcome.stopped_early = true;
            return Ok(outcome);
        }
        let ids: Vec<<E::PrimaryKey as PrimaryKeyTrait>::ValueType> =
            key_page::<E>(&condition, key_column, None, chunk)
                .into_tuple()
                .all(db)
                .await?;
        if ids.is_empty() {
            return Ok(outcome);
        }
        let fetched = ids.len();
        chunks += 1;
        let condition = condition.clone();
        outcome.rows += retry_transaction::<_, u64, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let delete = delete_keys::<E>(&condition, key_column, ids.clone());
                Box::pin(async move { delete.exec(txn).await.map(|result| result.rows_affected) })
            },
        )
        .await?;
        if fetched < chunk {
            return Ok(outcome);
        }
    }
}

/// [`delete_in_batches`] for entities with a composite primary key, matching
/// rows with a row-value `(k1, k2, …) IN ((…), (…))` predicate.
pub async fn delete_in_batches_by_tuple<E>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    condition: Condition,
    chunk: usize,
    max_chunks: Option<usize>,
) -> Result<BatchOutcome, DbErr>
where
    E: EntityTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: Clone + Send + Sync + 'static,
{
    let key_columns: Vec<E::Column> = E::PrimaryKey::iter()
        .map(PrimaryKeyToColumn::into_column)
        .collect();
    let chunk = clamp_chunk(chunk);
    let mut outcome = BatchOutcome::default();
    let mut chunks = 0usize;
    loop {
        if max_chunks.is_some_and(|budget| chunks >= budget) {
            outcome.stopped_early = true;
            return Ok(outcome);
        }
        let mut query = E::find()
            .filter(condition.clone())
            .select_only()
            .columns(key_columns.iter().copied());
        for column in &key_columns {
            query = query.order_by_asc(*column);
        }
        let keys: Vec<<E::PrimaryKey as PrimaryKeyTrait>::ValueType> =
            query.limit(chunk as u64).into_tuple().all(db).await?;
        if keys.is_empty() {
            return Ok(outcome);
        }
        let fetched = keys.len();
        chunks += 1;
        let key_columns = key_columns.clone();
        let condition = condition.clone();
        outcome.rows += retry_transaction::<_, u64, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let key_tuple = Expr::tuple(key_columns.iter().map(|column| Expr::col(*column)));
                let delete = E::delete_many()
                    .filter(key_tuple.in_tuples(keys.clone()))
                    .filter(condition.clone());
                Box::pin(async move { delete.exec(txn).await.map(|result| result.rows_affected) })
            },
        )
        .await?;
        if fetched < chunk {
            return Ok(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    mod row {
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "Row")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: String,
            pub expires_at: i64,
            pub status: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    struct Sized(usize);

    impl EstimateBytes for Sized {
        fn estimate_bytes(&self) -> usize {
            self.0
        }
    }

    fn sizes(chunks: &[Vec<Sized>]) -> Vec<Vec<usize>> {
        chunks
            .iter()
            .map(|chunk| chunk.iter().map(|row| row.0).collect())
            .collect()
    }

    fn expired() -> Condition {
        Condition::all().add(row::Column::ExpiresAt.lt(100))
    }

    #[test]
    fn chunks_are_clamped_to_the_write_budget() {
        assert_eq!(clamp_chunk(0), 1);
        assert_eq!(clamp_chunk(10), 10);
        assert_eq!(clamp_chunk(DEFAULT_WRITE_CHUNK * 5), DEFAULT_WRITE_CHUNK);
    }

    #[test]
    fn byte_chunking_closes_a_chunk_on_rows_or_bytes() {
        let rows = vec![Sized(4), Sized(4), Sized(4), Sized(1), Sized(1), Sized(1)];
        assert_eq!(
            sizes(&chunk_by_rows_and_bytes(rows, 2, 100)),
            vec![vec![4, 4], vec![4, 1], vec![1, 1]]
        );
        let rows = vec![Sized(4), Sized(4), Sized(4), Sized(1), Sized(1), Sized(1)];
        assert_eq!(
            sizes(&chunk_by_rows_and_bytes(rows, 100, 8)),
            vec![vec![4, 4], vec![4, 1, 1, 1]]
        );
    }

    #[test]
    fn byte_chunking_isolates_an_oversized_row_and_keeps_order() {
        let rows = vec![Sized(1), Sized(50), Sized(1)];
        assert_eq!(
            sizes(&chunk_by_rows_and_bytes(rows, 100, 8)),
            vec![vec![1], vec![50], vec![1]]
        );
        assert!(chunk_by_rows_and_bytes(Vec::<Sized>::new(), 10, 10).is_empty());
    }

    #[test]
    fn byte_chunking_never_exceeds_the_row_cap() {
        let rows = (0..(DEFAULT_WRITE_CHUNK * 2 + 1))
            .map(|_| Sized(1))
            .collect();
        let chunks = chunk_by_rows_and_bytes(rows, usize::MAX, usize::MAX);
        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= DEFAULT_WRITE_CHUNK)
        );
    }

    #[test]
    fn key_pages_walk_the_primary_key_in_order() {
        let first = key_page::<row::Entity>(&expired(), row::Column::Id, None, 500)
            .build(DatabaseBackend::Postgres)
            .to_string();
        assert_eq!(
            first,
            r#"SELECT "Row"."id" FROM "Row" WHERE "Row"."expires_at" < 100 ORDER BY "Row"."id" ASC LIMIT 500"#
        );
        let next = key_page::<row::Entity>(
            &expired(),
            row::Column::Id,
            Some(Expr::val("k").into()),
            500,
        )
        .build(DatabaseBackend::Postgres)
        .to_string();
        assert_eq!(
            next,
            r#"SELECT "Row"."id" FROM "Row" WHERE "Row"."expires_at" < 100 AND "id" > 'k' ORDER BY "Row"."id" ASC LIMIT 500"#
        );
    }

    #[test]
    fn writes_by_key_repeat_the_condition() {
        let delete = delete_keys::<row::Entity>(
            &expired(),
            row::Column::Id,
            vec!["a".to_string(), "b".to_string()],
        )
        .build(DatabaseBackend::Postgres)
        .to_string();
        assert_eq!(
            delete,
            r#"DELETE FROM "Row" WHERE "Row"."id" IN ('a', 'b') AND "Row"."expires_at" < 100"#
        );
        let update = update_keys::<row::Entity>(
            &expired(),
            row::Column::Id,
            vec!["a".to_string()],
            &[(row::Column::Status, Expr::val("timeout").into())],
        )
        .build(DatabaseBackend::Postgres)
        .to_string();
        assert_eq!(
            update,
            r#"UPDATE "Row" SET "status" = 'timeout' WHERE "Row"."id" IN ('a') AND "Row"."expires_at" < 100"#
        );
    }

    mod pair {
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "Pair")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub run_id: String,
            #[sea_orm(primary_key, auto_increment = false)]
            pub position: i32,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    #[test]
    fn composite_keys_are_refused_by_the_single_key_helpers() {
        assert!(single_key_column::<row::Entity>().is_ok());
        let error = single_key_column::<pair::Entity>().unwrap_err().to_string();
        assert!(error.contains("Pair"), "{error}");
        assert!(error.contains("tuple variant"), "{error}");
    }

    #[test]
    fn composite_keys_delete_by_row_value_tuples() {
        let key_tuple = Expr::tuple([
            Expr::col(pair::Column::RunId),
            Expr::col(pair::Column::Position),
        ]);
        let delete = pair::Entity::delete_many()
            .filter(key_tuple.in_tuples([("run-1".to_string(), 0i32), ("run-1".to_string(), 1)]))
            .filter(Condition::all().add(pair::Column::RunId.is_in(["run-1"])))
            .build(DatabaseBackend::Postgres)
            .to_string();
        assert_eq!(
            delete,
            r#"DELETE FROM "Pair" WHERE ("run_id", "position") IN (('run-1', 0), ('run-1', 1)) AND "Pair"."run_id" IN ('run-1')"#
        );
    }
}
