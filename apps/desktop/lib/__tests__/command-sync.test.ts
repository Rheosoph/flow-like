import type { IGenericCommand } from "@flow-like/flow-like-ui";
import { describe, expect, test } from "vitest";
import {
	LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES,
	MAX_COMMAND_SYNC_BODY_BYTES,
	MAX_LAMBDA_SYNC_PAYLOAD_BYTES,
	MAX_LEGACY_COMMAND_SYNC_BODY_BYTES,
	MAX_SINGLE_COMMAND_SYNC_BODY_BYTES,
	MAX_UNDO_REDO_SYNC_BODY_BYTES,
	chunkCommandsForSync,
	chunkLegacyCommandsForRecovery,
	commandSyncHasPendingMutation,
	evaluateBoardLineage,
	evaluateCommandSyncRemoteIdentity,
	systemTimeToNanos,
} from "../../components/tauri-provider/command-sync";

const commandOfSize = (id: number, bytes: number): IGenericCommand =>
	({
		command_type: "UpsertNode",
		node: { id: `node-${id}`, payload: "x".repeat(bytes) },
	}) as unknown as IGenericCommand;

const commandWithPayload = (payload: string): IGenericCommand =>
	({
		command_type: "UpsertNode",
		node: { id: "escaped-node", payload },
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

	test("large dependent batches stay atomic instead of exposing a persisted prefix", () => {
		const perCommand = 100 * 1024;
		const commands = Array.from({ length: 12 }, (_, i) =>
			commandOfSize(i, perCommand),
		);
		const chunks = chunkCommandsForSync(commands);

		expect(chunks).toHaveLength(1);
		expect(chunks[0]).toHaveLength(commands.length);
		chunks[0].forEach((command, index) => {
			expect(command).toBe(commands[index]);
		});
	});

	test("an aggregate oversized batch is rejected without returning a sendable prefix", () => {
		const perCommand = Math.floor(MAX_COMMAND_SYNC_BODY_BYTES / 2);
		const commands = [
			commandOfSize(0, perCommand),
			commandOfSize(1, perCommand),
		];

		expect(() => chunkCommandsForSync(commands)).toThrow(
			/atomic board command batch/,
		);
	});

	test("a command above the route-safe singleton limit is identified for blocked-outbox retention", () => {
		expect(() =>
			chunkCommandsForSync([
				commandOfSize(1, MAX_SINGLE_COMMAND_SYNC_BODY_BYTES + 1024),
			]),
		).toThrow(/safe sync limit/);
	});

	test("a typical plan can use the expanded four MiB raw ceiling", () => {
		const command = commandOfSize(1, MAX_COMMAND_SYNC_BODY_BYTES - 4096);
		expect(chunkCommandsForSync([command])).toEqual([[command]]);
	});

	test("escape-heavy JSON is rejected when its Lambda envelope exceeds six MiB", () => {
		const command = commandWithPayload("\\".repeat(1_600_000));
		const rawBytes = new TextEncoder().encode(
			JSON.stringify({ commands: [command] }),
		).length;
		expect(rawBytes).toBeLessThan(MAX_COMMAND_SYNC_BODY_BYTES);
		expect(() => chunkCommandsForSync([command])).toThrow(
			/escaped Lambda envelope/,
		);
	});

	test("empty input produces no chunks", () => {
		expect(chunkCommandsForSync([])).toHaveLength(0);
	});
});

describe("chunkLegacyCommandsForRecovery", () => {
	test("freezes an old flat tail into ordered route-safe recovery chunks", () => {
		const commands = Array.from({ length: 12 }, (_, i) =>
			commandOfSize(i, 100 * 1024),
		);
		const chunks = chunkLegacyCommandsForRecovery(commands);

		expect(chunks.length).toBeGreaterThan(1);
		expect(chunks.flat()).toEqual(commands);
		for (const chunk of chunks) {
			const bytes = new TextEncoder().encode(
				JSON.stringify({ commands: chunk }),
			).length;
			expect(bytes).toBeLessThanOrEqual(MAX_LEGACY_COMMAND_SYNC_BODY_BYTES);
		}
	});

	test("blocks a legacy singleton that cannot pass the deployment ingress", () => {
		expect(() =>
			chunkLegacyCommandsForRecovery([
				commandOfSize(0, MAX_COMMAND_SYNC_BODY_BYTES + 1024),
			]),
		).toThrow(/atomic board command batch/);
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

describe("evaluateCommandSyncRemoteIdentity", () => {
	const current = {
		remoteProfileId: "profile-a",
		remotePrincipalId: "principal-a",
		remoteHub: "https://hub-a.example",
	};
	const bound = { ...current, remoteIdentityVersion: 1 as const };

	test("identity comparison leaves ownerless legacy authorization to the caller", () => {
		expect(evaluateCommandSyncRemoteIdentity({}, current).apply).toBe(true);
		expect(evaluateCommandSyncRemoteIdentity(bound, current).apply).toBe(true);
	});

	test.each([
		["profile", { ...bound, remoteProfileId: "profile-b" }],
		["account", { ...bound, remotePrincipalId: "principal-b" }],
		["Hub", { ...bound, remoteHub: "https://hub-b.example" }],
	])("refuses a different remote %s", (label, recorded) => {
		const decision = evaluateCommandSyncRemoteIdentity(recorded, current);
		expect(decision.apply).toBe(false);
		expect(decision.refusalReason).toContain(label);
	});

	test("refuses a captured identity while signed out", () => {
		expect(evaluateCommandSyncRemoteIdentity(bound, {}).apply).toBe(false);
	});

	test("new rows fail closed when no account was available at insertion", () => {
		expect(
			evaluateCommandSyncRemoteIdentity(
				{
					remoteIdentityVersion: 1,
					remoteProfileId: current.remoteProfileId,
					remoteHub: current.remoteHub,
				},
				current,
			).apply,
		).toBe(false);
	});

	test("later profile hydration is allowed when the authenticated owner matches", () => {
		expect(
			evaluateCommandSyncRemoteIdentity(
				{
					remoteIdentityVersion: 1,
					remotePrincipalId: current.remotePrincipalId,
				},
				current,
			).apply,
		).toBe(true);
	});
});

describe("sync body limits", () => {
	test("the four MiB raw cap is paired with a separate Lambda envelope guard", () => {
		expect(MAX_COMMAND_SYNC_BODY_BYTES).toBe(4 * 1024 * 1024);
		expect(MAX_COMMAND_SYNC_BODY_BYTES).toBeLessThan(
			MAX_LAMBDA_SYNC_PAYLOAD_BYTES,
		);
		expect(MAX_LAMBDA_SYNC_PAYLOAD_BYTES).toBe(6 * 1024 * 1024);
		expect(LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES).toBeGreaterThan(0);
		expect(MAX_COMMAND_SYNC_BODY_BYTES).toBeLessThan(16 * 1024 * 1024);
		expect(MAX_UNDO_REDO_SYNC_BODY_BYTES).toBeLessThan(16 * 1024 * 1024);
		expect(MAX_UNDO_REDO_SYNC_BODY_BYTES).toBe(MAX_COMMAND_SYNC_BODY_BYTES);
	});
});

describe("commandSyncHasPendingMutation", () => {
	test("completed receipt tombstones do not block remote freshness", () => {
		expect(
			commandSyncHasPendingMutation({
				chunks: [],
				commands: undefined,
			}),
		).toBe(false);
	});

	test("unsent tails and blocked exact payloads remain pending", () => {
		expect(
			commandSyncHasPendingMutation({ chunks: [[commandOfSize(1, 10)]] }),
		).toBe(true);
		expect(commandSyncHasPendingMutation({ blockedReason: "too large" })).toBe(
			true,
		);
	});
});
