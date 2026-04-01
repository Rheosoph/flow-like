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

export class AddNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "math_add_as";
		def.friendly_name = "Add (AS)";
		def.description = "Adds two numbers together";
		def.category = "Math/Arithmetic";

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
			PinDefinition.output("result", "Result", "Sum of A and B", DataType.F64),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setF64("result", ctx.getF64("a") + ctx.getF64("b"));
		return ctx.success();
	}
}

export class SubtractNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "math_subtract_as";
		def.friendly_name = "Subtract (AS)";
		def.description = "Subtracts B from A";
		def.category = "Math/Arithmetic";

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
			PinDefinition.output("result", "Result", "A minus B", DataType.F64),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setF64("result", ctx.getF64("a") - ctx.getF64("b"));
		return ctx.success();
	}
}

export class MultiplyNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "math_multiply_as";
		def.friendly_name = "Multiply (AS)";
		def.description = "Multiplies two numbers";
		def.category = "Math/Arithmetic";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"a",
				"A",
				"First number",
				DataType.F64,
			).withDefaultF64(1.0),
		);
		def.addPin(
			PinDefinition.input(
				"b",
				"B",
				"Second number",
				DataType.F64,
			).withDefaultF64(1.0),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "A times B", DataType.F64),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setF64("result", ctx.getF64("a") * ctx.getF64("b"));
		return ctx.success();
	}
}

export class DivideNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "math_divide_as";
		def.friendly_name = "Divide (AS)";
		def.description = "Divides A by B";
		def.category = "Math/Arithmetic";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input("a", "A", "Dividend", DataType.F64).withDefaultF64(
				0.0,
			),
		);
		def.addPin(
			PinDefinition.input("b", "B", "Divisor", DataType.F64).withDefaultF64(
				1.0,
			),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "A divided by B", DataType.F64),
		);
		def.addPin(
			PinDefinition.output(
				"is_valid",
				"Valid",
				"False if division by zero",
				DataType.Bool,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const a = ctx.getF64("a");
		const b = ctx.getF64("b");

		if (b == 0.0) {
			ctx.setF64("result", 0.0);
			ctx.setBool("is_valid", false);
		} else {
			ctx.setF64("result", a / b);
			ctx.setBool("is_valid", true);
		}
		return ctx.success();
	}
}

export class ClampNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "math_clamp_as";
		def.friendly_name = "Clamp (AS)";
		def.description = "Clamps a value between min and max";
		def.category = "Math/Utility";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"value",
				"Value",
				"Value to clamp",
				DataType.F64,
			).withDefaultF64(0.0),
		);
		def.addPin(
			PinDefinition.input(
				"min",
				"Min",
				"Minimum value",
				DataType.F64,
			).withDefaultF64(0.0),
		);
		def.addPin(
			PinDefinition.input(
				"max",
				"Max",
				"Maximum value",
				DataType.F64,
			).withDefaultF64(1.0),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "Clamped value", DataType.F64),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const value = ctx.getF64("value");
		const min = ctx.getF64("min");
		const max = ctx.getF64("max");
		ctx.setF64("result", Math.max(min, Math.min(max, value)));
		return ctx.success();
	}
}

const pkg = new NodePackage();
pkg.register(new AddNode());
pkg.register(new SubtractNode());
pkg.register(new MultiplyNode());
pkg.register(new DivideNode());
pkg.register(new ClampNode());

export function get_nodes(): i64 {
	return pkg.getNodes();
}

export function run(ptr: i32, len: i32): i64 {
	return pkg.run(ptr, len);
}
