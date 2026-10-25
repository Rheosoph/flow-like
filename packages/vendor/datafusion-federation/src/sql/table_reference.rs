use std::sync::Arc;

use datafusion::{
    error::DataFusionError,
    sql::{
        sqlparser::{
            self,
            ast::{FunctionArg, ObjectNamePart},
            dialect::{Dialect, GenericDialect},
            tokenizer::Token,
        },
        TableReference,
    },
};

/// A multipart identifier to a remote table, view or parameterized view.
///
/// RemoteTableRef can be created by parsing from a string representing a table object with optional
/// ```rust
/// use datafusion_federation::sql::RemoteTableRef;
/// use datafusion::sql::sqlparser::dialect::PostgreSqlDialect;
///
/// RemoteTableRef::try_from("myschema.table");
/// RemoteTableRef::try_from(r#"myschema."Table""#);
/// RemoteTableRef::try_from("myschema.view('obj')");
///
/// RemoteTableRef::parse_with_dialect("myschema.view(name = 'obj')", &PostgreSqlDialect {});
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteTableRef {
    pub table_ref: TableReference,
    pub args: Option<Arc<[FunctionArg]>>,
}

impl RemoteTableRef {
    /// Get quoted_string representation for the table it is referencing, this is same as calling to_quoted_string on the inner table reference.
    pub fn to_quoted_string(&self) -> String {
        self.table_ref.to_quoted_string()
    }

    /// Create new using general purpose dialect. Prefer [`Self::parse_with_dialect`] if the dialect is known beforehand
    pub fn parse_with_default_dialect(s: &str) -> Result<Self, DataFusionError> {
        Self::parse_with_dialect(s, &GenericDialect {})
    }

    /// Create new using a specific instance of dialect.
    pub fn parse_with_dialect(s: &str, dialect: &dyn Dialect) -> Result<Self, DataFusionError> {
        let mut parser = sqlparser::parser::Parser::new(dialect).try_with_sql(s)?;
        let name = parser.parse_object_name(true)?;
        let args = if parser.consume_token(&Token::LParen) {
            parser.parse_optional_args()?
        } else {
            vec![]
        };

        let table_ref = match (name.0.first(), name.0.get(1), name.0.get(2)) {
            (
                Some(ObjectNamePart::Identifier(catalog)),
                Some(ObjectNamePart::Identifier(schema)),
                Some(ObjectNamePart::Identifier(table)),
            ) => TableReference::full(
                catalog.value.clone(),
                schema.value.clone(),
                table.value.clone(),
            ),
            (
                Some(ObjectNamePart::Identifier(schema)),
                Some(ObjectNamePart::Identifier(table)),
                None,
            ) => TableReference::partial(schema.value.clone(), table.value.clone()),
            (Some(ObjectNamePart::Identifier(table)), None, None) => {
                TableReference::bare(table.value.clone())
            }
            _ => {
                return Err(DataFusionError::NotImplemented(
                    "Unable to parse string into TableReference".to_string(),
                ))
            }
        };

        if !args.is_empty() {
            Ok(RemoteTableRef {
                table_ref,
                args: Some(args.into()),
            })
        } else {
            Ok(RemoteTableRef {
                table_ref,
                args: None,
            })
        }
    }

    pub fn table_ref(&self) -> &TableReference {
        &self.table_ref
    }

    pub fn args(&self) -> Option<&[FunctionArg]> {
        self.args.as_deref()
    }
}

impl From<TableReference> for RemoteTableRef {
    fn from(table_ref: TableReference) -> Self {
        RemoteTableRef {
            table_ref,
            args: None,
        }
    }
}

impl From<RemoteTableRef> for TableReference {
    fn from(remote_table_ref: RemoteTableRef) -> Self {
        remote_table_ref.table_ref
    }
}

impl From<&RemoteTableRef> for TableReference {
    fn from(remote_table_ref: &RemoteTableRef) -> Self {
        remote_table_ref.table_ref.clone()
    }
}

impl From<(TableReference, Vec<FunctionArg>)> for RemoteTableRef {
    fn from((table_ref, args): (TableReference, Vec<FunctionArg>)) -> Self {
        RemoteTableRef {
            table_ref,
            args: Some(args.into()),
        }
    }
}

impl TryFrom<&str> for RemoteTableRef {
    type Error = DataFusionError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse_with_default_dialect(s)
    }
}

impl TryFrom<String> for RemoteTableRef {
    type Error = DataFusionError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse_with_default_dialect(&s)
    }
}

impl TryFrom<&String> for RemoteTableRef {
    type Error = DataFusionError;
    fn try_from(s: &String) -> Result<Self, Self::Error> {
        Self::parse_with_default_dialect(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::{
        ast::{self, Expr, FunctionArgOperator, Ident, Value},
        dialect,
    };

    #[test]
    fn bare_table_reference() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("table").unwrap();
        let expected = RemoteTableRef::from(TableReference::bare("table"));
        assert_eq!(table_ref, expected);

        let table_ref = RemoteTableRef::parse_with_default_dialect("Table").unwrap();
        let expected = RemoteTableRef::from(TableReference::bare("Table"));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn bare_table_reference_with_args() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("table(1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::bare("table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);

        let table_ref = RemoteTableRef::parse_with_default_dialect("Table(1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::bare("Table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn bare_table_reference_with_args_and_whitespace() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("table (1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::bare("table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);

        let table_ref = RemoteTableRef::parse_with_default_dialect("Table (1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::bare("Table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn multi_table_reference_with_no_args() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("schema.table").unwrap();
        let expected = RemoteTableRef::from(TableReference::partial("schema", "table"));
        assert_eq!(table_ref, expected);

        let table_ref = RemoteTableRef::parse_with_default_dialect("schema.Table").unwrap();
        let expected = RemoteTableRef::from(TableReference::partial("schema", "Table"));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn multi_table_reference_with_args() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("schema.table(1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::partial("schema", "table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);

        let table_ref = RemoteTableRef::parse_with_default_dialect("schema.Table(1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::partial("schema", "Table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn multi_table_reference_with_args_and_whitespace() {
        let table_ref = RemoteTableRef::parse_with_default_dialect("schema.table (1, 2)").unwrap();
        let expected = RemoteTableRef::from((
            TableReference::partial("schema", "table"),
            vec![
                FunctionArg::Unnamed(Expr::value(Value::Number("1".to_string(), false)).into()),
                FunctionArg::Unnamed(Expr::value(Value::Number("2".to_string(), false)).into()),
            ],
        ));
        assert_eq!(table_ref, expected);
    }

    #[test]
    fn bare_reference_with_named_args() {
        let table_ref = RemoteTableRef::parse_with_dialect(
            "Table (user_id => 1, age => 2)",
            &dialect::PostgreSqlDialect {},
        )
        .unwrap();
        let expected = RemoteTableRef::from((
            TableReference::bare("Table"),
            vec![
                FunctionArg::ExprNamed {
                    name: ast::Expr::Identifier(Ident::new("user_id")),
                    arg: Expr::value(Value::Number("1".to_string(), false)).into(),
                    operator: FunctionArgOperator::RightArrow,
                },
                FunctionArg::ExprNamed {
                    name: ast::Expr::Identifier(Ident::new("age")),
                    arg: Expr::value(Value::Number("2".to_string(), false)).into(),
                    operator: FunctionArgOperator::RightArrow,
                },
            ],
        ));
        assert_eq!(table_ref, expected);
    }
}
