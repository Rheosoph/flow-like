//! Give an untyped struct a declared shape.
//!
//! A struct that comes off a JSON parse, an HTTP response or a page payload carries no schema, so
//! `Break Struct` has nothing to derive its field pins from and every reader downstream is blind.
//! These two nodes are where a shape gets attached — one from a JSON Schema the user writes, one
//! from another struct that already has the shape. Attaching it is a claim about the data, so the
//! claim is checked: a value that could not be read as that shape takes the `Failed` branch with a
//! reason, rather than handing the rest of the board a schema the value does not honour.

use super::schema::{resolve_schema, resolve_schema_ref, unwrap_item_schema, value_matches_schema};
use crate::utils::json::parse_with_schema::into_json_schema;
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, dynamic_pin_source_literal},
    pin::{PinOptions, is_open_object_schema},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    json::{self, json},
};

/// The pins both casts share. Only where the target shape comes from differs.
///
/// `struct_in`/`struct_out` are not free-choice names: `isStructIOPin` in
/// `packages/ui/lib/flow-board-utils.tsx` and `schema_constraints_are_compatible` in
/// `flow/ast/reconcile.rs` both key the "this pin adopts whatever schema it is handed" rule on
/// those exact strings. Rename either and every typed struct producer stops being wireable.
fn add_cast_pins(node: &mut Node) {
    node.add_icon("/flow/icons/struct.svg");
    node.set_receiver("struct_in");
    node.set_scores(pure_scores());

    node.add_input_pin("exec_in", "In", "Run the cast", VariableType::Execution);
    node.add_input_pin(
        "struct_in",
        "Struct",
        "The struct to cast. Whatever schema it arrives with is ignored",
        VariableType::Struct,
    )
    .set_open_schema();

    node.add_output_pin(
        "exec_out",
        "Success",
        "The value fits the shape",
        VariableType::Execution,
    );
    // Declared open, never concrete: `repair_catalog_pin_schemas` treats a concrete catalog schema
    // as authoritative and would overwrite what `on_update` stamped on every board load.
    node.add_output_pin(
        "struct_out",
        "Struct",
        "The same value, now declaring the target shape",
        VariableType::Struct,
    )
    .set_open_schema();
    node.add_output_pin(
        "error",
        "Failed",
        "The value does not fit the shape",
        VariableType::Execution,
    );
    node.add_output_pin(
        "error_message",
        "Reason",
        "What did not fit, naming the field",
        VariableType::String,
    );
}

/// Read what the user declared into the schema document a value is checked against.
///
/// An OpenAI function definition is accepted alongside a JSON Schema — the same two spellings
/// `Parse JSON with Schema` takes — so a tool definition doubles as a shape.
fn parse_declared_schema(declared: &str) -> Result<Value, String> {
    if declared.trim().is_empty() {
        return Err("No schema given".to_string());
    }

    let parsed: Value =
        json::from_str(declared).map_err(|error| format!("Schema is not valid JSON: {error}"))?;

    let schema =
        into_json_schema(parsed).map_err(|error| format!("Schema is not usable: {error}"))?;

    // A schema is an object, or the bare `true`/`false` the dialect allows. Anything else — a
    // JSON array, a lone string, an example payload pasted in by mistake — constrains nothing, and
    // the matcher would wave every value through it without a word.
    if !schema.is_object() && !schema.is_boolean() {
        return Err("Schema must be a JSON object, not a bare value".to_string());
    }

    Ok(schema)
}

/// Put the resolved shape on `struct_out`, or hand the pin its open marker back.
///
/// The marker is what lets the output reach any struct consumer. Leaving the last resolved schema
/// behind instead would make the pin a contract for a shape nothing is producing any more, and
/// `doPinsMatch` would then reject the next consumer the user wires up.
fn set_output_schema(node: &mut Node, schema: Option<String>) {
    let Some(pin) = node.get_pin_mut_by_name("struct_out") else {
        return;
    };
    match schema {
        Some(schema) => pin.schema = Some(schema),
        None => {
            pin.set_open_schema();
        }
    }
}

/// Check the value and publish it, or take the `Failed` branch with the reason.
async fn run_cast(
    context: &mut ExecutionContext,
    value: Value,
    root: &Value,
) -> flow_like_types::Result<()> {
    // A struct pin's schema always describes ONE element, so a `Vec<T>` document is read as its
    // `T` — the same unwrapping `Break Struct` does — and the matcher then checks a value that
    // arrives as a list of them element by element.
    let target = unwrap_item_schema(resolve_schema(root, root), root);

    match value_matches_schema(&value, target, root) {
        Ok(()) => {
            context.set_pin_value("error_message", json!("")).await?;
            context.set_pin_value("struct_out", value).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        Err(reason) => fail(context, reason).await,
    }
}

/// A cast that does not hold is an outcome, not a node error, so this returns `Ok` and lets the
/// graph carry on down `Failed`. Returning `Err` instead would route to `handle_error` and abort
/// the run without ever firing the branch the user wired.
async fn fail(context: &mut ExecutionContext, reason: String) -> flow_like_types::Result<()> {
    context.log_message(&format!("Cast failed: {reason}"), LogLevel::Warn);
    context.set_pin_value("struct_out", Value::Null).await?;
    context
        .set_pin_value("error_message", json!(reason))
        .await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

/// The schema `on_update` stamped on `struct_out`, as JSON.
///
/// Read from the node rather than from a pin value: the run template carries pin schemas as ref
/// keys, so this has to go back through the board's table. `InternalPin` has no `schema` field at
/// all, which is why the full `Node` has to be locked for it.
async fn stamped_output_schema(context: &ExecutionContext) -> Option<String> {
    let declared = {
        let node = context.node.node.lock().await;
        node.get_pin_by_name("struct_out")
            .and_then(|pin| pin.schema.clone())
    }?;

    let refs = context
        .get_board()
        .await
        .ok()
        .map(|board| board.refs.clone());
    Some(match refs {
        Some(refs) => resolve_schema_ref(declared, &refs),
        None => declared,
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct CastToSchemaNode {}

impl CastToSchemaNode {
    pub fn new() -> Self {
        CastToSchemaNode {}
    }
}

#[async_trait]
impl NodeLogic for CastToSchemaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_cast_to_schema",
            "Cast to Schema",
            "Checks a struct against a JSON schema and hands it on carrying that shape",
            "Structs",
        );
        node.set_flowscript_name("struct", "castToSchema");
        add_cast_pins(&mut node);

        node.add_input_pin(
            "schema",
            "Schema",
            "JSON Schema or OpenAI function definition describing the target shape",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // `doPinsMatch` compares schema *strings*, and a schema typed into this pin will never be
        // byte-identical to the canonical JSON `Pin::set_schema::<T>()` emits for the very same
        // type — different key order, an extra `$schema`, a `title`. Enforcing that comparison
        // would leave the output unable to reach the typed consumers it exists to feed, so the
        // user's cast is taken at its word here and checked where it can be: at run time.
        if let Some(pin) = node.get_pin_mut_by_name("struct_out") {
            pin.set_options(PinOptions::new().set_enforce_schema(false).build());
        }

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let value: Value = context.evaluate_pin("struct_in").await?;
        let declared: String = context.evaluate_pin("schema").await?;

        match parse_declared_schema(&declared) {
            Ok(root) => run_cast(context, value, &root).await,
            Err(reason) => fail(context, reason).await,
        }
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        // `dynamic_pin_source_literal` answers `None` while the pin is wired, which is the case
        // that matters: the editor hides the literal behind the wire, so a stale one survives
        // underneath it and only the run knows the real schema.
        let Some(declared) = dynamic_pin_source_literal(node, "schema") else {
            set_output_schema(node, None);
            return;
        };

        if declared.trim().is_empty() {
            set_output_schema(node, None);
            return;
        }

        match parse_declared_schema(&declared) {
            // Re-serialized rather than stamped as typed: `on_update` has to reach a fixed point,
            // and the board settles by comparing node hashes, which cover `pin.schema`. Going
            // through `Value` makes the stamp depend on the schema and not on the whitespace.
            Ok(schema) => set_output_schema(node, json::to_string(&schema).ok()),
            Err(reason) => {
                set_output_schema(node, None);
                node.error = Some(reason);
            }
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CastToStructNode {}

impl CastToStructNode {
    pub fn new() -> Self {
        CastToStructNode {}
    }
}

/// Mirror the donor's shape onto `struct_shape` so the pin shows what it is lending.
///
/// `enforce_schema` stays off, and is re-set on every pass because a catalog version bump copies
/// `options` back over the pin: this pin exists to be re-pointed at a different shape, and an
/// enforced schema is one `doPinsMatch` refuses to let the user replace.
fn set_shape_schema(node: &mut Node, schema: Option<String>) {
    let Some(pin) = node.get_pin_mut_by_name("struct_shape") else {
        return;
    };
    match schema {
        Some(schema) => pin.schema = Some(schema),
        None => {
            pin.set_open_schema();
        }
    }
    pin.set_options(PinOptions::new().set_enforce_schema(false).build());
}

fn forget_shape(node: &mut Node, error: Option<String>) {
    set_shape_schema(node, None);
    set_output_schema(node, None);
    node.error = error;
}

fn missing_shape() -> String {
    "No target shape: wire a struct that declares one into Shape".to_string()
}

#[async_trait]
impl NodeLogic for CastToStructNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_cast_to_struct",
            "Cast to Struct",
            "Checks a struct against the shape of another struct and hands it on carrying that shape",
            "Structs",
        );
        node.set_flowscript_name("struct", "castToStruct");
        add_cast_pins(&mut node);

        // `struct_shape` is the third name `isStructIOPin` knows. Without that hatch a producer
        // that sets `enforce_schema` — the typed struct outputs this pin is most useful with —
        // is refused against the open marker before any other rule is reached.
        node.add_input_pin(
            "struct_shape",
            "Shape",
            "A struct of the shape to cast to. Only its schema is read — its value is never evaluated",
            VariableType::Struct,
        )
        .set_open_schema()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let value: Value = context.evaluate_pin("struct_in").await?;

        // The shape came off the donor pin at design time and lives on `struct_out`. Nothing
        // upstream of `struct_shape` has to run for the cast to happen — the pin lends a type,
        // not a value.
        let Some(declared) = stamped_output_schema(context).await else {
            return fail(context, missing_shape()).await;
        };

        if is_open_object_schema(&declared) {
            return fail(context, missing_shape()).await;
        }

        match json::from_str::<Value>(&declared) {
            Ok(root) => run_cast(context, value, &root).await,
            Err(error) => fail(context, format!("Target shape is not valid JSON: {error}")).await,
        }
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let Some(shape_pin) = node.get_pin_by_name("struct_shape") else {
            return;
        };

        let Some(donor_id) = shape_pin.depends_on.iter().next().cloned() else {
            forget_shape(node, None);
            return;
        };

        // A producer that is not on the board handed to us is not evidence that the wire is gone:
        // `node_updates` lifts the node being updated out of the board, and on load this runs
        // before `cleanup` has repaired anything. Keep the shape and wait for a full pass.
        let Some(donor) = board.get_pin_by_id(&donor_id) else {
            return;
        };

        let Some(schema_ref) = donor.schema.clone() else {
            forget_shape(node, Some("Connected struct has no schema".to_string()));
            return;
        };

        let schema = resolve_schema_ref(schema_ref, &board.refs);

        if is_open_object_schema(&schema) {
            forget_shape(node, Some(missing_shape()));
            return;
        }

        // Stamped verbatim, both here and on the output: `doPinsMatch` compares schema strings, so
        // passing the donor's own bytes through is what lets the cast reach a consumer that
        // declares the very same type.
        set_shape_schema(node, Some(schema.clone()));
        set_output_schema(node, Some(schema));
    }
}
