import { describe, expect, test } from "bun:test";
import type { INode } from "../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../lib/schema/flow/pin";
import type { IVariable } from "../../lib/schema/flow/variable";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import {
	PALETTE_MAX_RESULTS,
	type PaletteEntry,
	buildDropIndex,
	buildPaletteEntries,
	buildPaletteModel,
	buildPaletteRows,
	dropFit,
	findCategory,
	matchesWords,
	pinSignature,
	resolveRecents,
	searchPalette,
} from "./node-palette-model";

type PinSpec = [
	name: string,
	type: IPinType,
	data: IVariableType,
	value?: IValueType,
];

function pin(
	node: string,
	index: number,
	[name, pinType, dataType, valueType]: PinSpec,
): IPin {
	return {
		id: `${node}-${name}`,
		name,
		friendly_name: name,
		pin_type: pinType,
		data_type: dataType,
		value_type: valueType ?? IValueType.Normal,
		depends_on: [],
		connected_to: [],
		index,
		description: "",
		schema: null,
	};
}

function node(
	name: string,
	friendlyName: string,
	category: string,
	pins: PinSpec[] = [],
	extra: Partial<INode> = {},
): INode {
	return {
		id: `id-${name}`,
		name,
		friendly_name: friendlyName,
		category,
		description: "",
		icon: "/flow/icons/workflow.svg",
		pins: Object.fromEntries(
			pins.map((spec, i) => {
				const p = pin(name, i + 1, spec);
				return [p.id, p];
			}),
		),
		...extra,
	} as INode;
}

const { Input, Output } = IPinType;
const {
	Execution,
	String: Str,
	Integer,
	Generic,
	Boolean: Bool,
} = IVariableType;

const CATALOG: INode[] = [
	node("control_branch", "Branch", "Control", [
		["exec_in", Input, Execution],
		["condition", Input, Bool],
		["true", Output, Execution],
		["false", Output, Execution],
	]),
	node("string_trim", "Trim String", "Utils/String", [
		["string", Input, Str],
		["trimmed", Output, Str],
	]),
	node("string_length", "String Length", "Utils/String", [
		["string", Input, Str],
		["length", Output, Integer],
	]),
	node("string_upper", "String Upper", "Utils/String", [
		["string", Input, Str],
		["upper", Output, Str],
	]),
	node("int_add", "Add", "Math/Int", [
		["a", Input, Integer],
		["b", Input, Integer],
		["sum", Output, Integer],
	]),
	node("log_info", "Print Info", "Logging", [
		["exec_in", Input, Execution],
		["message", Input, Generic],
		["exec_out", Output, Execution],
	]),
	node("neq", "!=", "Math/Compare", [
		["a", Input, Generic],
		["b", Input, Generic],
		["result", Output, Bool],
	]),
	node(
		"variable_get",
		"Get Variable",
		"Variable",
		[
			["var_ref", Input, Str],
			["value_ref", Output, Generic],
		],
		{ icon: "/flow/icons/variable.svg" },
	),
	node(
		"variable_set",
		"Set Variable",
		"Variable",
		[
			["exec_in", Input, Execution],
			["var_ref", Input, Str],
			["value_in", Input, Generic],
			["exec_out", Output, Execution],
			["value_ref", Output, Generic],
		],
		{ icon: "/flow/icons/variable.svg" },
	),
	node("control_call_reference", "Call Reference", "Control/Call", [
		["exec_in", Input, Execution],
		["fn_ref", Input, Str],
	]),
	node("control_call_function", "Call Function", "Control/Functions", [
		["function_layer_id", Input, Str],
	]),
	node("events_simple", "Simple Event", "Events", [
		["exec_out", Output, Execution],
	]),
];

const VARIABLE: IVariable = {
	id: "var1",
	name: "report_date",
	data_type: IVariableType.Date,
	value_type: IValueType.Normal,
	editable: true,
	exposed: false,
	secret: false,
	schema: null,
};

const LABELS = {
	call: (name: string) => `Call ${name}`,
	get: (name: string) => `Get ${name}`,
	set: (name: string) => `Set ${name}`,
};

const NO_INPUTS = { startNodes: [], variables: [], functionLayers: [] };

const INPUTS = {
	startNodes: [node("on_ticket", "On Ticket Created", "Events")],
	variables: [VARIABLE],
	functionLayers: [{ id: "layer1", name: "Summarize Ticket" } as never],
};

const model = buildPaletteModel(buildPaletteEntries(CATALOG, INPUTS, LABELS));
const keys = (entries: PaletteEntry[]) => entries.map((entry) => entry.key);
const pinNamed = (n: INode, name: string) =>
	Object.values(n.pins).find((p) => p.name === name);
function nodeFor(key: string): INode {
	const entry = model.byKey.get(key);
	if (!entry) throw new Error(`no palette entry for ${key}`);
	return entry.node;
}
const defaultOf = (n: INode, pinName: string) =>
	parseUint8ArrayToJson(pinNamed(n, pinName)?.default_value);

describe("buildPaletteEntries", () => {
	test("binds generated variable entries to the variable and its type", () => {
		const set = nodeFor("set:var1");
		expect(set.friendly_name).toBe("Set report_date");
		expect(model.byKey.get("set:var1")?.path).toEqual(["Variables", "Set"]);
		expect(defaultOf(set, "var_ref")).toBe("var1");
		expect(pinNamed(set, "value_in")?.data_type).toBe(IVariableType.Date);
		expect(pinNamed(nodeFor("get:var1"), "value_ref")?.data_type).toBe(
			IVariableType.Date,
		);
	});

	test("gives each generated call its own key and reference", () => {
		const call = nodeFor("call:id-on_ticket");
		expect(call.friendly_name).toBe("Call On Ticket Created");
		expect(call.id).toBe("call:id-on_ticket");
		expect(defaultOf(call, "fn_ref")).toBe("id-on_ticket");
		expect(model.byKey.get("fn:layer1")?.path).toEqual(["Functions", "Call"]);
		expect(defaultOf(nodeFor("fn:layer1"), "function_layer_id")).toBe("layer1");
	});

	test("skips a node the catalog repeats under the same id", () => {
		const entries = buildPaletteEntries(
			[...CATALOG, node("control_branch", "Branch Copy", "Other")],
			NO_INPUTS,
			LABELS,
		);
		const branches = entries.filter(
			(entry) => entry.node.name === "control_branch",
		);
		expect(branches.map((entry) => entry.node.friendly_name)).toEqual([
			"Branch",
		]);
	});

	test("keeps ontology bindings that reuse their prototype's name", () => {
		const prototype = node("ontology_query_objects", "Query Objects", "Data");
		const binding = node(
			"ontology_query_objects",
			"List Customer",
			"Data Studio/CRM/Objects",
			[],
			{ id: "ontology_binding_crm_object_customer" },
		);
		const entries = buildPaletteEntries(
			[prototype, binding],
			NO_INPUTS,
			LABELS,
		);
		expect(keys(entries).sort()).toEqual([
			"ontology_query_objects",
			"ontology_query_objects#ontology_binding_crm_object_customer",
		]);
	});

	test("keeps same-named WASM nodes from different packages", () => {
		const wasmNode = (packageId: string) =>
			node("get_weather", "Get Weather", "Web", [], {
				id: "get_weather",
				wasm: { package_id: packageId, permissions: [] },
			} as Partial<INode>);
		const entries = buildPaletteEntries(
			[wasmNode("pkg.a"), wasmNode("pkg.b"), wasmNode("pkg.a")],
			NO_INPUTS,
			LABELS,
		);
		expect(keys(entries).sort()).toEqual([
			"get_weather",
			"get_weather#pkg.b:get_weather",
		]);
	});
});

describe("buildPaletteTree", () => {
	test("pins the core categories first, then sorts the rest", () => {
		expect(model.tree.children.map((c) => c.name)).toEqual([
			"Control",
			"Variables",
			"Events",
			"Functions",
			"Logging",
			"Math",
			"Utils",
			"Variable",
		]);
	});

	test("counts every descendant and picks the most common icon", () => {
		const math = findCategory(model.tree, ["Math"]);
		expect(math?.count).toBe(2);
		expect(findCategory(model.tree, ["Variables"])?.icon).toBe(
			"/flow/icons/variable.svg",
		);
		expect(model.tree.count).toBe(model.entries.length);
	});
});

describe("searchPalette", () => {
	const search = (
		query: string,
		scope: string[] = [],
		recents: string[] = [],
	) => keys(searchPalette(model, query, { scope, recents }));

	test("ranks an exact name above prefix and word matches", () => {
		expect(search("add")[0]).toBe("int_add");
		expect(search("string")[0]).toBe("string_length");
	});

	test("finds operator names", () => {
		expect(search("!=")).toContain("neq");
	});

	test("limits results to the open category", () => {
		expect(search("string", ["Math"])).toEqual([]);
		expect(search("trim", ["Utils"])).toEqual(["string_trim"]);
	});

	test("lifts recently placed nodes within the same match tier", () => {
		const [first, second] = search("string").filter((key) =>
			["string_length", "string_upper"].includes(key),
		);
		const boosted = search("string", [], [second]);
		expect(boosted.indexOf(second)).toBeLessThan(boosted.indexOf(first));
	});

	test("never lifts a recent word match above a name prefix match", () => {
		const boosted = search("string", [], ["string_trim"]);
		expect(boosted.indexOf("string_length")).toBeLessThan(
			boosted.indexOf("string_trim"),
		);
	});
});

describe("dropFit", () => {
	const formatted = pin("format_datetime", 3, ["formatted", Output, Str]);
	const refs = {};

	test("lands a String wire on the first matching input", () => {
		expect(dropFit(nodeFor("string_trim"), formatted, refs)?.target?.name).toBe(
			"string",
		);
		expect(dropFit(nodeFor("log_info"), formatted, refs)?.target?.name).toBe(
			"message",
		);
	});

	test("rejects nodes without a compatible input", () => {
		expect(dropFit(nodeFor("int_add"), formatted, refs)).toBe(undefined);
		expect(dropFit(nodeFor("events_simple"), formatted, refs)).toBe(undefined);
	});

	test("matches function reference handles by capability", () => {
		const refOut = { ...formatted, id: "ref_out_abc" };
		const referencable = node("tool", "Tool", "AI", [], {
			fn_refs: {
				fn_refs: [],
				can_reference_fns: false,
				can_be_referenced_by_fns: true,
			},
		});
		expect(dropFit(referencable, refOut, refs)).toEqual({});
		expect(dropFit(nodeFor("string_trim"), refOut, refs)).toBe(undefined);
	});

	test("indexes only the entries a wire can land on", () => {
		const index = buildDropIndex(model.entries, formatted, refs);
		expect(index.has("string_trim")).toBe(true);
		expect(index.has("int_add")).toBe(false);
	});

	test("lands on the pin a manual connect would accept, not the first same-typed one", () => {
		const shape = (field: string, type: string) =>
			JSON.stringify({
				type: "object",
				properties: { [field]: { type } },
				required: [field],
			});
		const withSchema = (p: IPin, schema: string): IPin => ({ ...p, schema });
		const dropped = withSchema(
			pin("source", 1, ["record", Output, IVariableType.Struct]),
			shape("x", "string"),
		);
		const target = node("merge", "Merge", "Data", [
			["other", Input, IVariableType.Struct],
			["record", Input, IVariableType.Struct],
		]);
		for (const p of Object.values(target.pins)) {
			p.schema =
				p.name === "other" ? shape("y", "number") : shape("x", "string");
		}
		expect(dropFit(target, dropped, refs)?.target?.name).toBe("record");
	});

	test("never rewires a preset node's bound reference", () => {
		const title: IVariable = {
			...VARIABLE,
			id: "var2",
			name: "title",
			data_type: IVariableType.String,
		};
		const bound = buildPaletteModel(
			buildPaletteEntries(
				CATALOG,
				{ ...INPUTS, variables: [VARIABLE, title] },
				LABELS,
			),
		);
		const setTitle = bound.byKey.get("set:var2")?.node;
		const setDate = bound.byKey.get("set:var1")?.node;
		const call = bound.byKey.get("call:id-on_ticket")?.node;
		if (!setTitle || !setDate || !call) throw new Error("missing entries");
		expect(dropFit(setTitle, formatted, refs)?.target?.name).toBe("value_in");
		expect(dropFit(setDate, formatted, refs)).toBe(undefined);
		expect(dropFit(call, formatted, refs)).toBe(undefined);
	});

	test("leaves a node unconnected rather than wiring a pin a manual connect rejects", () => {
		const shape = (field: string, type: string) =>
			JSON.stringify({
				type: "object",
				properties: { [field]: { type } },
				required: [field],
			});
		const dropped: IPin = {
			...pin("source", 1, ["record", Output, IVariableType.Struct]),
			schema: shape("x", "string"),
		};
		const target = node("sink", "Sink", "Data", [
			["other", Input, IVariableType.Struct],
		]);
		for (const p of Object.values(target.pins)) p.schema = shape("y", "number");
		expect(dropFit(target, dropped, refs)).toBe(undefined);
	});

	test("does not promise a connection the placement would not make", () => {
		const execOut = pin("event", 1, ["exec_out", Output, Execution]);
		const reroute = node("reroute", "Reroute", "Control", [
			["route_in", Input, Generic],
			["route_out", Output, Generic],
		]);
		expect(dropFit(reroute, execOut, refs)).toBe(undefined);
	});
});

describe("buildPaletteRows", () => {
	const base = {
		query: "",
		scope: [],
		tree: model.tree,
		recents: resolveRecents(model, [
			"string_trim",
			"missing",
			"control_branch",
		]),
		results: [],
		actions: [],
		dropActive: false,
		canWiden: false,
	};
	const shape = (rows: ReturnType<typeof buildPaletteRows>) =>
		rows.map((row) => row.id);

	test("opens on recent nodes, then every top category", () => {
		const rows = shape(buildPaletteRows(base));
		expect(rows.slice(0, 3)).toEqual([
			"group:recent",
			"recent:string_trim",
			"recent:control_branch",
		]);
		expect(rows[3]).toBe("group:browse");
		expect(rows).toContain("category:Utils");
	});

	test("labels recents as matches when a wire was dropped", () => {
		expect(shape(buildPaletteRows({ ...base, dropActive: true }))[0]).toBe(
			"group:recentMatches",
		);
	});

	test("lists subcategories, a divider, then nodes inside a category", () => {
		const control = shape(buildPaletteRows({ ...base, scope: ["Control"] }));
		expect(control).toEqual([
			"category:Control/Call",
			"category:Control/Functions",
			"divider",
			"node:control_branch",
		]);
	});

	test("puts matching actions above node results and caps the list", () => {
		const many = Array.from(
			{ length: PALETTE_MAX_RESULTS + 5 },
			() => model.entries[0],
		);
		const rows = buildPaletteRows({
			...base,
			query: "comm",
			actions: ["comment"],
			results: many,
		});
		expect(rows[0]).toMatchObject({ kind: "group", group: "actions" });
		expect(rows[1]).toMatchObject({ kind: "action", action: "comment" });
		expect(rows[2]).toMatchObject({
			kind: "group",
			group: "nodes",
			count: PALETTE_MAX_RESULTS + 5,
		});
		expect(rows.at(-1)).toMatchObject({
			kind: "overflow",
			total: PALETTE_MAX_RESULTS + 5,
		});
	});

	test("lists uncategorized nodes at the root after the categories", () => {
		const loose = buildPaletteModel(
			buildPaletteEntries(
				[...CATALOG, node("loose", "Loose Node", "")],
				NO_INPUTS,
				LABELS,
			),
		);
		const rows = shape(buildPaletteRows({ ...base, tree: loose.tree }));
		expect(rows.slice(-2)).toEqual(["divider", "node:loose"]);
	});

	test("explains a category the compatible filter emptied", () => {
		expect(
			shape(buildPaletteRows({ ...base, scope: ["Math", "Int"] })),
		).not.toContain("empty");
		expect(
			shape(
				buildPaletteRows({ ...base, scope: ["Nowhere"], dropActive: true }),
			),
		).toEqual(["empty", "widen:scope", "widen:compatible"]);
	});

	test("offers ways out of an empty search", () => {
		const rows = shape(
			buildPaletteRows({
				...base,
				query: "zzz",
				scope: ["Utils"],
				dropActive: true,
				canWiden: true,
			}),
		);
		expect(rows).toEqual(["empty", "widen:scope", "widen:compatible"]);
	});
});

describe("labels", () => {
	test("summarizes pins with execution marks and value types", () => {
		expect(pinSignature(nodeFor("log_info"))).toBe("▸, Generic → ▸");
		const split = node("split", "Split", "Utils", [
			["s", Input, Str],
			["parts", Output, Str, IValueType.Array],
		]);
		expect(pinSignature(split)).toBe("String → String[]");
	});

	test("matches action words by prefix", () => {
		expect(matchesWords("var pin", "Variable from pin")).toBe(true);
		expect(matchesWords("note", "Comment", "note frame")).toBe(true);
		expect(matchesWords("xyz", "Comment")).toBe(false);
		expect(matchesWords("  ", "Comment")).toBe(false);
	});
});
