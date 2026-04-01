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

export class UppercaseNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_uppercase_as";
		def.friendly_name = "To Uppercase (AS)";
		def.description = "Converts text to uppercase";
		def.category = "String/Transform";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Text to convert",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"result",
				"Result",
				"Uppercase text",
				DataType.String,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setString("result", ctx.getString("text").toUpperCase());
		return ctx.success();
	}
}

export class LowercaseNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_lowercase_as";
		def.friendly_name = "To Lowercase (AS)";
		def.description = "Converts text to lowercase";
		def.category = "String/Transform";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Text to convert",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"result",
				"Result",
				"Lowercase text",
				DataType.String,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setString("result", ctx.getString("text").toLowerCase());
		return ctx.success();
	}
}

export class TrimNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_trim_as";
		def.friendly_name = "Trim (AS)";
		def.description = "Removes leading and trailing whitespace";
		def.category = "String/Transform";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Text to trim",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output("result", "Result", "Trimmed text", DataType.String),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setString("result", ctx.getString("text").trim());
		return ctx.success();
	}
}

export class LengthNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_length_as";
		def.friendly_name = "String Length (AS)";
		def.description = "Returns the length of a string";
		def.category = "String/Analysis";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Text to measure",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"length",
				"Length",
				"Number of characters",
				DataType.I64,
			),
		);
		def.addPin(
			PinDefinition.output(
				"is_empty",
				"Is Empty",
				"True if string is empty",
				DataType.Bool,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const text = ctx.getString("text");
		ctx.setI64("length", text.length);
		ctx.setBool("is_empty", text.length == 0);
		return ctx.success();
	}
}

export class ContainsNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_contains_as";
		def.friendly_name = "Contains (AS)";
		def.description = "Checks if text contains a substring";
		def.category = "String/Analysis";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Text to search in",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"search",
				"Search",
				"Substring to find",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"result",
				"Found",
				"True if substring found",
				DataType.Bool,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setBool(
			"result",
			ctx.getString("text").includes(ctx.getString("search")),
		);
		return ctx.success();
	}
}

export class ReplaceNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_replace_as";
		def.friendly_name = "Replace (AS)";
		def.description = "Replaces occurrences of a pattern";
		def.category = "String/Transform";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"text",
				"Text",
				"Original text",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"find",
				"Find",
				"Pattern to find",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"replace_with",
				"Replace With",
				"Replacement text",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"result",
				"Result",
				"Modified text",
				DataType.String,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setString(
			"result",
			ctx
				.getString("text")
				.replaceAll(ctx.getString("find"), ctx.getString("replace_with")),
		);
		return ctx.success();
	}
}

export class ConcatNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "string_concat_as";
		def.friendly_name = "Concatenate (AS)";
		def.description = "Joins two strings together";
		def.category = "String/Transform";

		def.addPin(
			PinDefinition.input("exec", "Execute", "Trigger", DataType.Exec),
		);
		def.addPin(
			PinDefinition.input(
				"a",
				"A",
				"First string",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"b",
				"B",
				"Second string",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"separator",
				"Separator",
				"Text between strings",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.output("exec_out", "Done", "Complete", DataType.Exec),
		);
		def.addPin(
			PinDefinition.output(
				"result",
				"Result",
				"Combined string",
				DataType.String,
			),
		);
		return def;
	}

	execute(ctx: Context): ExecutionResult {
		ctx.setString(
			"result",
			ctx.getString("a") + ctx.getString("separator") + ctx.getString("b"),
		);
		return ctx.success();
	}
}

const pkg = new NodePackage();
pkg.register(new UppercaseNode());
pkg.register(new LowercaseNode());
pkg.register(new TrimNode());
pkg.register(new LengthNode());
pkg.register(new ContainsNode());
pkg.register(new ReplaceNode());
pkg.register(new ConcatNode());

export function get_nodes(): i64 {
	return pkg.getNodes();
}

export function run(ptr: i32, len: i32): i64 {
	return pkg.run(ptr, len);
}
