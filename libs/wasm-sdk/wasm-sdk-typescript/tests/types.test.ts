import { describe, expect, it } from "vitest";
import {
	ABI_VERSION,
	ExecutionInput,
	ExecutionResult,
	LogLevel,
	NodeDefinition,
	NodeScores,
	PackageNodes,
	PinDefinition,
	PinType,
	ValueType,
	humanize,
} from "../src/types";

describe("PinType", () => {
	it("has all expected types", () => {
		expect(PinType.EXEC).toBe("Exec");
		expect(PinType.STRING).toBe("String");
		expect(PinType.I64).toBe("I64");
		expect(PinType.F64).toBe("F64");
		expect(PinType.BOOL).toBe("Bool");
		expect(PinType.GENERIC).toBe("Generic");
		expect(PinType.BYTES).toBe("Bytes");
		expect(PinType.DATE).toBe("Date");
		expect(PinType.PATH_BUF).toBe("PathBuf");
		expect(PinType.STRUCT).toBe("Struct");
	});

	it("validates known types", () => {
		expect(PinType.validate("String")).toBe("String");
		expect(PinType.validate("I64")).toBe("I64");
	});

	it("rejects unknown types", () => {
		expect(() => PinType.validate("Unknown")).toThrow("Invalid pin data type");
	});
});

describe("ValueType", () => {
	it("has expected value types", () => {
		expect(ValueType.NORMAL).toBe("Normal");
		expect(ValueType.ARRAY).toBe("Array");
		expect(ValueType.HASH_MAP).toBe("HashMap");
		expect(ValueType.HASH_SET).toBe("HashSet");
	});
});

describe("humanize", () => {
	it("converts snake_case to Title Case", () => {
		expect(humanize("input_text")).toBe("Input Text");
		expect(humanize("exec_out")).toBe("Exec Out");
		expect(humanize("a_b_c")).toBe("A B C");
	});

	it("handles single words", () => {
		expect(humanize("exec")).toBe("Exec");
	});

	it("handles empty string", () => {
		expect(humanize("")).toBe("");
	});
});

describe("PinDefinition", () => {
	it("creates input pin with defaults", () => {
		const pin = PinDefinition.inputPin("my_input", PinType.STRING);
		expect(pin.name).toBe("my_input");
		expect(pin.friendlyName).toBe("My Input");
		expect(pin.pinType).toBe("Input");
		expect(pin.dataType).toBe("String");
		expect(pin.defaultValue).toBeNull();
	});

	it("creates input pin with custom options", () => {
		const pin = PinDefinition.inputPin("val", PinType.I64, {
			description: "A number",
			defaultValue: 42,
			friendlyName: "Value",
		});
		expect(pin.description).toBe("A number");
		expect(pin.defaultValue).toBe(42);
		expect(pin.friendlyName).toBe("Value");
	});

	it("creates output pin", () => {
		const pin = PinDefinition.outputPin("result", PinType.STRING);
		expect(pin.pinType).toBe("Output");
		expect(pin.dataType).toBe("String");
	});

	it("creates exec pins", () => {
		const input = PinDefinition.inputExec();
		expect(input.name).toBe("exec");
		expect(input.dataType).toBe("Exec");
		expect(input.pinType).toBe("Input");

		const output = PinDefinition.outputExec();
		expect(output.name).toBe("exec_out");
		expect(output.pinType).toBe("Output");
	});

	it("supports chained builder methods", () => {
		const pin = PinDefinition.inputPin("x", PinType.I64)
			.withDefault(10)
			.withValueType(ValueType.ARRAY)
			.withSchema("int[]")
			.withValidValues(["1", "2", "3"])
			.withRange(0, 100);

		expect(pin.defaultValue).toBe(10);
		expect(pin.valueType).toBe("Array");
		expect(pin.schema).toBe("int[]");
		expect(pin.validValues).toEqual(["1", "2", "3"]);
		expect(pin.range).toEqual([0, 100]);
	});

	it("serializes to dict correctly", () => {
		const pin = PinDefinition.inputPin("test", PinType.STRING, {
			defaultValue: "hello",
		});
		const dict = pin.toDict();
		expect(dict.name).toBe("test");
		expect(dict.friendly_name).toBe("Test");
		expect(dict.pin_type).toBe("Input");
		expect(dict.data_type).toBe("String");
		expect(dict.default_value).toBe("hello");
	});

	it("omits null fields in dict", () => {
		const pin = PinDefinition.inputExec();
		const dict = pin.toDict();
		expect(dict.value_type).toBeUndefined();
		expect(dict.schema).toBeUndefined();
		expect(dict.valid_values).toBeUndefined();
		expect(dict.range).toBeUndefined();
	});
});

describe("NodeScores", () => {
	it("creates with defaults", () => {
		const scores = new NodeScores();
		expect(scores.privacy).toBe(0);
		expect(scores.security).toBe(0);
	});

	it("creates with partial data", () => {
		const scores = new NodeScores({ privacy: 5, cost: 3 });
		expect(scores.privacy).toBe(5);
		expect(scores.cost).toBe(3);
		expect(scores.security).toBe(0);
	});

	it("serializes to dict", () => {
		const scores = new NodeScores({ privacy: 1, security: 2 });
		const dict = scores.toDict();
		expect(dict.privacy).toBe(1);
		expect(dict.security).toBe(2);
		expect(dict.performance).toBe(0);
	});
});

describe("NodeDefinition", () => {
	it("creates with required fields", () => {
		const nd = new NodeDefinition(
			"test_node",
			"Test",
			"A test node",
			"Testing",
		);
		expect(nd.name).toBe("test_node");
		expect(nd.friendlyName).toBe("Test");
		expect(nd.category).toBe("Testing");
		expect(nd.abiVersion).toBe(ABI_VERSION);
		expect(nd.pins).toEqual([]);
	});

	it("adds pins", () => {
		const nd = new NodeDefinition("n", "N", "d", "c");
		nd.addPin(PinDefinition.inputExec());
		nd.addPin(PinDefinition.inputPin("x", PinType.I64));
		nd.addPin(PinDefinition.outputExec());
		expect(nd.pins).toHaveLength(3);
	});

	it("supports chaining", () => {
		const nd = new NodeDefinition("n", "N", "d", "c")
			.addPin(PinDefinition.inputExec())
			.setScores(new NodeScores({ privacy: 5 }))
			.setLongRunning(true);

		expect(nd.pins).toHaveLength(1);
		expect(nd.scores?.privacy).toBe(5);
		expect(nd.longRunning).toBe(true);
	});

	it("serializes to dict with snake_case keys", () => {
		const nd = new NodeDefinition("my_node", "My Node", "desc", "Cat", {
			icon: "star",
			docs: "Some docs",
		});
		nd.addPin(PinDefinition.inputExec());
		nd.setLongRunning(false);

		const dict = nd.toDict();
		expect(dict.name).toBe("my_node");
		expect(dict.friendly_name).toBe("My Node");
		expect(dict.category).toBe("Cat");
		expect(dict.icon).toBe("star");
		expect(dict.docs).toBe("Some docs");
		expect(dict.abi_version).toBe(ABI_VERSION);
		expect(dict.long_running).toBe(false);
		expect(dict.pins).toHaveLength(1);
	});

	it("omits null optional fields", () => {
		const nd = new NodeDefinition("n", "N", "d", "c");
		const dict = nd.toDict();
		expect(dict.icon).toBeUndefined();
		expect(dict.scores).toBeUndefined();
		expect(dict.long_running).toBeUndefined();
		expect(dict.docs).toBeUndefined();
	});

	it("serializes to JSON", () => {
		const nd = new NodeDefinition("n", "N", "d", "c");
		const json = nd.toJSON();
		const parsed = JSON.parse(json);
		expect(parsed.name).toBe("n");
	});
});

describe("PackageNodes", () => {
	it("collects multiple node definitions", () => {
		const pkg = new PackageNodes();
		pkg.addNode(new NodeDefinition("a", "A", "d1", "c1"));
		pkg.addNode(new NodeDefinition("b", "B", "d2", "c2"));
		expect(pkg.nodes).toHaveLength(2);
	});

	it("serializes to dict array", () => {
		const pkg = new PackageNodes();
		pkg.addNode(new NodeDefinition("a", "A", "d", "c"));
		const arr = pkg.toDict();
		expect(arr).toHaveLength(1);
		expect(arr[0].name).toBe("a");
	});

	it("serializes to JSON array", () => {
		const pkg = new PackageNodes();
		pkg.addNode(new NodeDefinition("a", "A", "d", "c"));
		const parsed = JSON.parse(pkg.toJSON());
		expect(Array.isArray(parsed)).toBe(true);
	});
});

describe("ExecutionInput", () => {
	it("creates from dict with all fields", () => {
		const input = ExecutionInput.fromDict({
			inputs: { x: 42, y: "hello" },
			node_id: "node-1",
			run_id: "run-1",
			app_id: "app-1",
			board_id: "board-1",
			user_id: "user-1",
			stream_state: true,
			log_level: LogLevel.DEBUG,
			node_name: "TestNode",
		});

		expect(input.inputs).toEqual({ x: 42, y: "hello" });
		expect(input.nodeId).toBe("node-1");
		expect(input.runId).toBe("run-1");
		expect(input.appId).toBe("app-1");
		expect(input.boardId).toBe("board-1");
		expect(input.userId).toBe("user-1");
		expect(input.streamState).toBe(true);
		expect(input.logLevel).toBe(LogLevel.DEBUG);
		expect(input.nodeName).toBe("TestNode");
	});

	it("creates from dict with defaults", () => {
		const input = ExecutionInput.fromDict({});
		expect(input.inputs).toEqual({});
		expect(input.nodeId).toBe("");
		expect(input.streamState).toBe(false);
		expect(input.logLevel).toBe(LogLevel.INFO);
	});

	it("creates from JSON string", () => {
		const json = JSON.stringify({ inputs: { a: 1 }, node_id: "n1" });
		const input = ExecutionInput.fromJSON(json);
		expect(input.inputs.a).toBe(1);
		expect(input.nodeId).toBe("n1");
	});
});

describe("ExecutionResult", () => {
	it("creates ok result", () => {
		const result = ExecutionResult.ok();
		expect(result.error).toBeNull();
		expect(result.outputs).toEqual({});
		expect(result.activateExec).toEqual([]);
	});

	it("creates fail result", () => {
		const result = ExecutionResult.fail("something broke");
		expect(result.error).toBe("something broke");
	});

	it("sets outputs", () => {
		const result = ExecutionResult.ok();
		result.setOutput("x", 42);
		result.setOutput("y", "hello");
		expect(result.outputs.x).toBe(42);
		expect(result.outputs.y).toBe("hello");
	});

	it("activates exec pins", () => {
		const result = ExecutionResult.ok();
		result.exec("exec_out");
		result.exec("branch_a");
		expect(result.activateExec).toEqual(["exec_out", "branch_a"]);
	});

	it("sets pending", () => {
		const result = ExecutionResult.ok();
		result.setPending(true);
		expect(result.pending).toBe(true);
	});

	it("serializes to dict", () => {
		const result = ExecutionResult.ok();
		result.setOutput("val", 1);
		result.exec("out");
		const dict = result.toDict();
		expect(dict.outputs).toEqual({ val: 1 });
		expect(dict.activate_exec).toEqual(["out"]);
		expect(dict.error).toBeUndefined();
		expect(dict.pending).toBeUndefined();
	});

	it("includes error in dict when present", () => {
		const result = ExecutionResult.fail("err");
		const dict = result.toDict();
		expect(dict.error).toBe("err");
	});

	it("serializes to JSON", () => {
		const result = ExecutionResult.ok().setOutput("x", 1).exec("out");
		const parsed = JSON.parse(result.toJSON());
		expect(parsed.outputs.x).toBe(1);
		expect(parsed.activate_exec).toContain("out");
	});
});

describe("LogLevel", () => {
	it("has correct ordering", () => {
		expect(LogLevel.DEBUG).toBeLessThan(LogLevel.INFO);
		expect(LogLevel.INFO).toBeLessThan(LogLevel.WARN);
		expect(LogLevel.WARN).toBeLessThan(LogLevel.ERROR);
		expect(LogLevel.ERROR).toBeLessThan(LogLevel.FATAL);
	});
});

describe("ABI_VERSION", () => {
	it("is a positive integer", () => {
		expect(ABI_VERSION).toBe(1);
	});
});
