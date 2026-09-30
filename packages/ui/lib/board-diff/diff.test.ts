import { describe, expect, it } from "bun:test";
import {
	type IBoard,
	type IComment,
	ICommentType,
	IExecutionMode,
	IExecutionStage,
	type ILayer,
	ILayerType,
	ILogLevel,
	type INode,
	type IPin,
	IPinType,
	IValueType,
	type IVariable,
	IVariableType,
} from "../schema/flow/board";
import { convertJsonToUint8Array } from "../uint8";
import { diffBoards } from "./diff";

type PinSpec = Partial<IPin> & { name: string };

function pin(nodeId: string, spec: PinSpec, index: number): IPin {
	const { value, ...rest } = spec as PinSpec & { value?: unknown };
	return {
		id: `${nodeId}.${spec.name}`,
		connected_to: [],
		depends_on: [],
		description: "",
		friendly_name: spec.name,
		index,
		pin_type: IPinType.Input,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		default_value: value === undefined ? null : convertJsonToUint8Array(value),
		...rest,
	};
}

function node(
	id: string,
	options: {
		friendly?: string;
		at?: [number, number];
		layer?: string;
		pins?: (PinSpec & { value?: unknown })[];
	} = {},
): INode {
	const pins = [
		{
			name: "exec_in",
			data_type: IVariableType.Execution,
			friendly_name: "Exec",
		},
		{
			name: "exec_out",
			data_type: IVariableType.Execution,
			pin_type: IPinType.Output,
			friendly_name: "Exec",
		},
		...(options.pins ?? []),
	].map((spec, index) => pin(id, spec, index));
	return {
		id,
		name: `test_${id}`,
		friendly_name: options.friendly ?? id,
		category: "Test",
		description: "",
		coordinates: options.at ?? [0, 0],
		layer: options.layer ?? null,
		pins: Object.fromEntries(pins.map((p) => [p.id, p])),
	};
}

function variable(
	id: string,
	value: unknown,
	extra: Partial<IVariable> = {},
): IVariable {
	return {
		id,
		name: id,
		data_type: IVariableType.Float,
		value_type: IValueType.Normal,
		default_value: convertJsonToUint8Array(value),
		editable: true,
		exposed: false,
		secret: false,
		...extra,
	};
}

function layer(id: string, extra: Partial<ILayer> = {}): ILayer {
	return {
		id,
		name: id,
		type: ILayerType.Collapsed,
		coordinates: [0, 0],
		comments: {},
		nodes: {},
		pins: {},
		variables: {},
		...extra,
	};
}

function board(parts: {
	nodes?: INode[];
	layers?: ILayer[];
	variables?: IVariable[];
	comments?: IComment[];
}): IBoard {
	return {
		id: "board",
		name: "Invoice Intake",
		description: "",
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		execution_mode: IExecutionMode.Hybrid,
		log_level: ILogLevel.Info,
		stage: IExecutionStage.Dev,
		version: [0, 0, 1],
		viewport: [0, 0, 1],
		page_ids: [],
		refs: {},
		nodes: Object.fromEntries((parts.nodes ?? []).map((n) => [n.id, n])),
		layers: Object.fromEntries((parts.layers ?? []).map((l) => [l.id, l])),
		variables: Object.fromEntries(
			(parts.variables ?? []).map((v) => [v.id, v]),
		),
		comments: Object.fromEntries((parts.comments ?? []).map((c) => [c.id, c])),
	};
}

function connect(from: INode, fromPin: string, to: INode, toPin: string) {
	from.pins[`${from.id}.${fromPin}`].connected_to.push(`${to.id}.${toPin}`);
	to.pins[`${to.id}.${toPin}`].depends_on.push(`${from.id}.${fromPin}`);
}

function clone<T>(value: T): T {
	return structuredClone(value);
}

describe("diffBoards", () => {
	it("reports nothing for identical boards", () => {
		const a = board({
			nodes: [node("ext", { pins: [{ name: "model", value: "gpt-4o-mini" }] })],
		});
		const diff = diffBoards(a, clone(a));
		expect(diff.changes).toEqual([]);
		expect(diff.logicCount).toBe(0);
	});

	it("reports an added node with the wire it runs after", () => {
		const ext = node("ext", { friendly: "AI Extractor" });
		const base = board({ nodes: [ext] });
		const head = clone(base);
		const check = node("check", { friendly: "Valid IBAN?", at: [300, 0] });
		head.nodes.check = check;
		connect(head.nodes.ext, "exec_out", check, "exec_in");

		const diff = diffBoards(base, head);
		expect(diff.counts.added).toBe(1);
		const change = diff.byKey.get("node:check");
		expect(change?.status).toBe("added");
		expect(change?.details).toContainEqual({
			kind: "wire",
			field: "exec_in",
			label: "Exec",
			role: "runs-after",
			after: "AI Extractor",
		});
		expect(diff.edgeStatus.get("ext.exec_out-check.exec_in")).toBe("added");
		expect(diff.itemStatus.get("check")).toBe("added");
	});

	it("reports changed pin values with readable before and after", () => {
		const base = board({
			nodes: [
				node("ext", {
					pins: [
						{ name: "model", value: "gpt-4o-mini" },
						{ name: "temperature", value: 0.2, data_type: IVariableType.Float },
					],
				}),
			],
		});
		const head = board({
			nodes: [
				node("ext", {
					pins: [
						{ name: "model", value: "claude-sonnet-5" },
						{ name: "temperature", value: 0, data_type: IVariableType.Float },
					],
				}),
			],
		});
		const change = diffBoards(base, head).byKey.get("node:ext");
		expect(change?.status).toBe("changed");
		expect(change?.details.map((d) => [d.label, d.before, d.after])).toEqual([
			["model", "gpt-4o-mini", "claude-sonnet-5"],
			["temperature", "0.2", "0"],
		]);
	});

	it("reports a rewired input on the node it feeds", () => {
		const dup = node("dup", {
			friendly: "Is duplicate?",
			pins: [
				{
					name: "false",
					data_type: IVariableType.Execution,
					pin_type: IPinType.Output,
					friendly_name: "False",
				},
			],
		});
		const iban = node("iban", {
			friendly: "Valid IBAN?",
			pins: [
				{
					name: "true",
					data_type: IVariableType.Execution,
					pin_type: IPinType.Output,
					friendly_name: "True",
				},
			],
		});
		const ins = node("ins", { friendly: "Insert Rows" });
		const base = board({ nodes: [dup, iban, ins] });
		connect(base.nodes.dup, "false", base.nodes.ins, "exec_in");
		const head = board({ nodes: [clone(dup), clone(iban), clone(ins)] });
		head.nodes.dup.pins["dup.false"].connected_to = [];
		head.nodes.ins.pins["ins.exec_in"].depends_on = [];
		connect(head.nodes.iban, "true", head.nodes.ins, "exec_in");

		const diff = diffBoards(base, head);
		const change = diff.byKey.get("node:ins");
		expect(change?.details).toEqual([
			{
				kind: "wire",
				field: "exec_in",
				label: "Exec",
				role: "runs-after",
				before: "Is duplicate? → False",
				after: "Valid IBAN? → True",
			},
		]);
		expect(diff.edgeStatus.get("dup.false-ins.exec_in")).toBe("removed");
		expect(diff.edgeStatus.get("iban.true-ins.exec_in")).toBe("added");
		expect(diff.changes.map((c) => c.key)).toEqual(["node:ins"]);
	});

	it("keeps layout-only moves out of the logic count", () => {
		const base = board({ nodes: [node("gt", { at: [100, 100] })] });
		const head = board({ nodes: [node("gt", { at: [400, 100] })] });
		const diff = diffBoards(base, head);
		expect(diff.changes[0]).toMatchObject({
			status: "moved",
			move: { from: [100, 100], to: [400, 100] },
		});
		expect(diff.logicCount).toBe(0);
		expect(diff.counts.moved).toBe(1);
	});

	it("ignores pins that appear without a value or a wire", () => {
		const base = board({ nodes: [node("ext")] });
		const head = board({
			nodes: [node("ext", { pins: [{ name: "dynamic_input" }] })],
		});
		expect(diffBoards(base, head).changes).toEqual([]);
	});

	it("withholds values of sensitive pins and secret variables", () => {
		const options = { sensitive: true };
		const base = board({
			nodes: [
				node("http", { pins: [{ name: "token", value: "abc", options }] }),
			],
			variables: [
				variable("apiKey", "old", {
					secret: true,
					data_type: IVariableType.String,
				}),
			],
		});
		const head = board({
			nodes: [
				node("http", { pins: [{ name: "token", value: "xyz", options }] }),
			],
			variables: [
				variable("apiKey", "new", {
					secret: true,
					data_type: IVariableType.String,
				}),
			],
		});
		const diff = diffBoards(base, head);
		for (const change of diff.changes) {
			const values = change.details.filter((d) => d.kind === "value");
			expect(values.length).toBe(1);
			expect(values[0].masked).toBe(true);
			expect(values[0].before).not.toContain("old");
			expect(values[0].before).not.toContain("abc");
			expect(values[0].after).not.toContain("new");
			expect(values[0].after).not.toContain("xyz");
		}
	});

	it("describes an added pure node by what it feeds", () => {
		const branch = node("brI", {
			friendly: "Valid IBAN?",
			pins: [{ name: "condition", data_type: IVariableType.Boolean }],
		});
		const base = board({ nodes: [branch] });
		const head = board({ nodes: [clone(branch)] });
		const iban: INode = {
			...node("iban", { friendly: "Regex Match" }),
			pins: {},
		};
		iban.pins["iban.matches"] = pin(
			"iban",
			{
				name: "matches",
				pin_type: IPinType.Output,
				data_type: IVariableType.Boolean,
			},
			0,
		);
		head.nodes.iban = iban;
		connect(iban, "matches", head.nodes.brI, "condition");

		const change = diffBoards(base, head).byKey.get("node:iban");
		expect(change?.details).toContainEqual({
			kind: "wire",
			field: "_feeds",
			label: "Feeds",
			role: "feeds",
			after: "Valid IBAN? · condition",
		});
	});

	it("folds the nodes of an added layer into the layer change", () => {
		const base = board({ nodes: [node("ev")] });
		const head = board({
			nodes: [
				node("ev"),
				node("a", { layer: "fn" }),
				node("b", { layer: "fn" }),
			],
			layers: [
				layer("fn", { name: "normalizeVendor", type: ILayerType.Function }),
			],
		});
		const diff = diffBoards(base, head);
		expect(diff.changes.map((c) => c.key)).toEqual(["layer:fn"]);
		expect(diff.changes[0].details).toContainEqual({
			kind: "setting",
			field: "nodes",
			label: "Nodes",
			after: "2",
		});
		expect(diff.itemStatus.get("a")).toBe("added");
	});

	it("marks every ancestor layer of a nested change", () => {
		const outer = layer("outer");
		const inner = layer("inner", { parent_id: "outer" });
		const base = board({
			nodes: [node("n", { layer: "inner", pins: [{ name: "x", value: 1 }] })],
			layers: [outer, inner],
		});
		const head = board({
			nodes: [node("n", { layer: "inner", pins: [{ name: "x", value: 2 }] })],
			layers: [clone(outer), clone(inner)],
		});
		const diff = diffBoards(base, head);
		expect([...diff.layersWithChanges].sort()).toEqual(["inner", "outer"]);
		expect(diff.changes[0].layerId).toBe("inner");
	});

	it("points a variable change at the nodes that read it", () => {
		const getter = node("thr", {
			pins: [{ name: "var_ref", value: "approvalThreshold" }],
		});
		const base = board({
			nodes: [getter],
			variables: [variable("approvalThreshold", 5000)],
		});
		const head = board({
			nodes: [clone(getter)],
			variables: [variable("approvalThreshold", 2500)],
		});
		const change = diffBoards(base, head).byKey.get(
			"variable:approvalThreshold",
		);
		expect(change?.details).toContainEqual({
			kind: "value",
			field: "default_value",
			label: "Default value",
			before: "5000",
			after: "2500",
			masked: false,
		});
		expect(change?.focusIds).toEqual(["thr"]);
	});

	it("reports comments that were edited or moved", () => {
		const note: IComment = {
			id: "c1",
			content: "Retry twice before giving up",
			comment_type: ICommentType.Text,
			coordinates: [0, 0, 0],
			timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		};
		const edited = {
			...clone(note),
			content: "Retry three times before giving up",
		};
		const moved = { ...clone(note), coordinates: [50, 0, 0] };
		expect(
			diffBoards(board({ comments: [note] }), board({ comments: [edited] }))
				.changes[0].status,
		).toBe("changed");
		expect(
			diffBoards(board({ comments: [note] }), board({ comments: [moved] }))
				.changes[0].status,
		).toBe("moved");
	});
});
