import { expect, test } from "bun:test";
import { base64url } from "./crypto";
import {
	type ManagementCall,
	acknowledgeGroupTelemetryThrough,
	applyTelemetryPolicy,
	digestText,
	readTelemetryChunks,
	readerPosition,
} from "./telemetry";
import type { BrowserMlsEndpoint, ManagementResponse } from "./types";

/** A device outbox that publishes on its own and acknowledges like an older agent. */
function outboxDevice(options: { prefixAcknowledge?: boolean } = {}) {
	const outbox: { sequence: number; envelope_jws: string }[] = [];
	let latest = 0;
	const publish = () => {
		latest++;
		outbox.push({ sequence: latest, envelope_jws: `envelope-${latest}` });
	};
	const commands: Record<string, unknown>[] = [];
	const rejected = (
		result: Record<string, unknown> = {},
	): ManagementResponse => ({
		operation_id: "id",
		state: "rejected",
		result,
	});
	const completed = (result: Record<string, unknown>): ManagementResponse => ({
		operation_id: "id",
		state: "completed",
		result,
	});
	let beforePolicy: () => ManagementResponse | undefined = () => undefined;
	const call: ManagementCall = async (command) => {
		commands.push(command);
		if (command.type === "telemetry_read") {
			const entry =
				command.sequence === 0
					? outbox[0]
					: outbox.find((row) => row.sequence === command.sequence);
			if (!entry) return completed({ available: false, latest });
			const text = JSON.stringify({
				policy_jws: "policy",
				envelope_jws: entry.envelope_jws,
			});
			return completed({
				available: true,
				offset: 0,
				total: text.length,
				digest: await digestText(text),
				sequence: entry.sequence,
				latest,
				chunk: base64url(new TextEncoder().encode(text)),
			});
		}
		if (command.type === "telemetry_acknowledge") {
			const index = outbox.findIndex(
				(row) => row.sequence === command.sequence,
			);
			const entry = outbox[index];
			if (
				!entry ||
				(await digestText(entry.envelope_jws)) !== command.envelope_digest ||
				(!options.prefixAcknowledge && index !== 0)
			)
				return rejected();
			outbox.splice(0, index + 1);
			return completed({});
		}
		if (command.type === "telemetry_policy") {
			const early = beforePolicy();
			if (early) return early;
			if (command.sequence !== latest + 1) return rejected();
			latest++;
			return completed({ sequence: latest });
		}
		throw new Error(`unexpected ${String(command.type)}`);
	};
	return {
		call,
		commands,
		outbox,
		publish,
		rejected,
		before: (next: () => ManagementResponse | undefined) => {
			beforePolicy = next;
		},
	};
}

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

test("group reads show a coded device rejection and keep the legacy message for older agents", async () => {
	const rejecting =
		(result: Record<string, unknown>): ManagementCall =>
		async () => ({ operation_id: "id", state: "rejected", result });
	await expect(
		readTelemetryChunks(
			rejecting({
				error: "The current management grant does not allow this command.",
				code: "unauthorized",
				retryable: false,
			}),
			{ type: "telemetry_read", sequence: 0 },
		),
	).rejects.toThrow("does not allow this command");
	await expect(
		readTelemetryChunks(rejecting({}), { type: "telemetry_read", sequence: 0 }),
	).rejects.toThrow("could not return the requested group telemetry");
});

test("reader positions must be plain JSON objects from the wasm glue", () => {
	const endpoint = (value: unknown) =>
		({ position: () => value }) as unknown as BrowserMlsEndpoint;
	expect(
		readerPosition(endpoint({ joined: true, retired: false, sequence: 4 })),
	).toEqual({ joined: true, retired: false, sequence: 4 });
	expect(() =>
		readerPosition(
			endpoint(
				new Map<string, unknown>([
					["joined", true],
					["retired", false],
					["sequence", 4],
				]),
			),
		),
	).toThrow("unreadable");
});

test("a membership change re-reads the sequence after the publisher moves ahead", async () => {
	const device = outboxDevice();
	device.publish();
	let raced = false;
	device.before(() => {
		if (!raced) {
			raced = true;
			device.publish();
		}
		return undefined;
	});
	const sequence = await applyTelemetryPolicy(
		device.call,
		{ scope: "device", policy_jws: "signed", key_packages: [] },
		async () => {},
	);
	expect(sequence).toBe(3);
	expect(
		device.commands
			.filter((command) => command.type === "telemetry_policy")
			.map((command) => command.sequence),
	).toEqual([2, 3]);
});

test("a definitive rejection code stops membership retries with the device reason", async () => {
	const device = outboxDevice();
	device.before(() =>
		device.rejected({
			error: "Telemetry readers endpoint-a no longer hold a Metrics grant",
			code: "invalid",
			retryable: false,
		}),
	);
	await expect(
		applyTelemetryPolicy(
			device.call,
			{ scope: "device", policy_jws: "signed", key_packages: [] },
			async () => {},
		),
	).rejects.toThrow("endpoint-a no longer hold a Metrics grant");
	expect(
		device.commands.filter((command) => command.type === "telemetry_policy"),
	).toHaveLength(1);
});

test("removing acknowledged deliveries walks an older agent's outbox from its oldest entry", async () => {
	const device = outboxDevice();
	for (let count = 0; count < 5; count++) device.publish();
	await acknowledgeGroupTelemetryThrough(device.call, "device", {
		sequence: 4,
		envelopeDigest: await digestText("envelope-4"),
	});
	expect(device.outbox.map((row) => row.sequence)).toEqual([5]);
});

test("a prefix-evicting agent clears the backlog with one acknowledgement", async () => {
	const device = outboxDevice({ prefixAcknowledge: true });
	for (let count = 0; count < 5; count++) device.publish();
	await acknowledgeGroupTelemetryThrough(device.call, "device", {
		sequence: 4,
		envelopeDigest: await digestText("envelope-4"),
	});
	expect(device.outbox.map((row) => row.sequence)).toEqual([5]);
	expect(
		device.commands.filter((command) => command.type === "telemetry_read"),
	).toHaveLength(0);
});
