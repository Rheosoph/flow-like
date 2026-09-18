//! Save an immutable dispatch before invoking Lambda. Recovery repeats delivery only.
use crate::{error::ApiError, state::AppState};
use flow_like_storage::{
    Path,
    object_store::{ObjectStoreExt, PutMode, PutOptions},
};
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};

#[derive(Clone, Debug, FromQueryResult)]
pub struct DispatchIntent {
    pub id: String,
    #[sea_orm(from_alias = "jobId")]
    pub job_id: String,
    #[sea_orm(from_alias = "functionName")]
    pub function_name: String,
    #[sea_orm(from_alias = "tenantId")]
    pub tenant_id: Option<String>,
    #[sea_orm(from_alias = "payloadPath")]
    pub payload_path: String,
}
fn sql(query: &str, args: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, query, args)
}
pub async fn stage(
    state: &AppState,
    id: &str,
    job_id: &str,
    function: &str,
    tenant: Option<&str>,
    payload: &[u8],
) -> Result<DispatchIntent, ApiError> {
    let path = Path::from(format!(
        "system/dispatch/{}/{}.json",
        blake3::hash(id.as_bytes()).to_hex(),
        blake3::hash(job_id.as_bytes()).to_hex()
    ));
    match state
        .meta_bucket
        .as_generic()
        .put_opts(
            &path,
            payload.to_vec().into(),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await
    {
        Ok(_) | Err(flow_like_storage::object_store::Error::AlreadyExists { .. }) => {}
        Err(error) => return Err(ApiError::internal_error(error.into())),
    }
    let now = chrono::Utc::now().timestamp_millis();
    // The payload is stored before the row so recovery never claims an intent it cannot read.
    // No row back means an intent already exists or the workflow reservation does not.
    if let Some(row) = state.db.query_one_raw(sql("INSERT INTO \"CloudDispatchIntent\" (id,\"payerId\",\"jobId\",\"functionName\",\"tenantId\",\"payloadPath\",state,\"nextAttemptAt\",\"expiresAt\",attempts,\"createdAt\") SELECT $1::TEXT,\"payerId\",$2::TEXT,$3::TEXT,$4::TEXT,$5::TEXT,'pending',$6::BIGINT,deadline,0,$6::BIGINT FROM \"QuotaOperation\" WHERE id=$1::TEXT AND kind='workflow' ON CONFLICT (id) DO NOTHING RETURNING id,\"jobId\",\"functionName\",\"tenantId\",\"payloadPath\"",vec![id.into(),job_id.into(),function.into(),tenant.map(str::to_owned).into(),path.to_string().into(),now.into()])).await? {
        return Ok(DispatchIntent::from_query_result(&row, "")?);
    }
    let Some(row) = state
        .db
        .query_one_raw(sql(
            "SELECT * FROM \"CloudDispatchIntent\" WHERE id=$1",
            vec![id.into()],
        ))
        .await?
    else {
        let _ = state.meta_bucket.as_generic().delete(&path).await;
        return Err(ApiError::NOT_FOUND);
    };
    let intent = DispatchIntent::from_query_result(&row, "")?;
    // A repeated stage carries a new job id, so its payload is not the one the intent points at.
    if intent.payload_path != path.to_string() {
        let _ = state.meta_bucket.as_generic().delete(&path).await;
    }
    Ok(intent)
}
pub async fn claim(state: &AppState, id: &str) -> Result<bool, ApiError> {
    let now = chrono::Utc::now().timestamp_millis();
    let op = state
        .db
        .query_one_raw(sql(
            "SELECT status FROM \"QuotaOperation\" WHERE id=$1 AND \"cancelRequested\"=FALSE AND deadline>$2",
            vec![id.into(),now.into()],
        ))
        .await?;
    if op
        .map(|r| r.try_get::<String>("", "status"))
        .transpose()?
        .as_deref()
        != Some("reserved")
    {
        state
            .db
            .execute_raw(sql(
                "UPDATE \"CloudDispatchIntent\" SET state='resolved' WHERE id=$1",
                vec![id.into()],
            ))
            .await?;
        return Ok(false);
    }
    Ok(state.db.execute_raw(sql("UPDATE \"CloudDispatchIntent\" SET state='sending',attempts=attempts+1,\"nextAttemptAt\"=$2 WHERE id=$1 AND state IN ('pending','sending') AND \"nextAttemptAt\"<=$3 AND \"expiresAt\">$3",vec![id.into(),(now+60_000).into(),now.into()])).await?.rows_affected()==1)
}
pub async fn accepted(state: &AppState, id: &str) -> Result<(), ApiError> {
    state
        .db
        .execute_raw(sql(
            "UPDATE \"CloudDispatchIntent\" SET state='accepted' WHERE id=$1",
            vec![id.into()],
        ))
        .await?;
    Ok(())
}
pub async fn due(state: &AppState) -> Result<Vec<DispatchIntent>, ApiError> {
    let rows=state.db.query_all_raw(sql("SELECT * FROM \"CloudDispatchIntent\" WHERE state IN ('pending','sending') AND \"nextAttemptAt\"<=$1 ORDER BY \"nextAttemptAt\" LIMIT 25",vec![chrono::Utc::now().timestamp_millis().into()])).await?;
    rows.iter()
        .map(|r| DispatchIntent::from_query_result(r, "").map_err(ApiError::from))
        .collect()
}
pub async fn payload(state: &AppState, intent: &DispatchIntent) -> Result<Vec<u8>, ApiError> {
    let response = state
        .meta_bucket
        .as_generic()
        .get(&Path::from(intent.payload_path.as_str()))
        .await?;
    if response.meta.size > 1_048_576 {
        return Err(ApiError::internal("Staged Lambda event exceeds its limit"));
    }
    Ok(response.bytes().await?.to_vec())
}
