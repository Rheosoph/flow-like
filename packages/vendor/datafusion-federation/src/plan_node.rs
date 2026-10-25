use core::fmt;
use std::{
    fmt::Debug,
    hash::{Hash, Hasher},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    common::DFSchemaRef,
    error::{DataFusionError, Result},
    execution::context::{QueryPlanner, SessionState},
    logical_expr::{Expr, LogicalPlan, UserDefinedLogicalNode, UserDefinedLogicalNodeCore},
    physical_plan::ExecutionPlan,
    physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner},
};

pub struct FederatedPlanNode {
    pub plan: LogicalPlan,
    pub planner: Arc<dyn FederationPlanner>,
}

impl FederatedPlanNode {
    pub fn new(plan: LogicalPlan, planner: Arc<dyn FederationPlanner>) -> Self {
        Self { plan, planner }
    }

    pub fn plan(&self) -> &LogicalPlan {
        &self.plan
    }
}

impl Debug for FederatedPlanNode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        UserDefinedLogicalNodeCore::fmt_for_explain(self, f)
    }
}

impl UserDefinedLogicalNodeCore for FederatedPlanNode {
    fn name(&self) -> &str {
        "Federated"
    }

    fn inputs(&self) -> Vec<&LogicalPlan> {
        Vec::new()
    }

    fn schema(&self) -> &DFSchemaRef {
        self.plan.schema()
    }

    fn expressions(&self) -> Vec<Expr> {
        Vec::new()
    }

    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Federated\n {}", self.plan)
    }

    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        if !inputs.is_empty() {
            return Err(DataFusionError::Plan("input size inconsistent".into()));
        }
        if !exprs.is_empty() {
            return Err(DataFusionError::Plan("expression size inconsistent".into()));
        }

        Ok(Self {
            plan: self.plan.clone(),
            planner: self.planner.clone(),
        })
    }
}

#[derive(Default, Debug)]
pub struct FederatedQueryPlanner {}

impl FederatedQueryPlanner {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl QueryPlanner for FederatedQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session_state: &SessionState,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        // Get provider here?

        let physical_planner =
            DefaultPhysicalPlanner::with_extension_planners(vec![
                Arc::new(FederatedPlanner::new()),
            ]);
        physical_planner
            .create_physical_plan(logical_plan, session_state)
            .await
    }
}

#[async_trait]
pub trait FederationPlanner: Send + Sync {
    async fn plan_federation(
        &self,
        node: &FederatedPlanNode,
        session_state: &SessionState,
    ) -> Result<Arc<dyn ExecutionPlan>>;
}

impl std::fmt::Debug for dyn FederationPlanner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "FederationPlanner")
    }
}

impl PartialEq<FederatedPlanNode> for FederatedPlanNode {
    /// Comparing name, args and return_type
    fn eq(&self, other: &FederatedPlanNode) -> bool {
        self.plan == other.plan
    }
}

impl PartialOrd<FederatedPlanNode> for FederatedPlanNode {
    fn partial_cmp(&self, other: &FederatedPlanNode) -> Option<std::cmp::Ordering> {
        self.plan.partial_cmp(&other.plan)
    }
}

impl Eq for FederatedPlanNode {}

impl Hash for FederatedPlanNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.plan.hash(state);
    }
}

#[derive(Default)]
pub struct FederatedPlanner {}

impl FederatedPlanner {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExtensionPlanner for FederatedPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        logical_inputs: &[&LogicalPlan],
        physical_inputs: &[Arc<dyn ExecutionPlan>],
        session_state: &SessionState,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let dc_node = node.as_any().downcast_ref::<FederatedPlanNode>();
        if let Some(fed_node) = dc_node {
            if !logical_inputs.is_empty() || !physical_inputs.is_empty() {
                return Err(DataFusionError::Plan(
                    "Inconsistent number of inputs".into(),
                ));
            }

            let fed_planner = Arc::clone(&fed_node.planner);
            let exec_plan = fed_planner.plan_federation(fed_node, session_state).await?;
            return Ok(Some(exec_plan));
        }
        Ok(None)
    }
}
