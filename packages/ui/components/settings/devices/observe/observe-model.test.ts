import { describe, expect, test } from "bun:test";
import type { ArchiveRoster } from "../../../../lib/device-management/types";
import type { HistoryStream } from "./use-history";

/*
 * The modules under test import React components, so they load after the test
 * runner is up; none of them is rendered here.
 */
const {
	logRecords,
	metricSamples,
	mergeSamples,
	recordsOf,
	seriesOf,
	spanOf,
	stepOf,
} = await import("./observe-data");
const { READER_REQUEST_KIND, parseReaderRequest, readerRequestOf } =
	await import("./reader-request");
const { historyPause, timelineFacts } = await import("./timeline-model");
const { historyRows } = await import("./history-settings");
const { readerGrantExpiry, recordingOf, unsavedOutcome } = await import(
	"./use-history"
);
const { METRICS_REQUEST_KIND, keyThumbprint, parseMetricsRequest } =
	await import("./use-shared-metrics");

const NOW = 1_790_769_600;
const row = (
	sequence: number,
	data: Record<string, unknown>,
	timestamp = NOW,
) => ({ sequence, timestamp, kind: "message", data });

describe("samples", () => {
	test("a read with records is sorted oldest first; a bare sample needs the time it was read", () => {
		const samples = metricSamples({
			records: [
				{ timestamp: 20, data: { cpu_percent: 2 } },
				{ timestamp: 10, data: { cpu_percent: 1 } },
				{ timestamp: "x", data: { cpu_percent: 9 } },
				{ timestamp: 30, data: null },
			],
		});
		expect(samples.map((sample) => sample.at)).toEqual([10, 20]);
		expect(metricSamples({ cpu_percent: 4 }, 99)).toEqual([
			{ at: 99, data: { cpu_percent: 4 } },
		]);
		expect(metricSamples({ cpu_percent: 4 })).toEqual([]);
		expect(metricSamples(undefined, 99)).toEqual([]);
		expect(metricSamples({ records: [] }, 99)).toEqual([
			{ at: 99, data: { records: [] } },
		]);
	});

	test("live samples continue the trend; a missing value is a gap, never zero", () => {
		const trend = [
			{ at: 10, data: { cpu_percent: 1 } },
			{ at: 40, data: {} },
		];
		const merged = mergeSamples(trend, [
			{ at: 40, data: { cpu_percent: 7 } },
			{ at: 45, data: { cpu_percent: 3 } },
		]);
		expect(merged.map((sample) => sample.at)).toEqual([10, 40, 45]);
		const series = seriesOf(merged, (data) =>
			typeof data.cpu_percent === "number" ? data.cpu_percent : undefined,
		);
		expect(series[0]).toBe(1);
		expect(Number.isNaN(series[1])).toBe(true);
		expect(series[2]).toBe(3);
		expect(spanOf(merged)).toBe(35);
		expect(stepOf(merged)).toBe(18);
		expect(stepOf([merged[0] as (typeof merged)[number]])).toBeUndefined();
	});

	test("records of a stream keep only well-formed rows", () => {
		expect(
			recordsOf({
				records: [
					row(1, { a: 1 }),
					{ sequence: "2", timestamp: NOW, data: {} },
					{ sequence: 3, timestamp: NOW, data: null },
				],
				meta: {},
			}).map((entry) => entry.sequence),
		).toEqual([1]);
		expect(recordsOf(undefined)).toEqual([]);
	});
});

describe("log records", () => {
	test("a record that only reports dropped lines is a marker, and the deletion marker leads", () => {
		const records = logRecords(
			[
				row(5, { stream: "stdout", message: "a", truncated: false }),
				row(6, {
					stream: "stdout",
					message: "[flow-like] 3",
					dropped_lines: 3,
				}),
				row(7, { stream: "stderr", message: "b", truncated: true }),
				row(8, { stream: "weird", message: 12 }),
			],
			4,
		);
		expect(records.map((record) => record.kind)).toEqual([
			"gap",
			"line",
			"gap",
			"line",
			"line",
		]);
		expect(records[0]).toMatchObject({ reason: "evicted", before: 5 });
		expect(records[2]).toMatchObject({ reason: "dropped", count: 3 });
		expect(records[3]).toMatchObject({ stream: "stderr", truncated: true });
		expect(records[4]).toMatchObject({ stream: "stdout", message: "" });
		expect(logRecords([]).length).toBe(0);
	});
});

describe("timeline facts", () => {
	const records = [
		row(1, { kind: "operation", source_id: "op-1", state: "accepted" }, 100),
		row(
			2,
			{
				kind: "replica",
				placement_id: "api",
				state: "starting",
				replica_slot: 1,
				config_revision: 4,
				process_id: 77,
			},
			110,
		),
		row(
			3,
			{
				kind: "operation",
				source_id: "op-1",
				placement_id: "api",
				state: "completed",
			},
			120,
		),
		row(4, { kind: "operation", source_id: "op-2", state: "<b>" }, 130),
		row(5, { kind: "arbitrary", source_id: "x", state: "completed" }, 140),
		row(6, { kind: "replica", placement_id: "web", state: "running" }, 150),
	];

	test("one entry per command with its newest state; unknown kinds and states are left out", () => {
		const facts = timelineFacts({ records, serviceId: null, now: NOW });
		expect(facts.map((fact) => fact.id)).toEqual(["m6", "oop-1", "m2"]);
		expect(facts[1]).toMatchObject({
			type: "command",
			state: "completed",
			service: "api",
			at: 120,
			acceptedAt: 100,
		});
		expect(facts[2]).toMatchObject({
			type: "instance",
			slot: 1,
			settings: 4,
			process: 77,
		});
	});

	test("a service sees only its own entries and no device facts", () => {
		const facts = timelineFacts({
			records,
			serviceId: "api",
			now: NOW,
			host: { booted_at: 50, agent_started_at: 60 },
			access: { version: 2, issuedAt: 70, applied: true, people: 1 },
			rollouts: [
				{
					rollout_id: "r1",
					placement_id: "api",
					project_id: "app",
					state: "rolled_back",
					updated_at: 125,
					base_revision: 3,
				},
				{
					rollout_id: "r2",
					placement_id: "web",
					project_id: "app",
					state: "healthy",
					updated_at: 126,
				},
			],
		});
		expect(facts.map((fact) => fact.type)).toEqual([
			"update",
			"command",
			"instance",
		]);
		expect(facts[0]).toMatchObject({ state: "rolled_back", from: 3 });
	});

	test("device facts: the boot, an agent restart only when it came later, saved access rules, a paused recording", () => {
		const facts = timelineFacts({
			records: [],
			serviceId: null,
			now: NOW,
			host: { booted_at: 1_000, agent_started_at: 1_100 },
			access: { version: 5, issuedAt: 900, applied: false, people: 2 },
			history: [
				{
					scope: "device",
					kind: "metrics",
					expiresAt: NOW - 10,
					policyVersion: 3,
				},
				{
					scope: "device",
					kind: "logs",
					expiresAt: NOW + 10,
					policyVersion: 5,
				},
			],
		});
		expect(facts.map((fact) => fact.type)).toEqual([
			"history_paused",
			"boot",
			"access",
		]);
		const later = timelineFacts({
			records: [],
			serviceId: null,
			now: NOW,
			host: { booted_at: 1_000, agent_started_at: 5_000 },
		});
		expect(later.map((fact) => fact.type)).toEqual(["agent_start", "boot"]);
		const unknownBoot = timelineFacts({
			records: [],
			serviceId: null,
			now: NOW,
			host: { booted_at: null, agent_started_at: 5_000 },
		});
		expect(unknownBoot.map((fact) => fact.type)).toEqual(["agent_start"]);
	});

	test("a paused recording follows the device's word, else the list's expiry", () => {
		const base = { scope: "device", kind: "logs" as const, policyVersion: 1 };
		expect(historyPause({ ...base, expiresAt: NOW + 1 }, NOW)).toBeUndefined();
		expect(historyPause({ ...base, expiresAt: NOW - 1 }, NOW)).toEqual({
			since: NOW - 1,
			reason: "roster_expired",
		});
		expect(
			historyPause(
				{
					...base,
					expiresAt: NOW + 99,
					status: { state: "paused", reason: "quota_reached", since: 5 },
				},
				NOW,
			),
		).toEqual({ since: 5, reason: "quota_reached" });
		expect(
			historyPause(
				{
					...base,
					expiresAt: NOW - 99,
					status: { state: "recording", reason: null, since: 5 },
				},
				NOW,
			),
		).toBeUndefined();
	});
});

describe("readers lists", () => {
	const roster = (expires: number, digest: string | null): ArchiveRoster => ({
		version: 1,
		device_id: "d",
		scope: "device",
		project_id: null,
		kind: "logs",
		policy_version: 2,
		previous_policy_digest: null,
		management_policy_digest: digest,
		recipients: [],
		issued_at: 0,
		expires_at: expires,
	});
	const stream = (
		partial: Partial<HistoryStream> & Pick<HistoryStream, "scope" | "kind">,
	): HistoryStream => ({
		roster: null,
		rosterText: null,
		status: undefined,
		denied: false,
		...partial,
	});

	test("recording state: the device's word first; for an older agent expiry and the rules it was signed for", () => {
		expect(recordingOf({ roster: null, status: undefined }, NOW, "a")).toEqual({
			state: "none",
		});
		expect(
			recordingOf(
				{ roster: roster(NOW + 5, "a"), status: undefined },
				NOW,
				"a",
			),
		).toEqual({ state: "recording", until: NOW + 5 });
		expect(
			recordingOf(
				{ roster: roster(NOW - 5, "a"), status: undefined },
				NOW,
				"a",
			),
		).toMatchObject({
			state: "paused",
			reason: "roster_expired",
			since: NOW - 5,
		});
		expect(
			recordingOf(
				{ roster: roster(NOW + 5, "a"), status: undefined },
				NOW,
				"b",
			),
		).toMatchObject({ state: "paused", reason: "rules_changed" });
		// A newer agent reports the state itself: no comparison (the digest is not passed).
		expect(
			recordingOf(
				{ roster: roster(NOW + 5, "a"), status: undefined },
				NOW,
				undefined,
			),
		).toMatchObject({ state: "recording" });
		expect(
			recordingOf(
				{
					roster: roster(NOW + 5, "a"),
					status: { state: "paused", reason: "outbox_full", since: 7 },
				},
				NOW,
				"a",
			),
		).toMatchObject({ state: "paused", reason: "outbox_full", since: 7 });
	});

	test("rows: one per list; a scope with neither list is one combined row", () => {
		const rows = historyRows(
			[
				stream({ scope: "device", kind: "logs", roster: roster(NOW + 5, "a") }),
				stream({ scope: "device", kind: "metrics" }),
				stream({ scope: "api", kind: "logs" }),
				stream({ scope: "api", kind: "metrics" }),
			],
			["device", "api", "absent"],
			NOW,
			"a",
		);
		expect(rows.map((entry) => [entry.key, entry.kinds.join("+")])).toEqual([
			["device|logs", "logs"],
			["device|metrics", "metrics"],
			["api", "logs+metrics"],
		]);
		expect(rows.map((entry) => entry.recording.state)).toEqual([
			"recording",
			"none",
			"none",
		]);
	});

	test("a reader needs a current grant with the kind on the scope", () => {
		const policy = {
			version: 1 as const,
			device_id: "d",
			policy_version: 1,
			previous_policy_digest: null,
			issued_at: 0,
			expires_at: NOW + 1_000,
			grants: [
				{
					grant_id: "g1",
					user_id: "mira",
					controller_key: { kty: "OKP", crv: "Ed25519", x: "k" } as never,
					scope: { kind: "project" as const, project_id: "app" },
					capabilities: ["logs" as const],
					expires_at: NOW + 50,
					group_id: null,
					group_version: null,
				},
				{
					grant_id: "g2",
					user_id: "mira",
					controller_key: { kty: "OKP", crv: "Ed25519", x: "k" } as never,
					scope: { kind: "device" as const },
					capabilities: ["logs" as const],
					expires_at: NOW - 1,
					group_id: null,
					group_version: null,
				},
			],
		};
		const of = (
			scope: string,
			kind: "logs" | "metrics",
			projectId: string | null,
		) => readerGrantExpiry(policy, "mira", { scope, kind, projectId }, NOW);
		expect(of("api", "logs", "app")).toBe(NOW + 50);
		expect(of("api", "metrics", "app")).toBeUndefined();
		expect(of("api", "logs", "other")).toBeUndefined();
		expect(of("device", "logs", null)).toBeUndefined();
		expect(
			readerGrantExpiry(
				undefined,
				"mira",
				{ scope: "device", kind: "logs", projectId: null },
				NOW,
			),
		).toBeUndefined();
	});

	test("a signature that didn't go through: no reply is never reported as nothing changed", () => {
		const error = new Error("timeout");
		expect(unsavedOutcome({ status: "unknown", error }, false)).toEqual({
			status: "failed",
			reason: "unconfirmed",
		});
		expect(unsavedOutcome({ status: "cancelled" }, false)).toEqual({
			status: "stopped",
		});
		expect(
			unsavedOutcome(
				{
					status: "failed",
					failure: { code: "unknown" } as never,
					error: new Error("Incorrect password or damaged vault"),
				},
				true,
			),
		).toEqual({ status: "failed", reason: "wrong_password" });
	});
});

describe("reader requests", () => {
	const recipient = {
		recipient_id: "key-x",
		user_id: "usr_1",
		public_key: [1, 2, 3],
	};

	test("the file, one bare reader and a list all parse; another device's file is refused", () => {
		const file = JSON.stringify(readerRequestOf("dev-1", recipient));
		expect(JSON.parse(file).kind).toBe(READER_REQUEST_KIND);
		expect(parseReaderRequest(file, "dev-1")).toEqual({
			ok: true,
			recipients: [recipient],
		});
		expect(parseReaderRequest(file, "dev-2")).toEqual({
			ok: false,
			problem: "other_device",
		});
		expect(parseReaderRequest(JSON.stringify(recipient), "dev-1").ok).toBe(
			true,
		);
		expect(
			parseReaderRequest(JSON.stringify([recipient, recipient]), "dev-1"),
		).toMatchObject({
			ok: true,
		});
	});

	test("anything else is invalid, and a list is capped", () => {
		for (const text of [
			"not json",
			"{}",
			"[]",
			JSON.stringify({ ...recipient, public_key: [300] }),
			JSON.stringify({ ...recipient, user_id: "" }),
			JSON.stringify([recipient, { nope: true }]),
		])
			expect(parseReaderRequest(text, "dev-1")).toEqual({
				ok: false,
				problem: "invalid",
			});
		expect(
			parseReaderRequest(JSON.stringify(Array(32).fill(recipient)), "dev-1"),
		).toEqual({ ok: false, problem: "too_many" });
	});

	test("shared-metrics requests: the file is bound to the device and the group", () => {
		const request = {
			member: {
				endpoint_id: "endpoint-1",
				signing_key: { kty: "OKP", crv: "Ed25519", x: "abc" },
			},
			key_package: "wire",
		};
		const file = JSON.stringify({
			kind: METRICS_REQUEST_KIND,
			version: 1,
			device_id: "dev-1",
			scope: "device",
			request,
		});
		expect(parseMetricsRequest(file, "dev-1", "device")).toMatchObject({
			ok: true,
		});
		expect(parseMetricsRequest(file, "dev-1", "api")).toEqual({
			ok: false,
			problem: "other_device",
		});
		expect(
			parseMetricsRequest(JSON.stringify([request]), "dev-1", "device").ok,
		).toBe(true);
		expect(
			parseMetricsRequest(JSON.stringify({ member: {} }), "dev-1", "device"),
		).toEqual({
			ok: false,
			problem: "invalid",
		});
	});

	test("a key thumbprint is the RFC 7638 digest the device reports", async () => {
		expect(
			await keyThumbprint({
				kty: "OKP",
				crv: "Ed25519",
				x: "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
			} as never),
		).toBe("kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
	});
});
