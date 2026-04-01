import {
	Context,
	ExecutionInput,
	LogLevel,
	MockHostBridge,
	NodeDefinition,
	setHost,
} from "@flow-like/wasm-sdk-typescript";
import { describe, expect, it } from "vitest";
import { getDefinition, run } from "../src/node";

function makeContext(
	inputs: Record<string, unknown> = {},
	host?: MockHostBridge,
): { ctx: Context; host: MockHostBridge } {
	const h = host ?? new MockHostBridge();
	setHost(h);
	const input = ExecutionInput.fromDict({
		inputs,
		node_id: "test-node",
		run_id: "test-run",
		app_id: "test-app",
		board_id: "test-board",
		user_id: "test-user",
		stream_state: true,
		log_level: LogLevel.DEBUG,
		node_name: "my_custom_node_ts",
	});
	return { ctx: new Context(input, h), host: h };
}

describe("Node Definition", () => {
	it("returns a valid NodeDefinition", () => {
		const def = getDefinition();
		expect(def).toBeInstanceOf(NodeDefinition);
		expect(def.name).toBe("my_custom_node_ts");
		expect(def.friendlyName).toBe("My Custom Node");
		expect(def.category).toBe("Custom/WASM");
	});

	it("has required exec pins", () => {
		const def = getDefinition();
		const pinNames = def.pins.map((p) => p.name);
		expect(pinNames).toContain("exec");
		expect(pinNames).toContain("exec_out");
	});

	it("has input pins", () => {
		const def = getDefinition();
		const inputs = def.pins.filter(
			(p) => p.pinType === "Input" && p.dataType !== "Exec",
		);
		expect(inputs.length).toBeGreaterThanOrEqual(1);
		const inputNames = inputs.map((p) => p.name);
		expect(inputNames).toContain("input_text");
		expect(inputNames).toContain("multiplier");
	});

	it("has output pins", () => {
		const def = getDefinition();
		const outputs = def.pins.filter(
			(p) => p.pinType === "Output" && p.dataType !== "Exec",
		);
		expect(outputs.length).toBeGreaterThanOrEqual(1);
		const outputNames = outputs.map((p) => p.name);
		expect(outputNames).toContain("output_text");
		expect(outputNames).toContain("char_count");
	});

	it("serializes to valid dict", () => {
		const def = getDefinition();
		const dict = def.toDict();
		expect(dict.name).toBe("my_custom_node_ts");
		expect(dict.friendly_name).toBe("My Custom Node");
		expect(dict.pins).toBeInstanceOf(Array);
		expect(dict.abi_version).toBe(1);
	});

	it("serializes to valid JSON array (as getNode would)", () => {
		const def = getDefinition();
		const json = JSON.stringify([def.toDict()]);
		const parsed = JSON.parse(json);
		expect(parsed).toHaveLength(1);
		expect(parsed[0].name).toBe("my_custom_node_ts");
	});
});

describe("Node Execution", () => {
	it("repeats input text by multiplier", () => {
		const { ctx } = makeContext({ input_text: "ab", multiplier: 3 });
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("ababab");
		expect(result.outputs.char_count).toBe(6);
		expect(result.activateExec).toContain("exec_out");
	});

	it("handles empty text", () => {
		const { ctx } = makeContext({ input_text: "", multiplier: 5 });
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("");
		expect(result.outputs.char_count).toBe(0);
	});

	it("handles multiplier of zero", () => {
		const { ctx } = makeContext({ input_text: "hello", multiplier: 0 });
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("");
		expect(result.outputs.char_count).toBe(0);
	});

	it("handles multiplier of one", () => {
		const { ctx } = makeContext({ input_text: "test", multiplier: 1 });
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("test");
		expect(result.outputs.char_count).toBe(4);
	});

	it("handles negative multiplier", () => {
		const { ctx } = makeContext({ input_text: "test", multiplier: -1 });
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("");
		expect(result.outputs.char_count).toBe(0);
	});

	it("uses default values when inputs missing", () => {
		const { ctx } = makeContext({});
		const result = run(ctx);
		expect(result.outputs.output_text).toBe("");
		expect(result.outputs.char_count).toBe(0);
		expect(result.activateExec).toContain("exec_out");
	});

	it("streams progress when streaming enabled", () => {
		const { ctx, host } = makeContext({ input_text: "hi", multiplier: 2 });
		run(ctx);
		const textStreams = host.streams.filter(([type]) => type === "text");
		expect(textStreams.length).toBeGreaterThanOrEqual(1);
	});

	it("logs debug messages", () => {
		const { ctx, host } = makeContext({ input_text: "x", multiplier: 1 });
		run(ctx);
		expect(host.logs.length).toBeGreaterThanOrEqual(1);
		const debugLogs = host.logs.filter(([level]) => level === LogLevel.DEBUG);
		expect(debugLogs.length).toBeGreaterThanOrEqual(1);
	});

	it("result serializes to valid JSON", () => {
		const { ctx } = makeContext({ input_text: "a", multiplier: 2 });
		const result = run(ctx);
		const json = result.toJSON();
		const parsed = JSON.parse(json);
		expect(parsed.outputs.output_text).toBe("aa");
		expect(parsed.activate_exec).toContain("exec_out");
		expect(parsed.error).toBeUndefined();
	});
});
