import { describe, expect, it } from "bun:test";
import type { IBoard } from "../schema/flow/board";
import { serializeBoard } from "./board-json";
import {
	canCompress,
	fingerprint,
	packSnapshot,
	unpackSnapshot,
} from "./snapshot-codec";
import { createSnapshotEngine } from "./snapshot-engine";

function bigBoard(nodes: number, marker = ""): IBoard {
	return {
		id: "b",
		name: "Big",
		description: "",
		version: [1, 0, 0],
		layers: { l1: { id: "l1", name: "fn", nodes: {}, pins: {} } },
		variables: { v: { id: "v", name: "v", default_value: [49] } },
		comments: {},
		skipped: undefined,
		nodes: Object.fromEntries(
			Array.from({ length: nodes }, (_, i) => [
				`n${i}`,
				{
					id: `n${i}`,
					friendly_name: `Node ${i}${i === 3 ? marker : ""}`,
					coordinates: [i, i, 0],
					pins: { p: { id: `n${i}p`, default_value: [1, 2, 3] } },
				},
			]),
		),
	} as unknown as IBoard;
}

describe("serializeBoard", () => {
	it("produces exactly JSON.stringify's text", async () => {
		const board = bigBoard(50);
		expect(await serializeBoard(board)).toBe(JSON.stringify(board));
	});

	it("keeps producing the same text when every entry yields", async () => {
		const board = bigBoard(200);
		expect(await serializeBoard(board, 0)).toBe(JSON.stringify(board));
	});
});

describe("snapshot codec", () => {
	it("round-trips a board and compresses it", async () => {
		const json = JSON.stringify(bigBoard(300));
		const packed = await packSnapshot(json);
		expect(await unpackSnapshot(packed)).toBe(json);
		if (canCompress()) {
			expect(packed.encoding).toBe("gzip-base64");
			expect(packed.payload.length).toBeLessThan(json.length / 4);
		}
	});

	it("reads snapshots stored as plain JSON on unload", async () => {
		expect(await unpackSnapshot({ payload: "{}", encoding: "json" })).toBe(
			"{}",
		);
	});

	it("fingerprints differ for different text and match for equal text", () => {
		const a = JSON.stringify(bigBoard(20));
		expect(fingerprint(a)).toBe(fingerprint(`${a}`));
		expect(fingerprint(a)).not.toBe(
			fingerprint(JSON.stringify(bigBoard(20, "!"))),
		);
	});
});

describe("snapshot engine", () => {
	it("skips packing a board identical to the stored one", async () => {
		const engine = createSnapshotEngine();
		const json = JSON.stringify(bigBoard(10));
		const stored = await packSnapshot(json);
		await engine.handle({ type: "open", key: "k", stored, current: json });
		const same = await engine.handle({ type: "pack", key: "k", current: json });
		expect(same).toEqual({ type: "pack" });

		const changed = JSON.stringify(bigBoard(10, "!"));
		const next = await engine.handle({
			type: "pack",
			key: "k",
			current: changed,
		});
		expect(next.type === "pack" && next.packed).toBeTruthy();
	});

	it("packs again after a failed write was unmarked", async () => {
		const engine = createSnapshotEngine();
		const json = JSON.stringify(bigBoard(5));
		await engine.handle({ type: "pack", key: "k", current: json });
		await engine.handle({ type: "unmark", key: "k" });
		const again = await engine.handle({
			type: "pack",
			key: "k",
			current: json,
		});
		expect(again.type === "pack" && again.packed).toBeTruthy();
	});
});
