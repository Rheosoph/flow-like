import { describe, expect, test } from "bun:test";
import {
	NODE_CHROME_HEIGHT,
	NODE_WIDTH,
	PIN_MARGIN_TOP,
	PIN_ROW_HEIGHT,
} from "../lib/flow-layout/measure";
import type { ResolvedGhost } from "../lib/node-suggestions/resolve";
import type { SuggestionResult } from "../lib/node-suggestions/types";
import type { IBoard } from "../lib/schema/flow/board";
import { ICommandType } from "../lib/schema/flow/board/commands/generic-command";
import type { INode, IPin } from "../lib/schema/flow/node";
import { IPinType, IValueType, IVariableType } from "../lib/schema/flow/node";
import {
	GHOST_GAP,
	type GhostAnchorGeometry,
	type GhostSuggestionEngine,
	buildGhostAcceptBatch,
	findGhostAnchor,
	ghostKeyAction,
	ghostPlacement,
	isGhostKeyTarget,
	suggestForAnchor,
} from "./use-node-suggestion-ghost";

const pin = (
	id: string,
	name: string,
	pinType: IPinType,
	index: number,
	dataType = IVariableType.Execution,
): IPin => ({
	id,
	name,
	friendly_name: name,
	description: "",
	pin_type: pinType,
	data_type: dataType,
	value_type: IValueType.Normal,
	index,
	connected_to: [],
	depends_on: [],
});

const node = (
	id: string,
	name: string,
	pins: IPin[],
	extra: Partial<INode> = {},
): INode => ({
	id,
	name,
	friendly_name: name,
	description: "",
	category: "Test",
	pins: Object.fromEntries(pins.map((entry) => [entry.id, entry])),
	...extra,
});

const template = node("template", "log_info", [
	pin("t-exec-in", "exec_in", IPinType.Input, 1),
	pin("t-message", "message", IPinType.Input, 2, IVariableType.String),
	pin("t-exec-out", "exec_out", IPinType.Output, 1),
]);

const ghost = (targetPinName: string): ResolvedGhost => ({
	node: template,
	targetPinName,
	probability: 0.6,
	confidence: 0.5,
});

const board = (nodes: INode[]): IBoard =>
	({
		nodes: Object.fromEntries(nodes.map((entry) => [entry.id, entry])),
		layers: {},
		refs: {},
	}) as unknown as IBoard;

describe("findGhostAnchor", () => {
	const root = node("a", "delay", [
		pin("a-out", "exec_out", IPinType.Output, 1),
	]);
	const nested = node("b", "delay", [], { layer: "layer-1" });
	const blankLayer = node("c", "delay", [], { layer: "" });
	const reroute = node("r", "reroute", []);
	const current = board([root, nested, blankLayer, reroute]);

	test("needs exactly one selected, existing, non-reroute node", () => {
		expect(findGhostAnchor(current, ["a"], undefined)).toBe(root);
		expect(findGhostAnchor(current, [], undefined)).toBeUndefined();
		expect(findGhostAnchor(current, ["a", "c"], undefined)).toBeUndefined();
		expect(findGhostAnchor(current, ["missing"], undefined)).toBeUndefined();
		expect(findGhostAnchor(current, ["r"], undefined)).toBeUndefined();
	});

	test("only anchors in the current layer, treating empty layers alike", () => {
		expect(findGhostAnchor(current, ["b"], undefined)).toBeUndefined();
		expect(findGhostAnchor(current, ["b"], "layer-1")).toBe(nested);
		expect(findGhostAnchor(current, ["a"], "layer-1")).toBeUndefined();
		expect(findGhostAnchor(current, ["c"], undefined)).toBe(blankLayer);
		expect(findGhostAnchor(current, ["a"], "")).toBe(root);
	});
});

describe("ghostPlacement", () => {
	const anchor: GhostAnchorGeometry = {
		measured: { width: 180, height: 60 },
		internals: {
			positionAbsolute: { x: 100, y: 200 },
			handleBounds: {
				source: [{ id: "a-out", x: 174, y: 22, width: 12, height: 12 }],
				target: null,
			},
		},
	};

	test("puts the ghost right of the anchor with its target row level with the pin", () => {
		const placement = ghostPlacement(anchor, "a-out", ghost("message"));
		if (!placement) throw new Error("expected a placement");
		expect(placement.source).toEqual({ x: 280, y: 228 });
		expect(placement.origin.x).toBe(100 + 180 + GHOST_GAP);
		expect(placement.targetOffset).toBe(PIN_MARGIN_TOP + PIN_ROW_HEIGHT);
		expect(placement.target).toEqual({ x: placement.origin.x, y: 228 });
		expect(placement.origin.y).toBe(228 - PIN_MARGIN_TOP - PIN_ROW_HEIGHT);
		expect(placement.size).toEqual({
			width: NODE_WIDTH,
			height: 2 * PIN_ROW_HEIGHT + NODE_CHROME_HEIGHT,
		});
	});

	test("first input row sits at the pin margin", () => {
		expect(
			ghostPlacement(anchor, "a-out", ghost("exec_in"))?.targetOffset,
		).toBe(PIN_MARGIN_TOP);
	});

	test("falls back to the default width and gives up without the handle", () => {
		const unmeasured = { ...anchor, measured: {} };
		expect(
			ghostPlacement(unmeasured, "a-out", ghost("exec_in"))?.origin.x,
		).toBe(100 + NODE_WIDTH + GHOST_GAP);
		expect(ghostPlacement(anchor, "other", ghost("exec_in"))).toBeUndefined();
	});
});

describe("ghostKeyAction", () => {
	const key = (value: string, modifiers: Partial<KeyboardEvent> = {}) => ({
		key: value,
		altKey: false,
		ctrlKey: false,
		metaKey: false,
		shiftKey: false,
		...modifiers,
	});

	test("maps the ghost keys", () => {
		expect(ghostKeyAction(key("Tab"))).toBe("accept");
		expect(ghostKeyAction(key("Escape"))).toBe("dismiss");
		expect(ghostKeyAction(key("ArrowDown", { altKey: true }))).toBe("next");
		expect(ghostKeyAction(key("ArrowUp", { altKey: true }))).toBe("previous");
	});

	test("leaves modified and unrelated keys alone", () => {
		expect(ghostKeyAction(key("Tab", { shiftKey: true }))).toBeUndefined();
		expect(ghostKeyAction(key("Tab", { ctrlKey: true }))).toBeUndefined();
		expect(ghostKeyAction(key("Tab", { altKey: true }))).toBeUndefined();
		expect(ghostKeyAction(key("Escape", { metaKey: true }))).toBeUndefined();
		expect(ghostKeyAction(key("ArrowDown"))).toBeUndefined();
		expect(
			ghostKeyAction(key("ArrowDown", { altKey: true, shiftKey: true })),
		).toBeUndefined();
		expect(ghostKeyAction(key("Enter"))).toBeUndefined();
	});
});

describe("isGhostKeyTarget", () => {
	const body = { closest: () => null };
	const documentElement = { closest: () => null };
	const ownerDocument = { body, documentElement };
	Object.assign(body, { ownerDocument });
	const element = (
		ancestors: string[],
		extra: { isContentEditable?: boolean } = {},
	) => ({
		ownerDocument,
		...extra,
		closest: (selector: string) =>
			selector
				.split(",")
				.map((part) => part.trim())
				.some((part) => ancestors.some((ancestor) => part.startsWith(ancestor)))
				? {}
				: null,
	});

	test("accepts the page body and the board canvas", () => {
		expect(isGhostKeyTarget(body)).toBe(true);
		expect(isGhostKeyTarget(null)).toBe(true);
		expect(isGhostKeyTarget(element([".react-flow"]))).toBe(true);
	});

	test("rejects fields, editors, dialogs and anything outside the canvas", () => {
		expect(isGhostKeyTarget(element([".react-flow", "input"]))).toBe(false);
		expect(isGhostKeyTarget(element([".react-flow", "textarea"]))).toBe(false);
		expect(
			isGhostKeyTarget(element([".react-flow"], { isContentEditable: true })),
		).toBe(false);
		expect(isGhostKeyTarget(element([".monaco-editor"]))).toBe(false);
		expect(isGhostKeyTarget(element(['[role="dialog"]', ".react-flow"]))).toBe(
			false,
		);
		expect(isGhostKeyTarget(element(["aside"]))).toBe(false);
	});
});

describe("buildGhostAcceptBatch", () => {
	test("adds a clone and wires it in one batch without touching the template", () => {
		const before = structuredClone(template);
		const batch = buildGhostAcceptBatch({
			ghost: ghost("message"),
			anchorNodeId: "anchor",
			anchorPinId: "anchor-text",
			position: { x: 360, y: 185 },
			currentLayer: "layer-1",
		});
		expect(template).toEqual(before);
		expect(batch?.commands).toHaveLength(2);
		const [add, connect] = batch?.commands ?? [];
		expect(add.command_type).toBe(ICommandType.AddNode);
		expect(add.current_layer).toBe("layer-1");
		const added = add.node as INode;
		expect(added.id).toBe(batch?.nodeId as string);
		expect(added.id).not.toBe(template.id);
		expect(added.coordinates).toEqual([360, 185, 0]);
		const message = Object.values(added.pins).find(
			(entry) => entry.name === "message",
		);
		expect(message?.id).not.toBe("t-message");
		expect(connect).toMatchObject({
			command_type: ICommandType.ConnectPin,
			from_node: "anchor",
			from_pin: "anchor-text",
			to_node: added.id,
			to_pin: message?.id,
		});
	});

	test("gives up when the target pin is not an input of the node", () => {
		expect(
			buildGhostAcceptBatch({
				ghost: ghost("exec_out"),
				anchorNodeId: "anchor",
				anchorPinId: "anchor-out",
				position: { x: 0, y: 0 },
			}),
		).toBeUndefined();
	});
});

describe("suggestForAnchor", () => {
	const source = node("src", "source", [
		pin("src-exec", "exec_out", IPinType.Output, 1),
		pin("src-text", "text", IPinType.Output, 2, IVariableType.String),
	]);
	const current = board([source]);
	const catalogByName = new Map([[template.name, template]]);
	const byPin: Record<string, SuggestionResult> = {
		exec_out: {
			candidates: [{ type: "log_info", pin: "exec_in", probability: 0.8 }],
			pUnconnected: 0.2,
			confidence: 0.64,
		},
		text: {
			candidates: [{ type: "log_info", pin: "message", probability: 0.9 }],
			pUnconnected: 0.1,
			confidence: 0.81,
		},
	};

	const fakeEngine = (results: Record<string, SuggestionResult>) => {
		let inFlight = 0;
		const calls: string[] = [];
		const engine: GhostSuggestionEngine = {
			async suggest(context) {
				inFlight++;
				if (inFlight > 1) throw new Error("suggest called concurrently");
				calls.push(context.srcPin);
				await Promise.resolve();
				inFlight--;
				return results[context.srcPin];
			},
		};
		return { engine, calls };
	};

	const run = (
		engine: GhostSuggestionEngine,
		overrides: { dismissed?: Set<string>; isStale?: () => boolean } = {},
	) =>
		suggestForAnchor({
			engine,
			board: current,
			anchorNodeId: "src",
			catalogByName,
			dismissed: overrides.dismissed ?? new Set(),
			isStale: overrides.isStale ?? (() => false),
		});

	test("asks pin by pin and shows the most confident one", async () => {
		const { engine, calls } = fakeEngine(byPin);
		const state = await run(engine);
		expect(calls).toEqual(["exec_out", "text"]);
		expect(state?.pinId).toBe("src-text");
		expect(state?.ghosts[0].targetPinName).toBe("message");
		expect(state?.index).toBe(0);
	});

	test("skips dismissed pins and ghosts below the confidence floor", async () => {
		const dismissed = await run(fakeEngine(byPin).engine, {
			dismissed: new Set(["src\u0000src-text"]),
		});
		expect(dismissed?.pinId).toBe("src-exec");

		const unsure = await run(
			fakeEngine({
				exec_out: { ...byPin.exec_out, pUnconnected: 0.9 },
			}).engine,
		);
		expect(unsure).toBeUndefined();
	});

	test("a stale run stops asking", async () => {
		const { engine, calls } = fakeEngine(byPin);
		let stale = false;
		const state = await run(engine, {
			isStale: () => {
				const current = stale;
				stale = true;
				return current;
			},
		});
		expect(calls).toEqual(["exec_out"]);
		expect(state).toBeUndefined();
	});
});
