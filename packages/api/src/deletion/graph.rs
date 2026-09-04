//! Foreign-key metadata lifted from the generated sea-orm entities.
//!
//! Every `belongs_to` relation of every entity in [`crate::entity::prelude`]
//! becomes one [`FkEdge`]. The set is pinned by `edges.snapshot.txt`: an
//! entity regeneration that adds a table, drops an edge or changes a
//! referential action fails `snapshot_edges` instead of silently changing what
//! the deleter drains.

use sea_orm::sea_query::{ColumnType, DynIden, ForeignKeyAction, TableRef};
use sea_orm::{ColumnTrait, EntityTrait, IdenStatic, Iterable, PrimaryKeyToColumn, RelationTrait};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::entity::prelude;

/// The referential action declared on a foreign key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FkAction {
    Cascade,
    SetNull,
    Restrict,
    NoAction,
    SetDefault,
}

impl FkAction {
    fn from_sea_query(action: Option<ForeignKeyAction>) -> Self {
        match action {
            Some(ForeignKeyAction::Cascade) => Self::Cascade,
            Some(ForeignKeyAction::SetNull) => Self::SetNull,
            Some(ForeignKeyAction::Restrict) => Self::Restrict,
            Some(ForeignKeyAction::SetDefault) => Self::SetDefault,
            Some(ForeignKeyAction::NoAction) | None => Self::NoAction,
            Some(_) => Self::NoAction,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cascade => "Cascade",
            Self::SetNull => "SetNull",
            Self::Restrict => "Restrict",
            Self::NoAction => "NoAction",
            Self::SetDefault => "SetDefault",
        }
    }

    /// Whether deleting the parent blocks while any child row still exists.
    pub fn blocks_parent_delete(self) -> bool {
        matches!(self, Self::Restrict | Self::NoAction | Self::SetDefault)
    }
}

/// How a primary-key column is read back from a page select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PkKind {
    Text,
    Int,
    BigInt,
}

impl PkKind {
    fn from_column_type(column_type: &ColumnType) -> Option<Self> {
        match column_type {
            ColumnType::Text | ColumnType::String(_) | ColumnType::Char(_) => Some(Self::Text),
            ColumnType::TinyInteger | ColumnType::SmallInteger | ColumnType::Integer => {
                Some(Self::Int)
            }
            ColumnType::BigInteger => Some(Self::BigInt),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PkColumn {
    pub name: String,
    /// `None` when the column type cannot be paged by the deleter.
    pub kind: Option<PkKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableMeta {
    pub name: String,
    pub primary_key: Vec<PkColumn>,
    pub columns: Vec<String>,
}

impl TableMeta {
    pub fn has_column(&self, column: &str) -> bool {
        self.columns.iter().any(|name| name == column)
    }
}

/// One foreign key: `child.column` references `parent.parent_column`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FkEdge {
    pub child: String,
    pub column: String,
    pub parent: String,
    pub parent_column: String,
    pub action: FkAction,
}

impl FkEdge {
    pub fn describe(&self) -> String {
        format!(
            "{}.{} -> {}.{} [{}]",
            self.child,
            self.column,
            self.parent,
            self.parent_column,
            self.action.as_str()
        )
    }
}

#[derive(Debug, Default)]
pub struct FkGraph {
    tables: BTreeMap<String, TableMeta>,
    edges: Vec<FkEdge>,
    composite_keys: Vec<String>,
}

impl FkGraph {
    fn register<E: EntityTrait>(&mut self) {
        let entity = E::default();
        let name = entity.table_name().to_owned();
        let primary_key = <E::PrimaryKey as Iterable>::iter()
            .map(|key| {
                let column = key.into_column();
                PkColumn {
                    name: column.as_str().to_owned(),
                    kind: PkKind::from_column_type(column.def().get_column_type()),
                }
            })
            .collect();
        let columns = <E::Column as Iterable>::iter()
            .map(|column| column.as_str().to_owned())
            .collect();

        for relation in <E::Relation as Iterable>::iter() {
            let def = relation.def();
            // `is_owner` marks the `has_many` / `has_one` side, whose def is the
            // reversed relation and carries no referential action. The foreign
            // key lives on the `belongs_to` side.
            if def.is_owner {
                continue;
            }
            let Some(parent) = table_ref_name(&def.to_tbl) else {
                continue;
            };
            let columns: Vec<String> = def.from_col.iter().map(iden_name).collect();
            let parent_columns: Vec<String> = def.to_col.iter().map(iden_name).collect();
            match (columns.as_slice(), parent_columns.as_slice()) {
                ([column], [parent_column]) => self.edges.push(FkEdge {
                    child: name.clone(),
                    column: column.clone(),
                    parent,
                    parent_column: parent_column.clone(),
                    action: FkAction::from_sea_query(def.on_delete),
                }),
                _ => self
                    .composite_keys
                    .push(format!("{name}({}) -> {parent}", columns.join(","))),
            }
        }

        self.tables.insert(
            name.clone(),
            TableMeta {
                name,
                primary_key,
                columns,
            },
        );
    }

    pub fn table(&self, name: &str) -> Option<&TableMeta> {
        self.tables.get(name)
    }

    pub fn tables(&self) -> impl Iterator<Item = &TableMeta> {
        self.tables.values()
    }

    pub fn edges(&self) -> &[FkEdge] {
        &self.edges
    }

    /// Foreign keys pointing at `parent`.
    pub fn children_of<'a>(&'a self, parent: &'a str) -> impl Iterator<Item = &'a FkEdge> + 'a {
        self.edges.iter().filter(move |edge| edge.parent == parent)
    }

    /// Foreign keys whose child cannot be paged by primary key.
    pub fn composite_foreign_keys(&self) -> &[String] {
        &self.composite_keys
    }
}

fn iden_name(iden: &DynIden) -> String {
    iden.inner().into_owned()
}

fn table_ref_name(table: &TableRef) -> Option<String> {
    match table {
        TableRef::Table(name, _) => Some(iden_name(&name.1)),
        _ => None,
    }
}

macro_rules! register_entities {
    ($graph:expr, [$($entity:ident),* $(,)?]) => {
        $( $graph.register::<prelude::$entity>(); )*
    };
}

fn build() -> FkGraph {
    let mut graph = FkGraph::default();
    register_entities!(
        graph,
        [
            AiActAssessment,
            AiActModelObservation,
            AiActModelRegistry,
            App,
            AppAnalyticsDaily,
            AppBoardScore,
            AppCacheEntry,
            AppConnection,
            AppDiscount,
            AppGroup,
            AppGroupMember,
            AppPackage,
            AppProcessNote,
            AppPurchase,
            AppSalesDaily,
            AppUsageLimit,
            AuditEntry,
            Bit,
            BitCache,
            BitTreeCache,
            BoardSync,
            Certificate,
            Challenge,
            Channel,
            Comment,
            Course,
            CourseAppLink,
            CourseAsset,
            CourseModule,
            DeletionJob,
            EmbeddingUsageTracking,
            ErrorReport,
            Event,
            EventAlias,
            EventRemoteAuth,
            EventRemoteRegistration,
            EventSetup,
            EventSink,
            ExecutionEvent,
            ExecutionRun,
            ExecutionRunCallerApp,
            ExecutionUsageTracking,
            Feedback,
            FlowScriptApplyFailure,
            ForkJob,
            Invitation,
            InviteLink,
            JoinQueue,
            LandingPage,
            LeaderboardOptIn,
            LearningPath,
            LearningPathCourse,
            Lesson,
            LessonAppRef,
            LlmModel,
            LlmUsageTracking,
            Membership,
            Meta,
            MutationLock,
            Node,
            Notification,
            Page,
            Pat,
            Profile,
            PublicationLog,
            PublicationRequest,
            PushNotificationTarget,
            RegressionCaseResult,
            RegressionSuite,
            RegressionSuiteRun,
            Role,
            SinkToken,
            SolutionLog,
            SolutionRequest,
            StripeEvent,
            TechnicalUser,
            TelemetryAlertEvent,
            TelemetryAlertRule,
            TelemetryDashboard,
            TelemetryDimensionDaily,
            TelemetryErrorEvent,
            TelemetryEvent,
            TelemetryEventDaily,
            TelemetryFlowpilotDaily,
            TelemetryFlowpilotFailureDaily,
            TelemetryInstallDaily,
            TelemetryIssue,
            TelemetryLlmCall,
            TelemetryLlmDaily,
            TelemetryPerfDaily,
            TelemetryPerfMetric,
            TelemetryRelease,
            TelemetrySavedQuery,
            TelemetrySession,
            TelemetrySessionDaily,
            TelemetrySourceMap,
            TelemetrySpan,
            Template,
            TemplateProfile,
            Transaction,
            UsageAlert,
            UsageInvocation,
            UsageLimitAuditLog,
            User,
            UserBit,
            UserChallengeAttempt,
            UserCourseEnrollment,
            UserLessonProgress,
            WasmPackage,
            WasmPackageAuthor,
            WasmPackageInvitation,
            WasmPackageJoinQueue,
            WasmPackagePurchase,
            WasmPackageReview,
            WasmPackageUser,
            WasmPackageVersion,
            WeeklyChallenge,
            Widget,
        ]
    );
    graph.edges.sort_by(|a, b| {
        (&a.child, &a.column, &a.parent, &a.parent_column).cmp(&(
            &b.child,
            &b.column,
            &b.parent,
            &b.parent_column,
        ))
    });
    graph
}

/// The foreign-key graph of the whole schema, built once per process.
pub fn fk_graph() -> &'static FkGraph {
    static GRAPH: OnceLock<FkGraph> = OnceLock::new();
    GRAPH.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = include_str!("edges.snapshot.txt");
    const SNAPSHOT_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/deletion/edges.snapshot.txt"
    );

    #[test]
    fn snapshot_edges() {
        let actual: Vec<String> = fk_graph().edges().iter().map(FkEdge::describe).collect();
        if std::env::var_os("DELETION_SNAPSHOT_WRITE").is_some() {
            std::fs::write(SNAPSHOT_PATH, format!("{}\n", actual.join("\n"))).unwrap();
        }
        let expected: Vec<&str> = SNAPSHOT
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let actual: Vec<&str> = actual.iter().map(String::as_str).collect();
        let missing: Vec<&&str> = expected.iter().filter(|e| !actual.contains(e)).collect();
        let added: Vec<&&str> = actual.iter().filter(|a| !expected.contains(a)).collect();
        assert!(
            missing.is_empty() && added.is_empty(),
            "foreign keys drifted from edges.snapshot.txt\nmissing: {missing:#?}\nadded: {added:#?}\nRegenerate with DELETION_SNAPSHOT_WRITE=1 once the deletion plans have been reviewed."
        );
        assert_eq!(actual, expected, "edge order must be deterministic");
        assert!(
            fk_graph().composite_foreign_keys().is_empty(),
            "composite foreign keys are not pageable: {:?}",
            fk_graph().composite_foreign_keys()
        );
    }

    #[test]
    fn every_edge_targets_a_known_column() {
        let graph = fk_graph();
        for edge in graph.edges() {
            let child = graph.table(&edge.child).expect(&edge.child);
            let parent = graph.table(&edge.parent).expect(&edge.parent);
            assert!(child.has_column(&edge.column), "{}", edge.describe());
            assert!(
                parent.has_column(&edge.parent_column),
                "{}",
                edge.describe()
            );
        }
    }

    #[test]
    fn every_primary_key_is_pageable() {
        for table in fk_graph().tables() {
            assert!(
                !table.primary_key.is_empty(),
                "{} has no primary key",
                table.name
            );
            for column in &table.primary_key {
                assert!(
                    column.kind.is_some(),
                    "{}.{} has a primary key type the deleter cannot page",
                    table.name,
                    column.name
                );
            }
        }
    }
}
