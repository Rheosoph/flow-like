// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use aurora_dsql_sqlx_connector::Result;
use sqlx::Row;

use super::test_util::build_conn_str;

#[tokio::test]
async fn test_dsql_connection() -> Result<()> {
    let conn_str = build_conn_str();

    let mut conn = aurora_dsql_sqlx_connector::connection::connect(&conn_str).await?;

    let row = sqlx::query("SELECT 1 as value")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let value: i32 = row.get("value");

    assert_eq!(value, 1);
    Ok(())
}
