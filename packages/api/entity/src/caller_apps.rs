//! Lineage rows mirroring `ExecutionRun.callerAppChain`.
//!
//! The chain itself stays on the run row as a JSON array (ordered reads); the
//! `ExecutionRunCallerApp` rows give membership queries ("runs whose chain
//! contains app X") a btree index without array or GIN support.

use sea_orm::{
    ActiveModelTrait, ActiveValue, DatabaseConnection, DbErr, EntityTrait, Set, TransactionTrait,
};

use crate::{execution_run, execution_run_caller_app};

pub fn caller_app_rows(
    run_id: &str,
    chain: &[String],
) -> Vec<execution_run_caller_app::ActiveModel> {
    chain
        .iter()
        .enumerate()
        .map(|(position, app_id)| execution_run_caller_app::ActiveModel {
            run_id: Set(run_id.to_string()),
            app_id: Set(app_id.clone()),
            position: Set(position as i32),
        })
        .collect()
}

/// Inserts the run and its caller-app rows in one transaction so the mirror
/// can never lag behind the chain column.
pub async fn insert_run_with_caller_apps(
    db: &DatabaseConnection,
    run: execution_run::ActiveModel,
) -> Result<execution_run::Model, DbErr> {
    let chain: Vec<String> = match &run.caller_app_chain {
        ActiveValue::Set(Some(chain)) | ActiveValue::Unchanged(Some(chain)) => chain.0.clone(),
        _ => Vec::new(),
    };
    let txn = db.begin().await?;
    let model = run.insert(&txn).await?;
    let rows = caller_app_rows(&model.id, &chain);
    if !rows.is_empty() {
        execution_run_caller_app::Entity::insert_many(rows)
            .exec(&txn)
            .await?;
    }
    txn.commit().await?;
    Ok(model)
}
