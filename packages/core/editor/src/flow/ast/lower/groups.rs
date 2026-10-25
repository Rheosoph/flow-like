//! Lowering of Collapsed/Macro layers into `@collapse` groups.
//!
//! A statement renders inside the groups its node sits in, from the section root (board root,
//! module or Function layer) down. Consecutive statements of one group share a block; a group
//! whose statements are not consecutive renders as several blocks with the same `//@l:` anchor.
//! Blocks nest lexically, so a statement whose group is not inside the group of the statement
//! enclosing it (a branch arm outside the group holding its `if`) restarts from the section root
//! under `@collapse("/")`. Reconcile reads every form back to the same placement.

use std::collections::HashSet;

use flow_like_ast::model::{Block, CollapseBlock, EventBlock, FnDecl, Stmt};

use super::{DetachedChain, Lowering};
use crate::flow::board::LayerType;

/// One statement and the group path of its node, `None` when it has no placed node.
type Placed = (Stmt, Option<Vec<String>>);

impl Lowering<'_> {
    /// Wrap the statements of every section in the groups their nodes sit in.
    pub(super) fn group_sections(
        &self,
        functions: &mut [FnDecl],
        events: &mut [EventBlock],
        detached: &mut [DetachedChain],
    ) {
        for function in functions {
            let root = function.anchor.clone();
            let body = std::mem::take(&mut function.body);
            function.body = self.group_block(body, root.as_deref(), None, &[]);
        }
        for event in events {
            let entry = event.anchor.clone();
            let root = entry
                .as_deref()
                .and_then(|id| self.nodes_by_id.get(id))
                .and_then(|node| self.section_root_of(node.layer.as_deref()));
            let body = std::mem::take(&mut event.body);
            event.body = self.group_block(body, root.as_deref(), entry.as_deref(), &[]);
        }
        for chain in detached {
            let root = chain
                .body
                .root_anchor()
                .and_then(|id| self.nodes_by_id.get(id))
                .and_then(|node| self.section_root_of(node.layer.as_deref()));
            let body = std::mem::take(&mut chain.body);
            chain.body = self.group_block(body, root.as_deref(), None, &[]);
        }
    }

    /// `block` with its statements grouped relative to `context`, the group path the block is
    /// written in. `entry` is the section's own entry node, a header rather than a statement.
    fn group_block(
        &self,
        block: Block,
        root: Option<&str>,
        entry: Option<&str>,
        context: &[String],
    ) -> Block {
        let placed = block
            .stmts
            .into_iter()
            .map(|mut stmt| {
                let path = self.statement_group_path(&stmt, root, entry);
                let inner = path.clone().unwrap_or_else(|| context.to_vec());
                self.group_nested(&mut stmt, root, entry, &inner);
                (stmt, path)
            })
            .collect();
        Block {
            stmts: self.emit_groups(placed, context),
        }
    }

    fn group_nested(
        &self,
        stmt: &mut Stmt,
        root: Option<&str>,
        entry: Option<&str>,
        context: &[String],
    ) {
        match stmt {
            Stmt::Branch { arms, .. } => {
                for arm in arms {
                    let body = std::mem::take(&mut arm.body);
                    arm.body = self.group_block(body, root, entry, context);
                }
            }
            Stmt::Loop { body, .. } => {
                let taken = std::mem::take(body);
                *body = self.group_block(taken, root, entry, context);
            }
            Stmt::Handler(event) => {
                let handler_entry = event.anchor.clone();
                let taken = std::mem::take(&mut event.body);
                event.body = self.group_block(taken, root, handler_entry.as_deref(), context);
            }
            _ => {}
        }
    }

    /// Emit `placed` relative to `context`: statements of a deeper group nest in a block per
    /// group level, statements outside `context` restart from the section root.
    fn emit_groups(&self, placed: Vec<Placed>, context: &[String]) -> Vec<Stmt> {
        let mut out = Vec::with_capacity(placed.len());
        let mut pending = placed.into_iter().peekable();
        while let Some((stmt, path)) = pending.next() {
            match path {
                Some(path) if !path.starts_with(context) => {
                    let run = take_run((stmt, Some(path)), &mut pending, |next| {
                        !next.starts_with(context)
                    });
                    out.push(self.root_block(run));
                }
                Some(path) if path.len() > context.len() => {
                    let group = path[context.len()].clone();
                    let run = take_run((stmt, Some(path)), &mut pending, |next| {
                        next.len() > context.len()
                            && next.starts_with(context)
                            && next[context.len()] == group
                    });
                    out.push(self.group_block_stmt(run, context, group));
                }
                _ => out.push(stmt),
            }
        }
        out
    }

    /// `@collapse("/") { … }`: statements whose group is not inside the enclosing one.
    fn root_block(&self, run: Vec<Placed>) -> Stmt {
        Stmt::Collapse(CollapseBlock {
            path: Vec::new(),
            absolute: true,
            body: Block {
                stmts: self.emit_groups(run, &[]),
            },
            anchor: None,
        })
    }

    /// `@collapse("Name") { … }   //@l:<group>` one level below `context`.
    fn group_block_stmt(&self, run: Vec<Placed>, context: &[String], group: String) -> Stmt {
        let mut inner = context.to_vec();
        inner.push(group.clone());
        let name = self
            .board
            .layers
            .get(&group)
            .map(|layer| layer.name.clone())
            .unwrap_or_default();
        Stmt::Collapse(CollapseBlock {
            path: vec![name],
            absolute: false,
            body: Block {
                stmts: self.emit_groups(run, &inner),
            },
            anchor: Some(group),
        })
    }

    /// The group path of the node a statement renders, outermost group first; `None` for a
    /// statement without a placed node (locals, function returns, the section's own entry).
    fn statement_group_path(
        &self,
        stmt: &Stmt,
        root: Option<&str>,
        entry: Option<&str>,
    ) -> Option<Vec<String>> {
        let anchor = match stmt {
            Stmt::Handler(event) => event.anchor.as_deref(),
            other => other.anchor(),
        }?;
        if Some(anchor) == entry {
            return None;
        }
        let node = self.nodes_by_id.get(anchor)?;
        let (path, section_root) = self.group_chain(node.layer.as_deref())?;
        (section_root.as_deref() == root).then_some(path)
    }

    /// The section root a layer belongs to: the first layer above it that is not a group.
    fn section_root_of(&self, layer: Option<&str>) -> Option<String> {
        self.group_chain(layer).and_then(|(_, root)| root)
    }

    /// The groups from `layer` up to its section root (outermost first), and that root. `None`
    /// for a damaged hierarchy (a cycle or a missing layer), which renders ungrouped.
    fn group_chain(&self, layer: Option<&str>) -> Option<(Vec<String>, Option<String>)> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = layer.filter(|id| !id.is_empty());
        while let Some(id) = current {
            if !seen.insert(id) {
                return None;
            }
            let layer = self.board.layers.get(id)?;
            if !matches!(layer.r#type, LayerType::Collapsed | LayerType::Macro) {
                break;
            }
            chain.push(id.to_string());
            current = layer
                .parent_id
                .as_deref()
                .filter(|parent| !parent.is_empty());
        }
        chain.reverse();
        Some((chain, current.map(str::to_string)))
    }
}

/// `first` and the statements after it that `belongs` admits, up to the first that it does not.
fn take_run(
    first: Placed,
    pending: &mut std::iter::Peekable<std::vec::IntoIter<Placed>>,
    belongs: impl Fn(&Vec<String>) -> bool,
) -> Vec<Placed> {
    let mut run = vec![first];
    while let Some((_, Some(next))) = pending.peek() {
        if !belongs(next) {
            break;
        }
        run.extend(pending.next());
    }
    run
}
