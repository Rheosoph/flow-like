import { describe, expect, test } from "bun:test";
import type { IBoard } from "../schema/flow/board";
import { IPinType, IValueType, IVariableType } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";
import {
	ROOT_SEGMENT,
	applyBoardSync,
	catalogByName,
	nodeSegment,
} from "./apply";
import { BoardSyncClient } from "./client";
import type {
	IBoardSyncManifest,
	IBoardSyncRequest,
	IBoardSyncResponse,
	ISyncNode,
} from "./types";

const manifest = (
	segments: Record<string, string>,
	rest: Partial<IBoardSyncManifest> = {},
): IBoardSyncManifest => ({
	meta: "m1",
	variables: "v1",
	comments: "c1",
	layers: { l1: "layer-l1" },
	segments,
	...rest,
});

const meta = (updated = 1): IBoardSyncResponse["meta"] => ({
	id: "b",
	name: "Board",
	description: "",
	viewport: [0, 0, 1],
	version: [0, 0, 1],
	stage: "Dev" as never,
	log_level: 1 as never,
	execution_mode: "Hybrid" as never,
	page_ids: [],
	hash: 7,
	created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	updated_at: { secs_since_epoch: updated, nanos_since_epoch: 0 },
});

const wireNode = (
	id: string,
	layer: string | null,
	extra: Partial<ISyncNode> = {},
): ISyncNode => ({
	id,
	name: "demo",
	version: 3,
	layer,
	friendly_name: "Demo",
	description: "d",
	category: "Test",
	pins: {
		[`${id}-p`]: {
			id: `${id}-p`,
			name: "value",
			index: 0,
			pin_type: IPinType.Input,
			data_type: IVariableType.String,
			value_type: IValueType.Normal,
			friendly_name: "Value",
			description: "",
			default_value: "ImhpIg==",
		},
	},
	...extra,
});

const catalogNode: INode = {
	id: "cat",
	name: "demo",
	version: 3,
	friendly_name: "Demo (catalog)",
	description: "from catalog",
	category: "Test",
	icon: "/i.svg",
	pins: {
		p: {
			id: "p",
			name: "value",
			index: 0,
			pin_type: IPinType.Input,
			data_type: IVariableType.String,
			value_type: IValueType.Normal,
			friendly_name: "Value (catalog)",
			description: "cat",
			connected_to: [],
			depends_on: [],
		},
	},
};

const layerDef = (id: string, name = id): IBoard["layers"][string] =>
	({
		id,
		name,
		type: "Collapsed",
		nodes: {},
		variables: {},
		comments: {},
		pins: {},
		coordinates: [0, 0, 0],
	}) as unknown as IBoard["layers"][string];

const fullResponse = (): IBoardSyncResponse => ({
	manifest: manifest({ [ROOT_SEGMENT]: "s-root", l1: "s-l1" }),
	meta: meta(),
	variables: {},
	comments: {},
	layers: { l1: layerDef("l1") },
	refs: { k: "{}" },
	segments: {
		[ROOT_SEGMENT]: { hash: "s-root", nodes: { a: wireNode("a", null) } },
		l1: { hash: "s-l1", nodes: { b: wireNode("b", "l1") } },
	},
});

describe("nodeSegment", () => {
	test("empty and missing layer are the root segment", () => {
		expect(nodeSegment({})).toBe(ROOT_SEGMENT);
		expect(nodeSegment({ layer: "" })).toBe(ROOT_SEGMENT);
		expect(nodeSegment({ layer: null })).toBe(ROOT_SEGMENT);
		expect(nodeSegment({ layer: "x" })).toBe("x");
	});
});

describe("applyBoardSync", () => {
	test("assembles a full board from a full response", () => {
		const { board, changed } = applyBoardSync(
			undefined,
			fullResponse(),
			undefined,
		);
		expect(changed).toBe(true);
		expect(Object.keys(board.nodes).sort()).toEqual(["a", "b"]);
		expect(board.refs).toEqual({ k: "{}" });
		expect(Object.keys(board.layers)).toEqual(["l1"]);
		expect(board.nodes.a.pins["a-p"].default_value).toEqual([
			0x22, 0x68, 0x69, 0x22,
		]);
		expect(board.nodes.a.pins["a-p"].connected_to).toEqual([]);
	});

	test("an empty diff returns the previous board by identity", () => {
		const first = applyBoardSync(undefined, fullResponse(), undefined).board;
		const { board, changed } = applyBoardSync(
			first,
			{ manifest: manifest({ [ROOT_SEGMENT]: "s-root", l1: "s-l1" }) },
			undefined,
		);
		expect(changed).toBe(false);
		expect(board).toBe(first);
	});

	test("a changed segment replaces its node set wholesale and leaves others by identity", () => {
		const first = applyBoardSync(undefined, fullResponse(), undefined).board;
		const { board } = applyBoardSync(
			first,
			{
				manifest: manifest(
					{ [ROOT_SEGMENT]: "s-root", l1: "s-l1-2" },
					{ meta: "m2" },
				),
				meta: meta(2),
				segments: {
					l1: { hash: "s-l1-2", nodes: { c: wireNode("c", "l1") } },
				},
			},
			undefined,
		);
		expect(Object.keys(board.nodes).sort()).toEqual(["a", "c"]);
		expect(board.nodes.a).toBe(first.nodes.a);
		expect(board.updated_at.secs_since_epoch).toBe(2);
	});

	test("a node that moved layers is removed from the segment it left", () => {
		const first = applyBoardSync(undefined, fullResponse(), undefined).board;
		// `a` moved from root to l1: server resends l1 (grew) and drops root (now empty).
		const { board } = applyBoardSync(
			first,
			{
				manifest: manifest({ l1: "s-l1-3" }, { meta: "m3" }),
				meta: meta(3),
				segments: {
					l1: {
						hash: "s-l1-3",
						nodes: { a: wireNode("a", "l1"), b: wireNode("b", "l1") },
					},
				},
				dropped_segments: [ROOT_SEGMENT],
			},
			undefined,
		);
		expect(Object.keys(board.nodes).sort()).toEqual(["a", "b"]);
		expect(board.nodes.a.layer).toBe("l1");
	});

	test("hydrates lean nodes from the catalog and flags nodes it cannot", () => {
		const catalog = catalogByName([catalogNode]);
		const lean = wireNode("a", null, {
			h: true,
			description: undefined,
			category: undefined,
			pins: {
				"a-p": { id: "a-p", name: "value", index: 0, default_value: null },
			},
		});
		const stale = wireNode("z", null, {
			h: true,
			version: 4,
			pins: { "z-p": { id: "z-p", name: "value", index: 0 } },
		});
		const { board, unhydratable } = applyBoardSync(
			undefined,
			{
				...fullResponse(),
				segments: {
					[ROOT_SEGMENT]: { hash: "h", nodes: { a: lean, z: stale } },
				},
			},
			catalog,
		);
		expect(board.nodes.a.friendly_name).toBe("Demo");
		expect(board.nodes.a.description).toBe("from catalog");
		expect(board.nodes.a.icon).toBe("/i.svg");
		expect(board.nodes.a.pins["a-p"].data_type).toBe(IVariableType.String);
		expect(board.nodes.a.pins["a-p"].friendly_name).toBe("Value (catalog)");
		expect(unhydratable.has(ROOT_SEGMENT)).toBe(true);
	});

	test("refs are upserted, never replaced", () => {
		const first = applyBoardSync(undefined, fullResponse(), undefined).board;
		const { board } = applyBoardSync(
			first,
			{
				manifest: manifest(
					{ [ROOT_SEGMENT]: "s-root-2", l1: "s-l1" },
					{ meta: "m2" },
				),
				meta: meta(2),
				refs: { k2: '{"new":true}' },
				segments: {
					[ROOT_SEGMENT]: {
						hash: "s-root-2",
						nodes: { a: wireNode("a", null) },
					},
				},
			},
			undefined,
		);
		expect(board.refs).toEqual({ k: "{}", k2: '{"new":true}' });
	});

	test("layer definitions merge by id and honour drops", () => {
		const first = applyBoardSync(undefined, fullResponse(), undefined).board;
		const { board } = applyBoardSync(
			first,
			{
				manifest: manifest(
					{ [ROOT_SEGMENT]: "s-root", l1: "s-l1" },
					{ meta: "m2", layers: { l1: "layer-l1", l2: "layer-l2" } },
				),
				meta: meta(2),
				layers: { l2: layerDef("l2", "Second") },
			},
			undefined,
		);
		expect(Object.keys(board.layers).sort()).toEqual(["l1", "l2"]);
		expect(board.layers.l1).toBe(first.layers.l1);

		const { board: after } = applyBoardSync(
			board,
			{
				manifest: manifest(
					{ [ROOT_SEGMENT]: "s-root", l1: "s-l1" },
					{ meta: "m3", layers: { l2: "layer-l2-renamed" } },
				),
				meta: meta(3),
				layers: { l2: layerDef("l2", "Renamed") },
				dropped_layers: ["l1"],
			},
			undefined,
		);
		expect(Object.keys(after.layers)).toEqual(["l2"]);
		expect(after.layers.l2.name).toBe("Renamed");
	});

	test("a full node never consults the catalog", () => {
		const catalog = catalogByName([catalogNode]);
		const { board, unhydratable } = applyBoardSync(
			undefined,
			fullResponse(),
			catalog,
		);
		expect(board.nodes.a.friendly_name).toBe("Demo");
		expect(unhydratable.size).toBe(0);
	});
});

describe("BoardSyncClient", () => {
	test("first sync sends an empty request, later syncs echo the manifest", async () => {
		const client = new BoardSyncClient();
		const requests: IBoardSyncRequest[] = [];
		const transport = async (request: IBoardSyncRequest) => {
			requests.push(request);
			return requests.length === 1
				? fullResponse()
				: { manifest: fullResponse().manifest };
		};
		const first = await client.sync("app", "b", undefined, transport);
		const second = await client.sync("app", "b", undefined, transport);
		expect(requests[0].segments).toBeUndefined();
		expect(requests[1].segments).toEqual({
			[ROOT_SEGMENT]: "s-root",
			l1: "s-l1",
		});
		expect(requests[1].meta).toBe("m1");
		expect(second).toBe(first);
	});

	test("retries stale-catalog segments without hydration", async () => {
		const client = new BoardSyncClient();
		client.setCatalog("app", [{ ...catalogNode, version: 2 }]);
		const requests: IBoardSyncRequest[] = [];
		const transport = async (request: IBoardSyncRequest) => {
			requests.push(request);
			if (requests.length === 1) {
				const response = fullResponse();
				const segments = response.segments ?? {};
				segments[ROOT_SEGMENT].nodes.a = wireNode("a", null, {
					h: true,
					description: undefined,
					pins: { "a-p": { id: "a-p", name: "value", index: 0 } },
				});
				return response;
			}
			expect(request.hydrate).toBe(false);
			expect(request.segments?.[ROOT_SEGMENT]).toBeUndefined();
			expect(request.segments?.l1).toBe("s-l1");
			return {
				manifest: fullResponse().manifest,
				segments: {
					[ROOT_SEGMENT]: { hash: "s-root", nodes: { a: wireNode("a", null) } },
				},
			};
		};
		const board = await client.sync("app", "b", undefined, transport);
		expect(requests).toHaveLength(2);
		expect(requests[0].hydrate).toBe(true);
		expect(board.nodes.a.description).toBe("d");
	});

	test("concurrent syncs share one round trip", async () => {
		const client = new BoardSyncClient();
		let calls = 0;
		const transport = async () => {
			calls++;
			return fullResponse();
		};
		const [a, b] = await Promise.all([
			client.sync("app", "b", undefined, transport),
			client.sync("app", "b", undefined, transport),
		]);
		expect(calls).toBe(1);
		expect(a).toBe(b);
	});
});

// Type-level guard: the assembled board is assignable to IBoard.
const _assign: IBoard = applyBoardSync(
	undefined,
	fullResponse(),
	undefined,
).board;
void _assign;
