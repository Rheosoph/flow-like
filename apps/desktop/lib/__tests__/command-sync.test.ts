import type { IGenericCommand } from "@flow-like/flow-like-ui";
import { describe, expect, test } from "vitest";
import {
	MAX_COMMAND_SYNC_BODY_BYTES,
	MAX_UNDO_REDO_SYNC_BODY_BYTES,
	chunkCommandsForSync,
	evaluateBoardLineage,
	systemTimeToNanos,
} from "../../components/tauri-provider/command-sync";

const commandOfSize = (id: number, bytes: number): IGenericCommand =>
	({
		command_type: "UpsertNode",
		node: { id: `node-${id}`, payload: "x".repeat(bytes) },
	}) as unknown as IGenericCommand;

describe("chunkCommandsForSync", () => {
	test("small batches stay in a single chunk", () => {
		const commands = Array.from({ length: 50 }, (_, i) =>
			commandOfSize(i, 100),
		);
		const chunks = chunkCommandsForSync(commands);
		expect(chunks).toHaveLength(1);
		expect(chunks[0]).toHaveLength(50);
	});

	test("large batches split below the body cap, preserving order", () => {
		const perCommand = 100 * 1024;
		const commands = Array.from({ length: 40 }, (_, i) =>
			commandOfSize(i, perCommand),
		);
		const chunks = chunkCommandsForSync(commands);

		expect(chunks.length).toBeGreaterThan(1);
		for (const chunk of chunks) {
			const size = chunk.reduce(
				(sum, command) => sum + JSON.stringify(command).length,
				0,
			);
			expect(size).toBeLessThanOrEqual(MAX_COMMAND_SYNC_BODY_BYTES);
		}
		const flattened = chunks.flat();
		expect(flattened).toHaveLength(commands.length);
		flattened.forEach((command, index) => {
			expect(command).toBe(commands[index]);
		});
	});

	test("a single oversized command gets its own chunk instead of being dropped", () => {
		const commands = [
			commandOfSize(0, 100),
			commandOfSize(1, MAX_COMMAND_SYNC_BODY_BYTES + 1024),
			commandOfSize(2, 100),
		];
		const chunks = chunkCommandsForSync(commands);
		expect(chunks.flat()).toHaveLength(3);
		expect(chunks.some((chunk) => chunk.length === 1)).toBe(true);
	});

	test("empty input produces no chunks", () => {
		expect(chunkCommandsForSync([])).toHaveLength(0);
	});
});

describe("systemTimeToNanos", () => {
	test("combines seconds and nanoseconds", () => {
		expect(
			systemTimeToNanos({ secs_since_epoch: 2, nanos_since_epoch: 5 }),
		).toBe(2_000_000_005);
	});

	test("missing or partial timestamps collapse to zero-based values", () => {
		expect(systemTimeToNanos(undefined)).toBe(0);
		expect(systemTimeToNanos(null)).toBe(0);
		expect(systemTimeToNanos({})).toBe(0);
		expect(systemTimeToNanos({ secs_since_epoch: 1 })).toBe(1_000_000_000);
	});
});

describe("evaluateBoardLineage", () => {
	const cached = 5_000_000_000;

	test("remote strictly newer than the cached lineage applies", () => {
		const decision = evaluateBoardLineage(cached + 1, cached);
		expect(decision.apply).toBe(true);
		expect(decision.refusalReason).toBeUndefined();
	});

	test("remote older than the cached lineage is refused", () => {
		const decision = evaluateBoardLineage(cached - 1, cached);
		expect(decision.apply).toBe(false);
		expect(decision.refusalReason).toContain("older");
	});

	test("remote equal to the cached lineage is refused", () => {
		const decision = evaluateBoardLineage(cached, cached);
		expect(decision.apply).toBe(false);
		expect(decision.refusalReason).toContain("equals");
	});

	test("missing cache leaves the guard inert", () => {
		expect(evaluateBoardLineage(cached, undefined).apply).toBe(true);
		expect(evaluateBoardLineage(cached, null).apply).toBe(true);
		expect(evaluateBoardLineage(cached, 0).apply).toBe(true);
		expect(evaluateBoardLineage(0, undefined).apply).toBe(true);
	});

	test("remote without a timestamp is refused once a lineage exists", () => {
		const decision = evaluateBoardLineage(0, cached);
		expect(decision.apply).toBe(false);
		expect(decision.refusalReason).toContain("no updated_at");
	});
});

describe("sync body limits", () => {
	test("undo/redo client cap stays below the server's 16MB route limit", () => {
		expect(MAX_UNDO_REDO_SYNC_BODY_BYTES).toBeLessThan(16 * 1024 * 1024);
		expect(MAX_UNDO_REDO_SYNC_BODY_BYTES).toBeGreaterThan(
			MAX_COMMAND_SYNC_BODY_BYTES,
		);
	});
});
