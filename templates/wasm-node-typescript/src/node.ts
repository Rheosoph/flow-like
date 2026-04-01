/**
 * Flow-Like WASM Node Template — Main Node (TypeScript)
 *
 * Edit getDefinition() to define your node's pins and run() to implement the logic.
 */

import {
	type Context,
	type ExecutionResult,
	NodeDefinition,
	PinDefinition,
	PinType,
} from "@flow-like/wasm-sdk-typescript";

export function getDefinition(): NodeDefinition {
	const nd = new NodeDefinition(
		"my_custom_node_ts",
		"My Custom Node",
		"A template WASM node built with TypeScript",
		"Custom/WASM",
	);

	nd.addPin(PinDefinition.inputExec("exec"));
	nd.addPin(
		PinDefinition.inputPin("input_text", PinType.STRING, { defaultValue: "" }),
	);
	nd.addPin(
		PinDefinition.inputPin("multiplier", PinType.I64, { defaultValue: 1 }),
	);

	nd.addPin(PinDefinition.outputExec("exec_out"));
	nd.addPin(PinDefinition.outputPin("output_text", PinType.STRING));
	nd.addPin(PinDefinition.outputPin("char_count", PinType.I64));

	nd.addPermission("streaming");

	return nd;
}

export function run(ctx: Context): ExecutionResult {
	const inputText = ctx.getString("input_text", "") ?? "";
	const multiplier = ctx.getI64("multiplier", 1) ?? 1;

	ctx.debug(`Processing: '${inputText}' x ${multiplier}`);

	const outputText = inputText.repeat(Math.max(multiplier, 0));
	const charCount = outputText.length;

	ctx.streamText(`Generated ${charCount} characters`);

	ctx.setOutput("output_text", outputText);
	ctx.setOutput("char_count", charCount);

	return ctx.success();
}
