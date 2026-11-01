use super::*;

impl StructuralPlanner<'_> {
    pub(super) fn warn_repeated_node_scoped_dirs(&mut self, block: &Block, scope: &str) {
        let mut calls = Vec::new();
        collect_block_calls(block, &mut calls);
        let mut counts = BTreeMap::<&str, usize>::new();
        let mut anchors = HashSet::new();
        for call in calls {
            let Ok(resolved) = self.resolve_call_target(call) else {
                continue;
            };
            let CallTarget::Catalog(metadata) = resolved.target else {
                continue;
            };
            let helper = match metadata.name.as_str() {
                "path_from_cache_dir" => "files::cacheDir",
                "path_from_storage_dir" => "files::storageDir",
                "path_from_user_dir" => "files::userDir",
                _ => continue,
            };
            if !resolved.call.args.iter().any(|arg| {
                pin_name_matches("node_scope", &arg.name)
                    && matches!(&arg.value, Expr::Literal(Literal::Bool(true)))
            }) {
                continue;
            }
            // An anchored helper can appear inline at several consumers of the same node.
            if let Some(anchor) = call.anchor.as_deref().filter(|anchor| !anchor.is_empty())
                && !anchors.insert(anchor)
            {
                continue;
            }
            *counts.entry(helper).or_default() += 1;
        }
        self.result.warnings.extend(counts.into_iter().filter_map(|(helper, count)| {
            (count > 1).then(|| format!(
                "Repeated `{helper}` calls with nodeScope: true in {scope} use different directories because each path is unique to its node. Reuse one shared path variable, or set nodeScope: false."
            ))
        }));
    }
}

fn collect_call<'a>(call: &'a Call, calls: &mut Vec<&'a Call>) {
    calls.push(call);
    if let Some(receiver) = &call.receiver {
        collect_expr_calls(receiver, calls);
    }
    for value in &call.positional {
        collect_expr_calls(value, calls);
    }
    for arg in &call.args {
        collect_expr_calls(&arg.value, calls);
    }
}

fn collect_expr_calls<'a>(expr: &'a Expr, calls: &mut Vec<&'a Call>) {
    match expr {
        Expr::Call(call) => collect_call(call, calls),
        Expr::Field { base, .. } | Expr::Member { base, .. } => collect_expr_calls(base, calls),
        Expr::Object(fields) => {
            for field in fields {
                collect_expr_calls(&field.value, calls);
            }
        }
        Expr::Array(items) => {
            for item in items {
                collect_expr_calls(item, calls);
            }
        }
        Expr::Index { base, index } => {
            collect_expr_calls(base, calls);
            collect_expr_calls(index, calls);
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            collect_expr_calls(cond, calls);
            collect_expr_calls(then, calls);
            collect_expr_calls(otherwise, calls);
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr_calls(lhs, calls);
            collect_expr_calls(rhs, calls);
        }
        Expr::Template { parts } => {
            for part in parts {
                if let TemplatePart::Expr(expr) = part {
                    collect_expr_calls(expr, calls);
                }
            }
        }
        Expr::Ref(_) | Expr::Literal(_) => {}
    }
}

fn collect_block_calls<'a>(block: &'a Block, calls: &mut Vec<&'a Call>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { call, .. } | Stmt::Destructure { call, .. } | Stmt::Call { call, .. } => {
                collect_call(call, calls);
            }
            Stmt::Branch {
                call,
                condition,
                arms,
                ..
            } => {
                collect_call(call, calls);
                if let Some(condition) = condition {
                    collect_expr_calls(condition, calls);
                }
                for arm in arms {
                    collect_block_calls(&arm.body, calls);
                }
            }
            Stmt::Loop {
                call,
                iterable,
                body,
                ..
            } => {
                collect_call(call, calls);
                if let Some(iterable) = iterable {
                    collect_expr_calls(iterable, calls);
                }
                collect_block_calls(body, calls);
            }
            Stmt::Assign { value, .. }
            | Stmt::FieldAssign { value, .. }
            | Stmt::LocalAlias { value, .. } => collect_expr_calls(value, calls),
            Stmt::Return { values, .. } => {
                for value in values {
                    collect_expr_calls(value, calls);
                }
            }
            Stmt::Collapse(group) => collect_block_calls(&group.body, calls),
            // Nested handlers are independent events, linted by plan_event.
            Stmt::Handler(_) | Stmt::Local(_) | Stmt::Comment(_) => {}
        }
    }
}
