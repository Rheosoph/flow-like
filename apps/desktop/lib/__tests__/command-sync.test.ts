import type { IGenericCommand } from "@flow-like/flow-like-ui";
import { describe, expect, test } from "vitest";
import {
	MAX_COMMAND_SYNC_BODY_BYTES,
	chunkCommandsForSync,
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
