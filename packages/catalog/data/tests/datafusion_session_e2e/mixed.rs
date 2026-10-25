use crate::harness::{Flow, ints, unique};
use crate::lance::{lance_database, register_lance};
use crate::scenarios::{Backend, mount};

/// Mounts every backend plus a Lance table into one session and joins them on id and
/// name, so key and text values must agree across every provider's type mapping.
async fn join_everything(backends: Vec<(&str, Box<dyn Backend>)>) {
    let flow = Flow::local();
    let session = flow.create_session(&unique("mixed")).await;
    let lance = lance_database(&flow).await;
    register_lance(&flow, &session, &lance.connection, "lance_people").await;

    let mut sources = Vec::new();
    for (table, backend) in &backends {
        let source = backend.create_source().await;
        mount(&flow, backend.as_ref(), &session, &source, table, true).await;
        sources.push(source);
    }

    let joins: String = backends
        .iter()
        .map(|(table, _)| format!(" JOIN {table} ON {table}.id = l.id AND {table}.name = l.name"))
        .collect();
    let rows = flow
        .query(
            &session,
            &format!("SELECT l.id FROM lance_people l{joins} WHERE l.score > 80 ORDER BY l.id"),
        )
        .await;
    assert_eq!(ints(&rows, "id"), vec![1, 3, 5]);

    let total: String = backends
        .iter()
        .map(|(table, _)| format!(" + (SELECT COUNT(*) FROM {table})"))
        .collect();
    let rows = flow
        .query(
            &session,
            &format!("SELECT (SELECT COUNT(*) FROM lance_people){total} AS n"),
        )
        .await;
    assert_eq!(ints(&rows, "n"), vec![5 * (backends.len() as i64 + 1)]);

    for ((_, backend), source) in backends.iter().zip(&sources) {
        backend.drop_source(source).await;
    }
}

#[cfg(all(feature = "sqlite", feature = "duckdb"))]
#[tokio::test(flavor = "multi_thread")]
async fn embedded_databases_join_with_lance_in_one_session() {
    join_everything(vec![
        ("sqlite_people", Box::new(crate::sqlite::sqlite())),
        ("duckdb_people", Box::new(crate::duckdb::duckdb())),
    ])
    .await;
}

#[cfg(all(feature = "postgres", feature = "mysql", feature = "clickhouse"))]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait`"]
async fn server_databases_join_with_lance_in_one_session() {
    join_everything(vec![
        ("pg_people", Box::new(crate::postgres::postgres())),
        ("crdb_people", Box::new(crate::postgres::cockroach())),
        ("mysql_people", Box::new(crate::mysql::mysql())),
        ("maria_people", Box::new(crate::mysql::mariadb())),
        ("ch_people", Box::new(crate::clickhouse::Clickhouse)),
    ])
    .await;
}
