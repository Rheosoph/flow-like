import {
	type Context,
	DataType,
	type ExecutionResult,
	FlowNode,
	NodeDefinition,
	NodePackage,
	PinDefinition,
} from "@flow-like/wasm-sdk/assembly/index";

export {
	alloc,
	dealloc,
	get_abi_version,
} from "@flow-like/wasm-sdk/assembly/index";

export class IfBranchNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "if_branch_as";
		def.friendly_name = "If Branch (AS)";
		def.description = "Routes execution based on a condition";
		def.category = "Control Flow";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"condition",
				"Condition",
				"Branch condition",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.output(
				"true_branch",
				"True",
				"Executes when true",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.output(
				"false_branch",
				"False",
				"Executes when false",
				DataType.Exec,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		if (ctx.getBool("condition")) {
			ctx.activateExec("true_branch");
		} else {
			ctx.activateExec("false_branch");
		}
		return ctx.finish();
	}
}

export class CompareNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "compare_as";
		def.friendly_name = "Compare Numbers (AS)";
		def.description = "Compares two numbers and outputs comparison results";
		def.category = "Control Flow";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"a",
				"A",
				"First number",
				DataType.F64,
			).withDefaultF64(0.0),
		);
		def.addPin(
			PinDefinition.input(
				"b",
				"B",
				"Second number",
				DataType.F64,
			).withDefaultF64(0.0),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("equal", "Equal", "A equals B", DataType.Bool),
		);
		def.addPin(
			PinDefinition.output("greater", "Greater", "A > B", DataType.Bool),
		);
		def.addPin(PinDefinition.output("less", "Less", "A < B", DataType.Bool));
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const a = ctx.getF64("a");
		const b = ctx.getF64("b");
		ctx.setBool("equal", a == b);
		ctx.setBool("greater", a > b);
		ctx.setBool("less", a < b);
		return ctx.success();
	}
}

export class AndGateNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "and_gate_as";
		def.friendly_name = "AND Gate (AS)";
		def.description = "Logical AND of two boolean inputs";
		def.category = "Control Flow/Logic";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"a",
				"A",
				"First boolean",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.input(
				"b",
				"B",
				"Second boolean",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "A AND B", DataType.Bool),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setBool("result", ctx.getBool("a") && ctx.getBool("b"));
		return ctx.success();
	}
}

export class OrGateNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "or_gate_as";
		def.friendly_name = "OR Gate (AS)";
		def.description = "Logical OR of two boolean inputs";
		def.category = "Control Flow/Logic";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"a",
				"A",
				"First boolean",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.input(
				"b",
				"B",
				"Second boolean",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "A OR B", DataType.Bool),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setBool("result", ctx.getBool("a") || ctx.getBool("b"));
		return ctx.success();
	}
}

export class NotGateNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "not_gate_as";
		def.friendly_name = "NOT Gate (AS)";
		def.description = "Logical NOT of a boolean input";
		def.category = "Control Flow/Logic";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"value",
				"Value",
				"Boolean to negate",
				DataType.Bool,
			).withDefaultBool(false),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "NOT value", DataType.Bool),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setBool("result", !ctx.getBool("value"));
		return ctx.success();
	}
}

export class GateNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "gate_as";
		def.friendly_name = "Gate (AS)";
		def.description = "Passes execution only when the gate is open";
		def.category = "Control Flow";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"open",
				"Open",
				"Gate state",
				DataType.Bool,
			).withDefaultBool(true),
		);
		def.addPin(
			PinDefinition.output(
				"passed",
				"Passed",
				"Fires when gate is open",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.output(
				"blocked",
				"Blocked",
				"Fires when gate is closed",
				DataType.Exec,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		if (ctx.getBool("open")) {
			ctx.activateExec("passed");
		} else {
			ctx.activateExec("blocked");
		}
		return ctx.finish();
	}
}

const pkg = new NodePackage();
pkg.register(new IfBranchNode());
pkg.register(new CompareNode());
pkg.register(new AndGateNode());
pkg.register(new OrGateNode());
pkg.register(new NotGateNode());
pkg.register(new GateNode());

export function get_nodes(): i64 {
	return pkg.getNodes();
}

export function run(ptr: i32, len: i32): i64 {
	return pkg.run(ptr, len);
}
