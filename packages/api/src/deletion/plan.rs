//! Children-first deletion plans derived from the foreign-key graph.
//!
//! A plan is a list of [`Step`]s; every `Drain`, `NullOut` and `SweepSoft`
//! step touches exactly one table, so one chunk of it fits one bounded
//! transaction. Steps are ordered by a Kahn topological sort with
//! child-before-parent constraints and table-name tie breaks, which makes the
//! order deterministic and testable.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use super::DeletionRoot;
use super::external::ExternalStep;
use super::graph::{FkAction, FkEdge, FkGraph, fk_graph};
use super::overrides::{RootOverrides, overrides_for};

const MAX_DEPTH: usize = 8;

/// The row a plan starts from: the root table and its single-column primary
/// key, the only value a job carries (`DeletionJob.rootId`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootKey {
    pub table: String,
    pub column: String,
}

/// Which rows of a table belong to the root being deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    /// `column = $root`
    Root { column: String },
    /// `column IN (SELECT parent_column FROM parent WHERE inner)`
    Via {
        column: String,
        parent: String,
        parent_column: String,
        inner: Box<Predicate>,
    },
}

impl Predicate {
    /// The predicate selecting the rows reached from the root along `path`
    /// (root → `path[0].child` → … → `path.last().child`).
    pub fn from_path(root: &RootKey, path: &[FkEdge]) -> Self {
        let (last, prefix) = path
            .split_last()
            .expect("a cascade path has at least one edge");
        if prefix.is_empty() {
            Self::from_root(root, last)
        } else {
            Self::Via {
                column: last.column.clone(),
                parent: last.parent.clone(),
                parent_column: last.parent_column.clone(),
                inner: Box::new(Self::from_path(root, prefix)),
            }
        }
    }

    /// The first hop out of the root. `$root` is the root's primary key, so an
    /// edge onto any other root column (`BitTreeCache.dependencyTreeHash ->
    /// Bit.dependencyTreeHash`) must read that column back out of the root row
    /// instead of comparing the child column to the root id.
    fn from_root(root: &RootKey, edge: &FkEdge) -> Self {
        if edge.parent_column == root.column {
            Self::Root {
                column: edge.column.clone(),
            }
        } else {
            Self::Via {
                column: edge.column.clone(),
                parent: root.table.clone(),
                parent_column: edge.parent_column.clone(),
                inner: Box::new(Self::Root {
                    column: root.column.clone(),
                }),
            }
        }
    }

    /// `edge.child.edge.column` rows whose parent is selected by `parent`.
    fn through(edge: &FkEdge, parent: Predicate) -> Self {
        Self::Via {
            column: edge.column.clone(),
            parent: edge.parent.clone(),
            parent_column: edge.parent_column.clone(),
            inner: Box::new(parent),
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Self::Root { .. } => 1,
            Self::Via { inner, .. } => 1 + inner.depth(),
        }
    }

    /// The table and column the innermost `= $root` comparison reads, given
    /// the table this predicate filters.
    pub fn root_hop<'p>(&'p self, table: &'p str) -> (&'p str, &'p str) {
        match self {
            Self::Root { column } => (table, column),
            Self::Via { parent, inner, .. } => inner.root_hop(parent),
        }
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { column } => write!(f, "\"{column}\" = $root"),
            Self::Via {
                column,
                parent,
                parent_column,
                inner,
            } => write!(
                f,
                "\"{column}\" IN (SELECT \"{parent_column}\" FROM \"{parent}\" WHERE {inner})"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Hide the root from readers before anything is removed.
    Tombstone,
    External(ExternalStep),
    /// Set `column` to NULL on rows referencing a parent that is about to go.
    NullOut {
        table: String,
        column: String,
        predicates: Vec<Predicate>,
    },
    /// Delete the rows of `table` reached from the root along each predicate.
    Drain {
        table: String,
        predicates: Vec<Predicate>,
    },
    /// Delete rows keyed by the root id without a foreign key.
    SweepSoft {
        table: String,
        column: String,
    },
    DeleteRoot,
}

impl Step {
    pub fn table(&self) -> Option<&str> {
        match self {
            Self::NullOut { table, .. }
            | Self::Drain { table, .. }
            | Self::SweepSoft { table, .. } => Some(table),
            Self::Tombstone | Self::External(_) | Self::DeleteRoot => None,
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tombstone => f.write_str("tombstone the root row"),
            Self::External(step) => write!(f, "external: {}", step.describe()),
            Self::NullOut {
                table,
                column,
                predicates,
            } => write!(
                f,
                "null out \"{table}\".\"{column}\" where {}",
                join_predicates(predicates)
            ),
            Self::Drain { table, predicates } => {
                write!(f, "drain \"{table}\" where {}", join_predicates(predicates))
            }
            Self::SweepSoft { table, column } => {
                write!(f, "sweep \"{table}\" where \"{column}\" = $root")
            }
            Self::DeleteRoot => f.write_str("delete the root row"),
        }
    }
}

fn join_predicates(predicates: &[Predicate]) -> String {
    predicates
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub root: DeletionRoot,
    pub steps: Vec<Step>,
}

impl Plan {
    /// Tables drained by this plan, in execution order.
    pub fn drained_tables(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                Step::Drain { table, .. } => Some(table.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn position(&self, step: &Step) -> Option<usize> {
        self.steps.iter().position(|s| s == step)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    UnknownTable(String),
    UnknownColumn {
        table: String,
        column: String,
    },
    /// A blocking foreign key points at a table the plan deletes, and no
    /// cascade path empties the referencing table first.
    RestrictNotCovered {
        child: String,
        column: String,
        parent: String,
    },
    UnsupportedPrimaryKey {
        table: String,
        column: String,
    },
    /// The root table has no single-column primary key, so `DeletionJob.rootId`
    /// cannot address one root row.
    CompositeRootKey {
        table: String,
    },
    /// A predicate's innermost comparison is not the root's primary key and no
    /// foreign key connects it to one, so it would select the wrong rows.
    PredicateNotRooted {
        table: String,
        column: String,
        root: String,
    },
    Cycle(Vec<String>),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTable(table) => write!(f, "table {table} is not part of the schema"),
            Self::UnknownColumn { table, column } => {
                write!(f, "column {table}.{column} is not part of the schema")
            }
            Self::RestrictNotCovered {
                child,
                column,
                parent,
            } => write!(
                f,
                "{child}.{column} blocks deleting {parent} and no cascade path empties {child} first"
            ),
            Self::UnsupportedPrimaryKey { table, column } => {
                write!(
                    f,
                    "primary key {table}.{column} has a type the deleter cannot page"
                )
            }
            Self::CompositeRootKey { table } => write!(
                f,
                "root table {table} needs a single-column primary key to be deleted by id"
            ),
            Self::PredicateNotRooted {
                table,
                column,
                root,
            } => write!(
                f,
                "{table}.{column} is compared to the {root} id but is not a foreign key onto the {root} primary key"
            ),
            Self::Cycle(tables) => write!(f, "cascade constraints form a cycle through {tables:?}"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<PlanError> for crate::error::ApiError {
    fn from(error: PlanError) -> Self {
        Self::bad_request(format!("deletion plan: {error}"))
    }
}

/// The plan for deleting one row of `root`, built from the live schema.
pub fn plan_for(root: DeletionRoot) -> Result<Plan, PlanError> {
    build_plan(fk_graph(), root, &overrides_for(root))
}

fn build_plan(
    graph: &FkGraph,
    root: DeletionRoot,
    overrides: &RootOverrides,
) -> Result<Plan, PlanError> {
    let root_table = root.table_name();
    let root_meta = graph
        .table(root_table)
        .ok_or_else(|| PlanError::UnknownTable(root_table.to_owned()))?;
    let [root_pk] = root_meta.primary_key.as_slice() else {
        return Err(PlanError::CompositeRootKey {
            table: root_table.to_owned(),
        });
    };
    if root_pk.kind.is_none() {
        return Err(PlanError::UnsupportedPrimaryKey {
            table: root_table.to_owned(),
            column: root_pk.name.clone(),
        });
    }
    let root_key = RootKey {
        table: root_table.to_owned(),
        column: root_pk.name.clone(),
    };

    let cascades = |edge: &FkEdge| {
        edge.action == FkAction::Cascade
            || overrides
                .restrict_as_cascade
                .iter()
                .any(|(child, column)| edge.child == *child && edge.column == *column)
    };

    let mut paths: BTreeMap<String, Vec<Vec<FkEdge>>> = BTreeMap::new();
    let mut queue: VecDeque<(String, Vec<FkEdge>)> = VecDeque::new();
    queue.push_back((root_table.to_owned(), Vec::new()));
    while let Some((table, path)) = queue.pop_front() {
        if path.len() >= MAX_DEPTH {
            continue;
        }
        for edge in graph.children_of(&table).filter(|edge| cascades(edge)) {
            if edge.child == root_table || path.iter().any(|seen| seen.child == edge.child) {
                continue;
            }
            let mut next = path.clone();
            next.push(edge.clone());
            paths
                .entry(edge.child.clone())
                .or_default()
                .push(next.clone());
            queue.push_back((edge.child.clone(), next));
        }
    }

    for table in paths.keys() {
        let meta = graph
            .table(table)
            .ok_or_else(|| PlanError::UnknownTable(table.clone()))?;
        if let Some(column) = meta.primary_key.iter().find(|column| column.kind.is_none()) {
            return Err(PlanError::UnsupportedPrimaryKey {
                table: table.clone(),
                column: column.name.clone(),
            });
        }
    }

    let in_set = |table: &str| table == root_table || paths.contains_key(table);
    let mut before: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut null_outs: BTreeMap<String, Vec<FkEdge>> = BTreeMap::new();
    let mut constrain = |child: &str, parent: &str| {
        if child != root_table && parent != root_table && child != parent {
            before
                .entry(child.to_owned())
                .or_default()
                .insert(parent.to_owned());
        }
    };

    for edge in graph.edges() {
        if !in_set(&edge.parent) {
            continue;
        }
        if cascades(edge) {
            if in_set(&edge.child) {
                constrain(&edge.child, &edge.parent);
            }
            continue;
        }
        match edge.action {
            FkAction::SetNull => {
                null_outs
                    .entry(edge.parent.clone())
                    .or_default()
                    .push(edge.clone());
                if in_set(&edge.child) {
                    constrain(&edge.child, &edge.parent);
                }
            }
            FkAction::Cascade => {}
            FkAction::Restrict | FkAction::NoAction | FkAction::SetDefault => {
                if in_set(&edge.child) {
                    constrain(&edge.child, &edge.parent);
                } else {
                    return Err(PlanError::RestrictNotCovered {
                        child: edge.child.clone(),
                        column: edge.column.clone(),
                        parent: edge.parent.clone(),
                    });
                }
            }
        }
    }

    let order = topological_order(paths.keys().cloned().collect(), &before)?;

    let predicates_for = |table: &str| -> Vec<Predicate> {
        paths
            .get(table)
            .map(|table_paths| {
                table_paths
                    .iter()
                    .map(|path| Predicate::from_path(&root_key, path))
                    .collect()
            })
            .unwrap_or_default()
    };
    let null_out_steps = |parent: &str| -> Vec<Step> {
        let mut edges = null_outs.get(parent).cloned().unwrap_or_default();
        edges.sort_by(|a, b| (&a.child, &a.column).cmp(&(&b.child, &b.column)));
        edges
            .into_iter()
            .map(|edge| {
                let predicates = if parent == root_table {
                    vec![Predicate::from_root(&root_key, &edge)]
                } else {
                    predicates_for(parent)
                        .into_iter()
                        .map(|parent_predicate| Predicate::through(&edge, parent_predicate))
                        .collect()
                };
                Step::NullOut {
                    table: edge.child,
                    column: edge.column,
                    predicates,
                }
            })
            .collect()
    };

    let mut steps = vec![Step::Tombstone];
    steps.extend(overrides.before_drain.iter().copied().map(Step::External));
    for table in &order {
        steps.extend(null_out_steps(table));
        steps.push(Step::Drain {
            table: table.clone(),
            predicates: predicates_for(table),
        });
    }
    steps.extend(null_out_steps(root_table));
    for sweep in &overrides.soft_sweeps {
        let meta = graph
            .table(sweep.table)
            .ok_or_else(|| PlanError::UnknownTable(sweep.table.to_owned()))?;
        if !meta.has_column(sweep.column) {
            return Err(PlanError::UnknownColumn {
                table: sweep.table.to_owned(),
                column: sweep.column.to_owned(),
            });
        }
        if let Some(column) = meta.primary_key.iter().find(|column| column.kind.is_none()) {
            return Err(PlanError::UnsupportedPrimaryKey {
                table: sweep.table.to_owned(),
                column: column.name.clone(),
            });
        }
        steps.push(Step::SweepSoft {
            table: sweep.table.to_owned(),
            column: sweep.column.to_owned(),
        });
    }
    steps.extend(overrides.after_drain.iter().copied().map(Step::External));
    steps.push(Step::DeleteRoot);

    ensure_rooted(graph, &root_key, &steps)?;

    Ok(Plan { root, steps })
}

/// Every predicate must bottom out at the root's primary key, either through
/// a foreign key onto it or by reading another root column out of the root
/// row. A bare `column = $root` on an edge that references a non-key column
/// compares unrelated values and silently matches nothing.
fn ensure_rooted(graph: &FkGraph, root: &RootKey, steps: &[Step]) -> Result<(), PlanError> {
    for step in steps {
        let (Step::Drain { table, predicates } | Step::NullOut {
            table, predicates, ..
        }) = step
        else {
            continue;
        };
        for predicate in predicates {
            let (owner, column) = predicate.root_hop(table);
            let rooted = (owner == root.table && column == root.column)
                || graph.edges().iter().any(|edge| {
                    edge.child == owner
                        && edge.column == column
                        && edge.parent == root.table
                        && edge.parent_column == root.column
                });
            if !rooted {
                return Err(PlanError::PredicateNotRooted {
                    table: owner.to_owned(),
                    column: column.to_owned(),
                    root: root.table.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Kahn's algorithm over `before` (child → parents that must come later),
/// breaking ties by table name.
fn topological_order(
    nodes: BTreeSet<String>,
    before: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, PlanError> {
    let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|node| (node.as_str(), 0)).collect();
    for parents in before.values() {
        for parent in parents {
            if let Some(degree) = indegree.get_mut(parent.as_str()) {
                *degree += 1;
            }
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(next) = ready.iter().next().copied() {
        ready.remove(next);
        order.push(next.to_owned());
        if let Some(parents) = before.get(next) {
            for parent in parents {
                if let Some(degree) = indegree.get_mut(parent.as_str()) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(parent.as_str());
                    }
                }
            }
        }
    }
    if order.len() != nodes.len() {
        let stuck = nodes
            .iter()
            .filter(|node| !order.contains(node))
            .cloned()
            .collect();
        return Err(PlanError::Cycle(stuck));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_plan() -> Plan {
        plan_for(DeletionRoot::App).expect("App plan")
    }

    fn drain_index(plan: &Plan, table: &str) -> usize {
        plan.steps
            .iter()
            .position(|step| matches!(step, Step::Drain { table: t, .. } if t == table))
            .unwrap_or_else(|| panic!("{table} is not drained by the plan"))
    }

    fn null_out_index(plan: &Plan, table: &str, column: &str) -> usize {
        plan.steps
            .iter()
            .position(|step| {
                matches!(step, Step::NullOut { table: t, column: c, .. } if t == table && c == column)
            })
            .unwrap_or_else(|| panic!("{table}.{column} is not nulled out by the plan"))
    }

    fn assert_before(plan: &Plan, first: &str, second: &str) {
        assert!(
            drain_index(plan, first) < drain_index(plan, second),
            "{first} must drain before {second}:\n{}",
            render(plan)
        );
    }

    fn render(plan: &Plan) -> String {
        plan.steps
            .iter()
            .enumerate()
            .map(|(i, step)| format!("{i:>3}: {step}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_root_but_role_has_a_plan() {
        for root in DeletionRoot::ALL {
            let plan = plan_for(root);
            if root == DeletionRoot::Role {
                assert_eq!(
                    plan,
                    Err(PlanError::RestrictNotCovered {
                        child: "Membership".into(),
                        column: "roleId".into(),
                        parent: "Role".into(),
                    })
                );
            } else {
                let plan = plan.unwrap_or_else(|e| panic!("{root:?}: {e}"));
                assert_eq!(plan.steps.first(), Some(&Step::Tombstone));
                assert_eq!(plan.steps.last(), Some(&Step::DeleteRoot));
            }
        }
    }

    /// The root hop is the one place `$root` is compared to a column, so it
    /// has to name the root's primary key — directly on a foreign key onto it,
    /// or through a subquery over the root row for every other column.
    #[test]
    fn every_root_hop_reaches_the_root_primary_key() {
        for root in DeletionRoot::ALL {
            let Ok(plan) = plan_for(root) else { continue };
            let root_table = root.table_name();
            let root_pk = fk_graph().table(root_table).expect(root_table).primary_key[0]
                .name
                .clone();
            for step in &plan.steps {
                let (Step::Drain { table, predicates } | Step::NullOut {
                    table, predicates, ..
                }) = step
                else {
                    continue;
                };
                for predicate in predicates {
                    let (owner, column) = predicate.root_hop(table);
                    if owner == root_table && column == root_pk {
                        continue;
                    }
                    let edge = fk_graph()
                        .edges()
                        .iter()
                        .find(|edge| edge.child == owner && edge.column == column)
                        .unwrap_or_else(|| panic!("{root:?}: {owner}.{column} is not a foreign key"));
                    assert_eq!(edge.parent, root_table, "{root:?}: {}", edge.describe());
                    assert_eq!(edge.parent_column, root_pk, "{root:?}: {}", edge.describe());
                }
            }
        }
    }

    /// `BitTreeCache.dependencyTreeHash` references `Bit.dependencyTreeHash`,
    /// not `Bit.id`, so the hash has to be read out of the root row.
    #[test]
    fn bit_plan_reads_the_tree_hash_out_of_the_root_row() {
        let plan = plan_for(DeletionRoot::Bit).unwrap();
        let through_root = Predicate::Via {
            column: "dependencyTreeHash".into(),
            parent: "Bit".into(),
            parent_column: "dependencyTreeHash".into(),
            inner: Box::new(Predicate::Root {
                column: "id".into(),
            }),
        };
        let Step::Drain { predicates, .. } = &plan.steps[drain_index(&plan, "BitTreeCache")] else {
            unreachable!()
        };
        assert_eq!(predicates, &[through_root.clone()], "{}", render(&plan));
        assert_eq!(
            through_root.to_string(),
            r#""dependencyTreeHash" IN (SELECT "dependencyTreeHash" FROM "Bit" WHERE "id" = $root)"#
        );

        let Step::Drain { predicates, .. } = &plan.steps[drain_index(&plan, "BitCache")] else {
            unreachable!()
        };
        assert!(
            predicates.contains(&Predicate::Root {
                column: "bitId".into()
            }),
            "{}",
            render(&plan)
        );
        assert!(
            predicates.contains(&Predicate::Via {
                column: "dependencyTreeHash".into(),
                parent: "BitTreeCache".into(),
                parent_column: "dependencyTreeHash".into(),
                inner: Box::new(through_root),
            }),
            "{}",
            render(&plan)
        );
        assert!(drain_index(&plan, "BitCache") < drain_index(&plan, "BitTreeCache"));
    }

    /// The guard that keeps H1 from recurring: a bare `= $root` on a column
    /// that is not a foreign key onto the root primary key is rejected.
    #[test]
    fn a_root_hop_off_the_primary_key_is_a_plan_error() {
        let root = RootKey {
            table: "Bit".into(),
            column: "id".into(),
        };
        let steps = vec![Step::Drain {
            table: "BitTreeCache".into(),
            predicates: vec![Predicate::Root {
                column: "dependencyTreeHash".into(),
            }],
        }];
        assert_eq!(
            ensure_rooted(fk_graph(), &root, &steps),
            Err(PlanError::PredicateNotRooted {
                table: "BitTreeCache".into(),
                column: "dependencyTreeHash".into(),
                root: "Bit".into(),
            })
        );
        let app_root = RootKey {
            table: "App".into(),
            column: "id".into(),
        };
        assert!(ensure_rooted(fk_graph(), &app_root, &app_plan().steps).is_ok());
        assert!(ensure_rooted(fk_graph(), &root, &plan_for(DeletionRoot::Bit).unwrap().steps).is_ok());
    }

    #[test]
    fn plans_are_deterministic() {
        assert_eq!(app_plan(), app_plan());
        assert_eq!(
            plan_for(DeletionRoot::Course).unwrap(),
            plan_for(DeletionRoot::Course).unwrap()
        );
    }

    #[test]
    fn app_plan_respects_every_foreign_key() {
        let plan = app_plan();
        let drained: Vec<&str> = plan.drained_tables();
        assert!(drained.iter().all(|t| *t != "App"));
        let unique: BTreeSet<&&str> = drained.iter().collect();
        assert_eq!(unique.len(), drained.len(), "each table drains once");
        for edge in fk_graph().edges() {
            let both =
                drained.contains(&edge.child.as_str()) && drained.contains(&edge.parent.as_str());
            if both && edge.child != edge.parent {
                assert_before(&plan, &edge.child, &edge.parent);
            }
        }
    }

    #[test]
    fn app_plan_named_order() {
        let plan = app_plan();
        for (child, parent) in [
            ("ExecutionEvent", "ExecutionRun"),
            ("ExecutionRunCallerApp", "ExecutionRun"),
            ("RegressionCaseResult", "RegressionSuiteRun"),
            ("PublicationLog", "PublicationRequest"),
            ("Meta", "Template"),
            ("Comment", "Template"),
            ("Feedback", "Template"),
            ("Meta", "Widget"),
            ("AppGroupMember", "AppGroup"),
            ("Meta", "AppGroup"),
            ("PublicationRequest", "AppGroup"),
            ("Invitation", "Membership"),
            ("TechnicalUser", "Membership"),
            ("Membership", "Role"),
            ("EventRemoteRegistration", "EventRemoteAuth"),
            ("EventRemoteAuth", "Event"),
            ("EventSetup", "Event"),
            ("EventAlias", "Event"),
            ("EventSink", "Event"),
            ("ExecutionRun", "TechnicalUser"),
            ("LLMUsageTracking", "TechnicalUser"),
        ] {
            assert_before(&plan, child, parent);
        }
    }

    #[test]
    fn app_plan_nulls_the_role_back_edge_before_roles_go() {
        let plan = app_plan();
        let role = drain_index(&plan, "Role");
        for column in ["defaultRoleId", "ownerRoleId"] {
            let index = null_out_index(&plan, "App", column);
            assert!(index < role, "{}", render(&plan));
            let Step::NullOut { predicates, .. } = &plan.steps[index] else {
                unreachable!()
            };
            assert_eq!(
                predicates,
                &[Predicate::Via {
                    column: column.into(),
                    parent: "Role".into(),
                    parent_column: "id".into(),
                    inner: Box::new(Predicate::Root {
                        column: "appId".into()
                    }),
                }]
            );
        }
        assert!(null_out_index(&plan, "TechnicalUser", "roleId") < role);
        assert!(null_out_index(&plan, "AppConnection", "roleId") < role);
    }

    #[test]
    fn app_plan_nulls_after_the_child_drained() {
        let plan = app_plan();
        let registration = drain_index(&plan, "EventRemoteRegistration");
        let auth_null = null_out_index(&plan, "EventRemoteRegistration", "authId");
        let auth = drain_index(&plan, "EventRemoteAuth");
        assert!(
            registration < auth_null && auth_null < auth,
            "{}",
            render(&plan)
        );

        let runs = drain_index(&plan, "ExecutionRun");
        let run_null = null_out_index(&plan, "ExecutionRun", "technicalUserId");
        let technical_user = drain_index(&plan, "TechnicalUser");
        assert!(
            runs < run_null && run_null < technical_user,
            "{}",
            render(&plan)
        );
    }

    #[test]
    fn app_plan_runs_external_cleanup_in_the_right_places() {
        let plan = app_plan();
        let schedules = plan
            .position(&Step::External(ExternalStep::AppSinkSchedules))
            .unwrap();
        assert_eq!(schedules, 1);
        assert!(schedules < drain_index(&plan, "EventSink"));
        let storage = plan
            .position(&Step::External(ExternalStep::AppStoragePrefixes))
            .unwrap();
        let cache = plan
            .position(&Step::External(ExternalStep::AppCacheBackend))
            .unwrap();
        let last_drain = plan
            .steps
            .iter()
            .rposition(|step| matches!(step, Step::Drain { .. } | Step::NullOut { .. }))
            .unwrap();
        let sweeps: Vec<usize> = plan
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| matches!(step, Step::SweepSoft { .. }))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(sweeps.len(), 5);
        assert!(sweeps.iter().all(|i| *i > last_drain && *i < storage));
        assert!(storage < cache && cache + 1 == plan.steps.len() - 1);
        assert_eq!(plan.steps.last(), Some(&Step::DeleteRoot));
    }

    #[test]
    fn app_plan_paths_are_shallow_and_start_at_the_root() {
        let plan = app_plan();
        let root_columns: BTreeSet<&str> = fk_graph()
            .children_of("App")
            .map(|edge| edge.column.as_str())
            .collect();
        for step in &plan.steps {
            let (Step::Drain { predicates, .. } | Step::NullOut { predicates, .. }) = step else {
                continue;
            };
            assert!(!predicates.is_empty(), "{step}");
            for predicate in predicates {
                assert!(predicate.depth() <= 5, "{step}");
                let mut inner = predicate;
                while let Predicate::Via { inner: next, .. } = inner {
                    inner = next;
                }
                let Predicate::Root { column } = inner else {
                    unreachable!()
                };
                assert!(root_columns.contains(column.as_str()), "{step}");
            }
        }
    }

    #[test]
    fn app_children_with_multiple_paths_get_one_predicate_each() {
        let plan = app_plan();
        let Step::Drain { predicates, .. } = &plan.steps[drain_index(&plan, "Meta")] else {
            unreachable!()
        };
        let described: Vec<String> = predicates.iter().map(ToString::to_string).collect();
        assert!(
            described.contains(&"\"appId\" = $root".to_string()),
            "{described:?}"
        );
        assert!(
            described.contains(
                &"\"templateId\" IN (SELECT \"id\" FROM \"Template\" WHERE \"appId\" = $root)"
                    .to_string()
            ),
            "{described:?}"
        );
    }

    #[test]
    fn user_plan_drains_wasm_invitations_through_the_override() {
        let plan = plan_for(DeletionRoot::User).unwrap();
        let invitations = drain_index(&plan, "WasmPackageInvitation");
        assert!(invitations < plan.steps.len() - 1);
        let Step::Drain { predicates, .. } = &plan.steps[invitations] else {
            unreachable!()
        };
        assert_eq!(predicates.len(), 2);
    }

    #[test]
    fn course_plan_is_leaf_first_to_depth_four() {
        let plan = plan_for(DeletionRoot::Course).unwrap();
        for (child, parent) in [
            ("UserChallengeAttempt", "Challenge"),
            ("WeeklyChallenge", "Challenge"),
            ("Challenge", "Lesson"),
            ("LessonAppRef", "Lesson"),
            ("UserLessonProgress", "Lesson"),
            ("Lesson", "CourseModule"),
        ] {
            assert_before(&plan, child, parent);
        }
        assert_eq!(
            plan.steps.get(1),
            Some(&Step::External(ExternalStep::CourseMedia))
        );
    }

    #[test]
    fn template_plan_drains_its_user_generated_children() {
        let plan = plan_for(DeletionRoot::Template).unwrap();
        assert_eq!(
            plan.drained_tables(),
            vec!["Comment", "Feedback", "Meta"],
            "{}",
            render(&plan)
        );
        for table in plan.drained_tables() {
            let Step::Drain { predicates, .. } = &plan.steps[drain_index(&plan, table)] else {
                unreachable!()
            };
            assert_eq!(
                predicates,
                &[Predicate::Root {
                    column: "templateId".into()
                }]
            );
        }
    }

    #[test]
    fn soft_references_marked_keep_are_not_foreign_keys() {
        for root in DeletionRoot::ALL {
            for reference in overrides_for(root).keep {
                let is_fk = fk_graph().edges().iter().any(|edge| {
                    edge.child == reference.table
                        && edge.column == reference.column
                        && edge.parent == root.table_name()
                });
                assert!(
                    !is_fk,
                    "{}.{} is a real foreign key",
                    reference.table, reference.column
                );
                let meta = fk_graph().table(reference.table).expect(reference.table);
                assert!(
                    meta.has_column(reference.column),
                    "{}.{}",
                    reference.table,
                    reference.column
                );
            }
        }
    }

    #[test]
    fn cycles_are_reported() {
        let nodes: BTreeSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
        let mut before = BTreeMap::new();
        before.insert("A".to_string(), BTreeSet::from(["B".to_string()]));
        before.insert("B".to_string(), BTreeSet::from(["A".to_string()]));
        assert_eq!(
            topological_order(nodes, &before),
            Err(PlanError::Cycle(vec!["A".into(), "B".into()]))
        );
    }
}
