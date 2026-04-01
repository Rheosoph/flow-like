/**
 * Flow-Like WASM Node Template (AssemblyScript)
 *
 * Single-node template using the FlowNode class pattern.
 *
 * # Building
 *
 * ```bash
 * npm install
 * npm run build
 * ```
 *
 * The compiled `.wasm` file will be at: `build/release.wasm`
 */

import {
	type Context,
	DataType,
	type ExecutionResult,
	FlowNode,
	NodeDefinition,
	PinDefinition,
	runSingle,
	singleNode,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

export {
	alloc,
	dealloc,
	get_abi_version,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

class MyCustomNode extends FlowNode {
	define(): NodeDefinition {
		const def = new NodeDefinition();
		def.name = "my_custom_node_as";
		def.friendly_name = "My Custom Node (AS)";
		def.description = "A template WASM node built with AssemblyScript";
		def.category = "Custom/WASM";
		def.addPermission("streaming");

		def.addPin(
			PinDefinition.input(
				"exec",
				"Execute",
				"Trigger execution",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.input(
				"input_text",
				"Input Text",
				"Text to process",
				DataType.String,
			).withDefaultString(""),
		);
		def.addPin(
			PinDefinition.input(
				"multiplier",
				"Multiplier",
				"Number of times to repeat",
				DataType.I64,
			).withDefaultI64(1),
		);

		def.addPin(
			PinDefinition.output(
				"exec_out",
				"Done",
				"Execution complete",
				DataType.Exec,
			),
		);
		def.addPin(
			PinDefinition.output(
				"output_text",
				"Output Text",
				"Processed text",
				DataType.String,
			),
		);
		def.addPin(
			PinDefinition.output(
				"char_count",
				"Character Count",
				"Number of characters in output",
				DataType.I64,
			),
		);

		return def;
	}

	execute(ctx: Context): ExecutionResult {
		const inputText = ctx.getString("input_text");
		const multiplier = ctx.getI64("multiplier", 1);

		let outputText = "";
		for (let i: i64 = 0; i < multiplier; i++) {
			outputText += inputText;
		}

		ctx.setString("output_text", outputText);
		ctx.setI64("char_count", outputText.length);
		return ctx.success();
	}
}

const node = new MyCustomNode();

export function get_node(): i64 {
	return singleNode(node);
}

export function run(ptr: i32, len: i32): i64 {
	return runSingle(node, ptr, len);
}
