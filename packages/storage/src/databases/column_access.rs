//! Column read restrictions: which top-level columns a reader may see, and the checks that
//! keep hidden columns out of projections, filters, searches and SQL. A hidden column reads
//! as missing, so its name never shows up in a schema, a row or an error.

use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::{Result as DataFusionResult, Statistics};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown, TableType};
use datafusion::physical_plan::ExecutionPlan;
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_types::{Result, async_trait};
use lance_index::scalar::inverted::query::FtsQuery;
use lancedb::Connection;

use crate::databases::vector::schema::TableInputRejected;

/// Top-level columns of `schema` that `reader` may not read.
pub fn hidden_columns(schema: &Schema, reader: &ColumnReader) -> Vec<String> {
    if reader.is_unrestricted() {
        return Vec::new();
    }
    schema
        .fields()
        .iter()
        .filter(|field| !reader.can_read_field(field.metadata()))
        .map(|field| field.name().clone())
        .collect()
}

pub fn is_hidden(hidden: &[String], column: &str) -> bool {
    hidden
        .iter()
        .any(|hidden| hidden.eq_ignore_ascii_case(column))
}

/// `schema` without its `hidden` columns.
pub fn without_hidden_columns(schema: &Schema, hidden: &[String]) -> Schema {
    if hidden.is_empty() {
        return schema.clone();
    }
    Schema::new_with_metadata(
        schema
            .fields()
            .iter()
            .filter(|field| !is_hidden(hidden, field.name()))
            .cloned()
            .collect::<Vec<_>>(),
        schema.metadata().clone(),
    )
}

/// The visible columns of `schema`, in storage order.
pub fn visible_columns(schema: &Schema, hidden: &[String]) -> Vec<String> {
    schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .filter(|name| !is_hidden(hidden, name))
        .collect()
}

pub fn no_such_column(table: &str, column: &str) -> TableInputRejected {
    TableInputRejected(format!("Table '{table}' has no column '{column}'"))
}

/// Refuses an SQL expression or filter that names a hidden column. Every word token counts,
/// quoted or not, so no spelling reaches a hidden column.
pub fn reject_hidden_references(expression: &str, hidden: &[String], table: &str) -> Result<()> {
    use datafusion::sql::sqlparser::{
        dialect::GenericDialect,
        tokenizer::{Token, Tokenizer},
    };
    if hidden.is_empty() {
        return Ok(());
    }
    let tokens = Tokenizer::new(&GenericDialect {}, expression)
        .tokenize()
        .map_err(|error| {
            TableInputRejected(format!("Invalid expression for table '{table}': {error}"))
        })?;
    for token in tokens {
        if let Token::Word(word) = token
            && is_hidden(hidden, &word.value)
        {
            return Err(no_such_column(table, &word.value).into());
        }
    }
    Ok(())
}

/// Refuses a column list that names a hidden column.
pub fn reject_hidden_columns<S: AsRef<str>>(
    columns: &[S],
    hidden: &[String],
    table: &str,
) -> Result<()> {
    match columns
        .iter()
        .find(|column| is_hidden(hidden, column.as_ref()))
    {
        Some(column) => Err(no_such_column(table, column.as_ref()).into()),
        None => Ok(()),
    }
}

/// The columns every leaf of a full-text query searches, or `None` when a leaf names no
/// column and therefore searches every indexed column.
pub fn full_text_query_columns(query: &FtsQuery) -> Option<Vec<String>> {
    fn collect(query: &FtsQuery, columns: &mut Vec<String>) -> bool {
        match query {
            FtsQuery::Match(query) => query
                .column
                .clone()
                .map(|column| columns.push(column))
                .is_some(),
            FtsQuery::Phrase(query) => query
                .column
                .clone()
                .map(|column| columns.push(column))
                .is_some(),
            FtsQuery::Boost(query) => {
                collect(&query.positive, columns) && collect(&query.negative, columns)
            }
            FtsQuery::MultiMatch(query) => query.match_queries.iter().all(|query| {
                query
                    .column
                    .clone()
                    .map(|column| columns.push(column))
                    .is_some()
            }),
            FtsQuery::Boolean(query) => query
                .should
                .iter()
                .chain(&query.must)
                .chain(&query.must_not)
                .all(|query| collect(query, columns)),
            FtsQuery::CombinedFields(query) => {
                columns.extend(
                    query
                        .weighted_columns()
                        .map(|(column, _)| column.to_string()),
                );
                true
            }
        }
    }
    let mut columns = Vec::new();
    collect(query, &mut columns).then_some(columns)
}

/// Exposes only the visible columns of `inner`. Restricted readers never write through
/// SQL, so the provider is read-only.
pub fn restrict_provider(
    inner: Arc<dyn TableProvider>,
    hidden: &[String],
) -> Arc<dyn TableProvider> {
    let full = inner.schema();
    let visible: Vec<usize> = full
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| !is_hidden(hidden, field.name()))
        .map(|(index, _)| index)
        .collect();
    if visible.len() == full.fields().len() {
        return inner;
    }
    let schema = Arc::new(
        full.project(&visible)
            .expect("projection indices come from the schema itself"),
    );
    Arc::new(ColumnRestrictedProvider {
        inner,
        hidden: hidden.to_vec(),
        visible,
        schema,
    })
}

/// The unrestricted provider and the hidden columns behind a [`restrict_provider`] result.
pub(crate) fn restricted_parts(
    provider: &Arc<dyn TableProvider>,
) -> Option<(&Arc<dyn TableProvider>, &[String])> {
    provider
        .downcast_ref::<ColumnRestrictedProvider>()
        .map(|restricted| (&restricted.inner, restricted.hidden.as_slice()))
}

/// The first table of the connection with a column `reader` may not read.
pub async fn first_table_with_hidden_columns(
    connection: &Connection,
    reader: &ColumnReader,
) -> Result<Option<String>> {
    if reader.is_unrestricted() {
        return Ok(None);
    }
    for name in connection.table_names().execute().await? {
        let table = match connection.open_table(&name).execute().await {
            Ok(table) => table,
            Err(lancedb::Error::TableNotFound { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        if !hidden_columns(table.schema().await?.as_ref(), reader).is_empty() {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

#[derive(Debug)]
struct ColumnRestrictedProvider {
    inner: Arc<dyn TableProvider>,
    hidden: Vec<String>,
    /// Indices of the visible columns in the inner schema.
    visible: Vec<usize>,
    schema: SchemaRef,
}

#[async_trait]
impl TableProvider for ColumnRestrictedProvider {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        self.inner.table_type()
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let projection: Vec<usize> = match projection {
            Some(projection) => projection
                .iter()
                .map(|index| self.visible[*index])
                .collect(),
            None => self.visible.clone(),
        };
        self.inner
            .scan(state, Some(&projection), filters, limit)
            .await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        let statistics = self.inner.statistics()?;
        let column_statistics = self
            .visible
            .iter()
            .map(|index| statistics.column_statistics.get(*index).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(Statistics {
            num_rows: statistics.num_rows,
            total_byte_size: statistics.total_byte_size,
            column_statistics,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use arrow_schema::{DataType, Field};
    use flow_like_storage_contracts::column_metadata::READ_ATTRIBUTES_KEY;
    use lance_index::scalar::inverted::query::{BooleanQuery, MatchQuery, Occur};

    use super::*;

    fn restricted(name: &str, attributes: &str) -> Field {
        Field::new(name, DataType::Utf8, true).with_metadata(HashMap::from([(
            READ_ATTRIBUTES_KEY.to_string(),
            attributes.to_string(),
        )]))
    }

    fn people() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            restricted("salary", r#"["hr"]"#),
            restricted("health", r#"["medical","hr"]"#),
        ])
    }

    #[test]
    fn hides_columns_without_a_matching_attribute() {
        let schema = people();

        assert!(hidden_columns(&schema, &ColumnReader::Unrestricted).is_empty());
        assert_eq!(
            hidden_columns(&schema, &ColumnReader::with_attributes(["medical"])),
            vec!["salary"]
        );
        assert_eq!(
            hidden_columns(&schema, &ColumnReader::with_attributes(["sales"])),
            vec!["salary", "health"]
        );
        let visible = without_hidden_columns(&schema, &["salary".into()]);
        assert_eq!(visible_columns(&visible, &[]), vec!["id", "health"]);
    }

    #[test]
    fn rejects_every_spelling_of_a_hidden_column() {
        let hidden = vec!["salary".to_string()];
        for expression in [
            "salary > 10",
            "SALARY > 10",
            "\"Salary\" > 10",
            "`salary` > 10",
            "id = 1 AND lower(salary) = 'x'",
        ] {
            let error = reject_hidden_references(expression, &hidden, "people")
                .expect_err(expression)
                .to_string();
            assert!(error.contains("has no column"), "{expression}: {error}");
        }
        reject_hidden_references("id = 1 AND name = 'salary'", &hidden, "people").unwrap();
        reject_hidden_references("salary > 1", &[], "people").unwrap();
    }

    #[test]
    fn full_text_queries_must_name_every_column() {
        let named = FtsQuery::Match(MatchQuery::new("ada".into()).with_column(Some("bio".into())));
        assert_eq!(full_text_query_columns(&named), Some(vec!["bio".into()]));

        let unnamed = FtsQuery::Boolean(BooleanQuery::new([
            (Occur::Must, named.clone()),
            (Occur::MustNot, FtsQuery::Match(MatchQuery::new("x".into()))),
        ]));
        assert_eq!(full_text_query_columns(&unnamed), None);
    }
}
