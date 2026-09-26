import { expect, test } from "bun:test";
import { base64url } from "./crypto";
import {
	type ManagementCall,
	digestText,
	readTelemetryChunks,
} from "./telemetry";

test("group chunks pin the first publication and verify complete exact bytes", async () => {
	const text = JSON.stringify({ data: "secret".repeat(1100) });
	const bytes = new TextEncoder().encode(text);
	const digest = await digestText(text);
	const seen: number[] = [];
	const call: ManagementCall = async (command) => {
		seen.push(Number(command.sequence));
		const offset = Number(command.offset);
		return {
			operation_id: "id",
			state: "completed",
			result: {
				available: true,
				offset,
				total: bytes.length,
				digest,
				sequence: 9,
				latest: 10,
				chunk: base64url(bytes.slice(offset, offset + 4096)),
			},
		};
	};
	expect(
		await readTelemetryChunks(call, { type: "telemetry_read", sequence: 0 }),
	).toEqual({ text, sequence: 9, latest: 10 });
	expect(seen).toEqual([0, 9]);
});
test("group chunks reject length, digest, sequence, eviction and truncation changes", async () => {
	const bytes = new TextEncoder().encode("a".repeat(5000));
	const digest = await digestText(new TextDecoder().decode(bytes));
	for (const altered of [
		{ digest: "b".repeat(43) },
		{ total: 4999 },
		{ sequence: 10 },
		{ available: false },
		{ chunk: "" },
		{ chunk: base64url(new Uint8Array(904)) },
	]) {
		const call: ManagementCall = async (command) => {
			const offset = Number(command.offset);
			return {
				operation_id: "id",
				state: "completed",
				result: {
					available: true,
					offset,
					total: bytes.length,
					digest,
					sequence: 9,
					latest: 10,
					chunk: base64url(bytes.slice(offset, offset + 4096)),
					...(offset ? altered : {}),
				},
			};
		};
		await expect(readTelemetryChunks(call, { sequence: 9 })).rejects.toThrow();
	}
	const oversized: ManagementCall = async () => ({
		operation_id: "id",
		state: "completed",
		result: {
			available: true,
			offset: 0,
			total: 3 * 1024 * 1024,
			digest,
			chunk: "AA",
		},
	});
	await expect(readTelemetryChunks(oversized, {})).rejects.toThrow("chunk");
});
