import { describe, expect, test } from "bun:test";
import { ICommandType } from "../schema/flow/board/commands/generic-command";
import {
	MAX_DRAG_NODES,
	editVerbs,
	sanitizeChatTyping,
	sanitizeDrag,
	sanitizeLastEdit,
	sanitizeLastRun,
	sanitizePing,
	sanitizeSummon,
	wireLayerPath,
} from "./presence-signals";

const ID = "nodeanchor0000000001";
const OTHER = "nodeanchor0000000002";

describe("presence signals (rule 2: ids, bounded numbers, closed enums)", () => {
	test("drag keeps id-shaped nodes with finite coordinates, deduped and capped", () => {
		const payload = sanitizeDrag({
			nodes: [
				{ id: ID, x: 10.5, y: -20 },
				{ id: ID, x: 1, y: 1 },
				{ id: "const leaked = 1", x: 0, y: 0 },
				{ id: OTHER, x: Number.NaN, y: 0 },
				...Array.from({ length: 100 }, (_, i) => ({
					id: `dragnode0000${String(i).padStart(8, "0")}`,
					x: i,
					y: i,
				})),
			],
			ts: 5,
		});
		expect(payload?.nodes[0]).toEqual({ id: ID, x: 10.5, y: -20 });
		expect(payload?.nodes.length).toBe(MAX_DRAG_NODES);
		expect(payload?.nodes.some((node) => node.id === OTHER)).toBe(false);
		expect(sanitizeDrag({ nodes: [], ts: 1 })).toBeUndefined();
		expect(sanitizeDrag("nodes")).toBeUndefined();
	});

	test("ping needs a layer path and a sequence; emoji outside the set is dropped", () => {
		expect(
			sanitizePing({
				x: 1,
				y: 2,
				layerPath: "root",
				seq: 3,
				ts: 4,
				emoji: "🎉",
			}),
		).toEqual({ x: 1, y: 2, layerPath: "root", seq: 3, ts: 4, emoji: "🎉" });
		expect(
			sanitizePing({ x: 1, y: 2, layerPath: "", seq: 3, ts: 4, emoji: "text" }),
		).toEqual({ x: 1, y: 2, layerPath: "root", seq: 3, ts: 4 });
		expect(
			sanitizePing({ x: 1, y: 2, layerPath: "Layer One", seq: 3, ts: 4 }),
		).toBeUndefined();
		expect(
			sanitizePing({ x: 1, y: 2, layerPath: "root", ts: 4 }),
		).toBeUndefined();
	});

	test("layer paths are root or slash-joined ids", () => {
		expect(wireLayerPath("root")).toBe("root");
		expect(wireLayerPath("")).toBe("root");
		expect(wireLayerPath(`${ID}/${OTHER}`)).toBe(`${ID}/${OTHER}`);
		expect(wireLayerPath(`${ID}/not an id`)).toBeUndefined();
		expect(wireLayerPath(42)).toBeUndefined();
	});

	test("summon clamps zoom and needs every field", () => {
		expect(
			sanitizeSummon({ x: 0, y: 0, zoom: 50, layerPath: ID, seq: 1, ts: 1 }),
		).toEqual({ x: 0, y: 0, zoom: 10, layerPath: ID, seq: 1, ts: 1 });
		expect(
			sanitizeSummon({ x: 0, y: 0, layerPath: ID, seq: 1, ts: 1 }),
		).toBeUndefined();
	});

	test("last edit carries only known command kinds and a count", () => {
		expect(
			sanitizeLastEdit({
				kinds: [ICommandType.MoveNode, "DropTable", ICommandType.MoveNode],
				count: 3,
				ts: 9,
			}),
		).toEqual({ kinds: [ICommandType.MoveNode], count: 3, ts: 9 });
		expect(
			sanitizeLastEdit({ kinds: ["free text"], count: 1, ts: 1 }),
		).toBeUndefined();
		expect(
			sanitizeLastEdit({ kinds: [ICommandType.AddNode], count: 0, ts: 1 }),
		).toBeUndefined();
		expect(
			editVerbs([
				ICommandType.MoveNode,
				ICommandType.MoveToLayer,
				ICommandType.ConnectPin,
			]),
		).toEqual(["moved", "connected"]);
	});

	test("last run is an id, a closed status and a count", () => {
		expect(
			sanitizeLastRun({ runId: ID, status: "error", executed: 12.4, ts: 1 }),
		).toEqual({ runId: ID, status: "error", executed: 12, ts: 1 });
		expect(
			sanitizeLastRun({ runId: ID, status: "crashed", executed: 1, ts: 1 }),
		).toBeUndefined();
		expect(
			sanitizeLastRun({ runId: "run 12", status: "ok", executed: 1, ts: 1 }),
		).toBeUndefined();
	});

	test("chat typing is a timestamp and nothing else", () => {
		expect(sanitizeChatTyping({ ts: 7, text: "hel" })).toEqual({ ts: 7 });
		expect(sanitizeChatTyping({ ts: 0 })).toBeUndefined();
		expect(sanitizeChatTyping("typing")).toBeUndefined();
	});
});
