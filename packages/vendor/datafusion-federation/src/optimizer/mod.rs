mod scan_result;

use std::sync::Arc;

use datafusion::{
    common::not_impl_err,
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    datasource::source_as_provider,
    error::Result,
    logical_expr::{Expr, Extension, LogicalPlan, Projection, TableScan, TableSource},
    optimizer::optimizer::{Optimizer, OptimizerConfig, OptimizerRule},
};

use crate::{
    FederatedTableProviderAdaptor, FederatedTableSource, FederationProvider, FederationProviderRef,
};

use scan_result::ScanResult;

/// An optimizer rule to identifying sub-plans to federate
///
/// The optimizer logic walks over the plan, look for the largest subtrees that only have
/// TableScans from the same [`FederationProvider`]. There 'largest sub-trees' are passed to their
/// respective [`FederationProvider::optimizer`].
#[derive(Default, Debug)]
pub struct FederationOptimizerRule {}

impl OptimizerRule for FederationOptimizerRule {
    /// Try to rewrite `plan` to an optimized form, returning `Transformed::yes`
    /// if the plan was rewritten and `Transformed::no` if it was not.
    ///
    /// Note: this function is only called if [`Self::supports_rewrite`] returns
    /// true. Otherwise the Optimizer calls  [`Self::try_optimize`]
    fn rewrite(
        &self,
        plan: LogicalPlan,
        config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        match self.optimize_plan_recursively(&plan, true, config)? {
            (Some(optimized_plan), _) => Ok(Transformed::yes(optimized_plan)),
            (None, _) => Ok(Transformed::no(plan)),
        }
    }

    /// Does this rule support rewriting owned plans (rather than by reference)?
    fn supports_rewrite(&self) -> bool {
        true
    }

    /// A human readable name for this optimizer rule
    fn name(&self) -> &str {
        "federation_optimizer_rule"
    }
}

impl FederationOptimizerRule {
    /// Creates a new [`FederationOptimizerRule`]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans a plan to see if it belongs to a single [`FederationProvider`].
    fn scan_plan_recursively(&self, plan: &LogicalPlan) -> Result<ScanResult> {
        let mut sole_provider: ScanResult = ScanResult::None;

        plan.apply(&mut |p: &LogicalPlan| -> Result<TreeNodeRecursion> {
            let exprs_provider = self.scan_plan_exprs(p)?;
            sole_provider.merge(exprs_provider);

            if sole_provider.is_ambiguous() {
                return Ok(TreeNodeRecursion::Stop);
            }

            let sub_provider = get_leaf_provider(p)?;
            sole_provider.add(sub_provider);

            Ok(sole_provider.check_recursion())
        })?;

        Ok(sole_provider)
    }

    /// Scans a plan's expressions to see if it belongs to a single [`FederationProvider`].
    fn scan_plan_exprs(&self, plan: &LogicalPlan) -> Result<ScanResult> {
        let mut sole_provider: ScanResult = ScanResult::None;

        let exprs = plan.expressions();
        for expr in &exprs {
            let expr_result = self.scan_expr_recursively(expr)?;
            sole_provider.merge(expr_result);

            if sole_provider.is_ambiguous() {
                return Ok(sole_provider);
            }
        }

        Ok(sole_provider)
    }

    /// scans an expression to see if it belongs to a single [`FederationProvider`]
    fn scan_expr_recursively(&self, expr: &Expr) -> Result<ScanResult> {
        let mut sole_provider: ScanResult = ScanResult::None;

        expr.apply(&mut |e: &Expr| -> Result<TreeNodeRecursion> {
            // TODO: Support other types of sub-queries
            match e {
                Expr::ScalarSubquery(ref subquery) => {
                    let plan_result = self.scan_plan_recursively(&subquery.subquery)?;

                    sole_provider.merge(plan_result);
                    Ok(sole_provider.check_recursion())
                }
                Expr::InSubquery(_) => not_impl_err!("InSubquery"),
                Expr::OuterReferenceColumn(..) => {
                    // Subqueries that reference outer columns are not supported
                    // for now. We handle this here as ambiguity to force
                    // federation lower in the plan tree.
                    sole_provider = ScanResult::Ambiguous;
                    Ok(TreeNodeRecursion::Stop)
                }
                _ => Ok(TreeNodeRecursion::Continue),
            }
        })?;

        Ok(sole_provider)
    }

    /// Recursively finds the largest sub-plans that can be federated
    /// to a single FederationProvider.
    ///
    /// Returns a plan if a sub-tree was federated, otherwise None.
    ///
    /// Returns a ScanResult of all FederationProviders in the subtree.
    fn optimize_plan_recursively(
        &self,
        plan: &LogicalPlan,
        is_root: bool,
        _config: &dyn OptimizerConfig,
    ) -> Result<(Option<LogicalPlan>, ScanResult)> {
        let mut sole_provider: ScanResult = ScanResult::None;

        if let LogicalPlan::Extension(Extension { ref node }) = plan {
            if node.name() == "Federated" {
                // Avoid attempting double federation
                return Ok((None, ScanResult::Ambiguous));
            }
        }

        // Check if this plan node is a leaf that determines the FederationProvider
        let leaf_provider = get_leaf_provider(plan)?;

        // Check if the expressions contain, a potentially different, FederationProvider
        let exprs_result = self.scan_plan_exprs(plan)?;
        let optimize_expressions = exprs_result.is_some();

        // Return early if this is a leaf and there is no ambiguity with the expressions.
        if leaf_provider.is_some() && (exprs_result.is_none() || exprs_result == leaf_provider) {
            return Ok((None, leaf_provider.into()));
        }
        // Aggregate leaf & expression providers
        sole_provider.add(leaf_provider);
        sole_provider.merge(exprs_result);

        let inputs = plan.inputs();
        // Return early if there are no sources.
        if inputs.is_empty() && sole_provider.is_none() {
            return Ok((None, ScanResult::None));
        }

        // Recursively optimize inputs
        let input_results = inputs
            .iter()
            .map(|i| self.optimize_plan_recursively(i, false, _config))
            .collect::<Result<Vec<_>>>()?;

        // Aggregate the input providers
        input_results.iter().for_each(|(_, scan_result)| {
            sole_provider.merge(scan_result.clone());
        });

        if sole_provider.is_none() {
            // No providers found
            // TODO: Is/should this be reachable?
            return Ok((None, ScanResult::None));
        }

        // If all sources are federated to the same provider
        if let ScanResult::Distinct(provider) = sole_provider {
            if !is_root {
                // The largest sub-plan is higher up.
                return Ok((None, ScanResult::Distinct(provider)));
            }

            // Analyze plans (EXPLAIN ANALYZE) cannot be converted to SQL by
            // the Unparser, so they must not be federated as a whole. Only the
            // inner query should be federated; DataFusion's AnalyzeExec will
            // handle executing it and collecting metrics.
            if matches!(plan, LogicalPlan::Analyze(_)) {
                // Fall through to federate children instead.
            } else {
                let Some(optimizer) = provider.optimizer() else {
                    // No optimizer provided
                    return Ok((None, ScanResult::None));
                };

                // If this is the root plan node; federate the entire plan
                let optimized = optimizer.optimize(plan.clone(), _config, |_, _| {})?;
                return Ok((Some(optimized), ScanResult::None));
            }
        }

        // The plan is ambiguous; any input that is not yet optimized and has a
        // sole provider represents a largest sub-plan and should be federated.
        //
        // We loop over the input optimization results, federate where needed and
        // return a complete list of new inputs for the optimized plan.
        let new_inputs = input_results
            .into_iter()
            .enumerate()
            .map(|(i, (input_plan, input_result))| {
                if let Some(federated_plan) = input_plan {
                    // Already federated deeper in the plan tree
                    return Ok(federated_plan);
                }

                let original_input = (*inputs.get(i).unwrap()).clone();
                if input_result.is_ambiguous() {
                    // Can happen if the input is already federated, so use
                    // the original input.
                    return Ok(original_input);
                }

                let provider = input_result.unwrap();
                let Some(provider) = provider else {
                    // No provider for this input; use the original input.
                    return Ok(original_input);
                };

                let Some(optimizer) = provider.optimizer() else {
                    // No optimizer for this input; use the original input.
                    return Ok(original_input);
                };

                // Replace the input with the federated counterpart
                let wrapped = wrap_projection(original_input)?;
                let optimized = optimizer.optimize(wrapped, _config, |_, _| {})?;

                Ok(optimized)
            })
            .collect::<Result<Vec<_>>>()?;

        // Optimize expressions if needed
        let new_expressions = if optimize_expressions {
            self.optimize_plan_exprs(plan, _config)?
        } else {
            plan.expressions()
        };

        // Construct the optimized plan
        let new_plan = plan.with_new_exprs(new_expressions, new_inputs)?;

        // Return the federated plan
        Ok((Some(new_plan), ScanResult::Ambiguous))
    }

    /// Optimizes all exprs of a plan
    fn optimize_plan_exprs(
        &self,
        plan: &LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Vec<Expr>> {
        plan.expressions()
            .iter()
            .map(|expr| {
                let transformed = expr
                    .clone()
                    .transform(&|e| self.optimize_expr_recursively(e, _config))?;
                Ok(transformed.data)
            })
            .collect::<Result<Vec<_>>>()
    }

    /// recursively optimize expressions
    /// Current logic: individually federate every sub-query.
    fn optimize_expr_recursively(
        &self,
        expr: Expr,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<Expr>> {
        match expr {
            Expr::ScalarSubquery(ref subquery) => {
                // Optimize as root to force federating the sub-query
                let (new_subquery, _) =
                    self.optimize_plan_recursively(&subquery.subquery, true, _config)?;
                let Some(new_subquery) = new_subquery else {
                    return Ok(Transformed::no(expr));
                };
                Ok(Transformed::yes(Expr::ScalarSubquery(
                    subquery.with_plan(new_subquery.into()),
                )))
            }
            Expr::InSubquery(_) => not_impl_err!("InSubquery"),
            _ => Ok(Transformed::no(expr)),
        }
    }
}

/// NopFederationProvider is used to represent tables that are not federated, but
/// are resolved by DataFusion. This simplifies the logic of the optimizer rule.
struct NopFederationProvider {}

impl FederationProvider for NopFederationProvider {
    fn name(&self) -> &str {
        "nop"
    }

    fn compute_context(&self) -> Option<String> {
        None
    }

    fn optimizer(&self) -> Option<Arc<Optimizer>> {
        None
    }
}

fn get_leaf_provider(plan: &LogicalPlan) -> Result<Option<FederationProviderRef>> {
    match plan {
        LogicalPlan::TableScan(TableScan { ref source, .. }) => {
            let Some(federated_source) = get_table_source(source)? else {
                // Table is not federated but provided by a standard table provider.
                // We use a placeholder federation provider to simplify the logic.
                return Ok(Some(Arc::new(NopFederationProvider {})));
            };
            let provider = federated_source.federation_provider();
            Ok(Some(provider))
        }
        _ => Ok(None),
    }
}

fn wrap_projection(plan: LogicalPlan) -> Result<LogicalPlan> {
    // TODO: minimize requested columns
    match plan {
        LogicalPlan::Projection(_) => Ok(plan),
        _ => {
            let expr = plan
                .schema()
                .columns()
                .iter()
                .map(|c| Expr::Column(c.clone()))
                .collect::<Vec<Expr>>();
            Ok(LogicalPlan::Projection(Projection::try_new(
                expr,
                Arc::new(plan),
            )?))
        }
    }
}

pub fn get_table_source(
    source: &Arc<dyn TableSource>,
) -> Result<Option<Arc<dyn FederatedTableSource>>> {
    // Unwrap TableSource
    let source = source_as_provider(source)?;

    // Get FederatedTableProviderAdaptor
    let Some(wrapper) = source
        .as_any()
        .downcast_ref::<FederatedTableProviderAdaptor>()
    else {
        return Ok(None);
    };

    // Return original FederatedTableSource
    Ok(Some(Arc::clone(&wrapper.source)))
}
