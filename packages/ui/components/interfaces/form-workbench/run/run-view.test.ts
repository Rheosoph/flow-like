import { describe, expect, test } from "bun:test";
import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import type {
	CopyValue,
	FileSlot,
	RunEntry,
	RunOutcome,
	RunOutput,
	RunStatus,
} from "../contracts";
import { NO_TERMINAL_SIGNALS } from "./events";
import {
	elapsedMs,
	hasOutput,
	liveStatusOf,
	pendingInteractionsOf,
	repeatBlocker,
	repeatable,
	stageRunOf,
	statusOfOutcome,
} from "./run-view";
import { EMPTY_RUN_SUMMARY } from "./summary";

const NOW = 1_790_000_000_000;

const slot = (parts: Partial<FileSlot> = {}): FileSlot => ({
	id: "slot-1",
	name: "invoice-RE-2026-0917.pdf",
	size: 1_284_096,
	type: "application/pdf",
	state: "sent",
	progress: null,
	ref: {
		kind: "flowpath",
		flowPath: {
			path: "tmp/a.pdf",
			store_ref: "__flow_like_http_request_files",
			cache_store_ref: null,
		},
		url: null,
	},
	error: null,
	sentAt: NOW - 60_000,
	expiresAt: null,
	...parts,
});

const entry = (
	parts: Partial<Omit<RunEntry, "copy">> & {
		readonly values?: Readonly<Record<string, CopyValue>>;
	} = {},
): RunEntry => {
	const { values = {}, ...rest } = parts;
	return {
		id: "run-1",
		n: 1,
		origin: "session",
		status: "done",
		createdAt: NOW - 120_000,
		startedAt: NOW - 100_000,
		endedAt: NOW - 52_000,
		copy: {
			values,
			presetName: null,
			leftAsIs: false,
			perRun: [],
			replaced: [],
		},
		target: "local",
		streamId: null,
		backendRunId: null,
		output: null,
		outcome: null,
		summary: EMPTY_RUN_SUMMARY,
		stopRequested: false,
		pendingSlotIds: [],
		waitingForPlace: false,
		failedAt: null,
		unseenFailure: false,
		...rest,
	};
};

const output = (parts: Partial<RunOutput> = {}): RunOutput => ({
	eventCount: 1,
	steps: [],
	answer: "",
	reasoning: null,
	attachments: [],
	result: null,
	interactions: [],
	terminal: NO_TERMINAL_SIGNALS,
	...parts,
});

const question = (
	parts: Partial<IInteractionRequest> = {},
): IInteractionRequest => ({
	id: "ask-1",
	name: "Approve?",
	description: "",
	interaction_type: { type: "single_choice", options: [] },
	status: "pending",
	ttl_seconds: 120,
	expires_at: NOW / 1000 + 60,
	...parts,
});

describe("stageRunOf", () => {
	const runs = [
		entry({ id: "r4", n: 4, status: "queued" }),
		entry({ id: "r3", n: 3, status: "sending" }),
		entry({ id: "r2", n: 2, status: "running" }),
		entry({ id: "r1", n: 1, status: "done" }),
	];

	test("the selected run", () => {
		expect(stageRunOf({ runs, view: { selectedRunId: "r1" } })?.id).toBe("r1");
		expect(stageRunOf({ runs, view: { selectedRunId: "r4" } })?.id).toBe("r4");
	});

	test("else the newest run that does not wait; a missing selection falls back too", () => {
		expect(stageRunOf({ runs, view: { selectedRunId: null } })?.id).toBe("r2");
		expect(stageRunOf({ runs, view: { selectedRunId: "gone" } })?.id).toBe(
			"r2",
		);
	});

	test("the empty stage while the first run's files are sent, and without runs", () => {
		expect(
			stageRunOf({ runs: runs.slice(0, 2), view: { selectedRunId: null } }),
		).toBeNull();
		expect(stageRunOf({ runs: [], view: { selectedRunId: null } })).toBeNull();
	});
});

describe("elapsedMs", () => {
	test("from the first sign of the run to its end", () => {
		expect(elapsedMs(entry(), NOW)).toBe(48_000);
	});

	test("to now while live; nothing before it started", () => {
		expect(elapsedMs(entry({ status: "running", endedAt: null }), NOW)).toBe(
			100_000,
		);
		expect(
			elapsedMs(
				entry({ status: "starting", startedAt: null, endedAt: null }),
				NOW,
			),
		).toBe(0);
	});

	test("an ended run without an end time does not keep ticking", () => {
		expect(elapsedMs(entry({ status: "unknown", endedAt: null }), NOW)).toBe(0);
	});
});

describe("repeatable and repeatBlocker (S4)", () => {
	test("a run of this session whose files are still held", () => {
		const run = entry({
			values: {
				invoice: slot(),
				documents: [
					slot({ id: "s2", state: "sending", progress: 0.4 }),
					slot({ id: "s3", state: "failed", error: "x" }),
				],
				vendor: "Nordwind Logistik GmbH",
				approved: true,
			},
		});
		expect(repeatable(run, NOW)).toBe(true);
		expect(repeatBlocker(run, NOW)).toBeNull();
	});

	test("a file of an older run must be picked again", () => {
		const run = entry({
			origin: "history",
			values: { invoice: slot({ state: "reminder", ref: null }) },
		});
		expect(repeatable(run, NOW)).toBe(false);
		expect(repeatBlocker(run, NOW)).toBe("files");
	});

	test("a URL upload past its expiry cannot be sent again", () => {
		const expired = slot({
			ref: { kind: "url", url: "https://x.example/a.pdf" },
			expiresAt: NOW - 1,
		});
		expect(repeatBlocker(entry({ values: { receipt: [expired] } }), NOW)).toBe(
			"files",
		);
		expect(
			repeatable(
				entry({ values: { receipt: [{ ...expired, expiresAt: NOW + 1 }] } }),
				NOW,
			),
		).toBe(true);
	});

	test("a value kept out of storage and lost to a reload", () => {
		const run = entry({
			origin: "history",
			values: { api_key: { $hidden: true } },
		});
		expect(repeatBlocker(run, NOW)).toBe("hidden");
		const both = entry({
			origin: "history",
			values: {
				api_key: { $hidden: true },
				invoice: slot({ state: "reminder" }),
			},
		});
		expect(repeatBlocker(both, NOW)).toBe("files");
	});

	test("an object property kept out of storage blocks it too", () => {
		const run = entry({
			origin: "history",
			values: { creds: { user: "bob", password: { $hidden: true } } },
		});
		expect(repeatBlocker(run, NOW)).toBe("hidden");
		expect(repeatable(run, NOW)).toBe(false);
		const filled = entry({
			values: { creds: { user: "bob", password: "hunter2", admin: true } },
		});
		expect(repeatable(filled, NOW)).toBe(true);
	});

	test("a run the form left before its turn: files never, without files Run again", () => {
		const formClosed = {
			status: "notStarted",
			outcome: { kind: "notStarted", reason: "formClosed" },
		} as const;
		expect(
			repeatBlocker(entry({ ...formClosed, values: { invoice: slot() } }), NOW),
		).toBe("files");
		expect(
			repeatable(entry({ ...formClosed, values: { order: "48213-7" } }), NOW),
		).toBe(true);
		const takenOut = entry({
			status: "notStarted",
			outcome: { kind: "notStarted", reason: "removedFromQueue" },
			values: { invoice: slot({ state: "waiting", ref: null, sentAt: null }) },
		});
		expect(repeatable(takenOut, NOW)).toBe(true);
	});

	test("a history run without files or secrets can run again", () => {
		expect(
			repeatable(
				entry({
					origin: "history",
					values: { order: "48213-7", quantity: "2" },
				}),
				NOW,
			),
		).toBe(true);
	});

	test("objects that only look a little like a file are values", () => {
		const group = { id: "x", name: "y", state: "sent" };
		expect(repeatable(entry({ values: { address: group } }), NOW)).toBe(true);
	});
});

describe("hasOutput and statusOfOutcome", () => {
	test("an answer, files or any result count; steps alone do not", () => {
		expect(hasOutput(null)).toBe(false);
		expect(hasOutput(output())).toBe(false);
		expect(hasOutput(output({ answer: "  \n" }))).toBe(false);
		expect(hasOutput(output({ answer: "Done." }))).toBe(true);
		expect(
			hasOutput(output({ attachments: ["https://x.example/a.pdf"] })),
		).toBe(true);
		expect(hasOutput(output({ result: { value: false } }))).toBe(true);
		expect(hasOutput(output({ result: { value: null } }))).toBe(true);
	});

	test("each outcome's status", () => {
		const cases: [RunOutcome, RunOutput | null, RunStatus][] = [
			[{ kind: "succeeded" }, output({ result: { value: 0 } }), "done"],
			[{ kind: "succeeded" }, output(), "empty"],
			[{ kind: "succeeded" }, null, "empty"],
			[
				{ kind: "failed", failure: "flow", message: null, detail: null },
				output(),
				"failed",
			],
			[{ kind: "stopped" }, output({ answer: "partial" }), "stopped"],
			[{ kind: "notStarted", reason: "declined" }, null, "notStarted"],
			[{ kind: "noPlace" }, null, "queued"],
			[{ kind: "unknown" }, null, "unknown"],
		];
		for (const [outcome, runOutput, status] of cases)
			expect(statusOfOutcome(outcome, runOutput)).toBe(status);
	});
});

describe("pendingInteractionsOf and liveStatusOf", () => {
	test("questions that still wait", () => {
		const waiting = output({
			interactions: [
				question(),
				question({ id: "ask-2", status: "responded" }),
				question({ id: "ask-3", expires_at: NOW / 1000 - 1 }),
				question({ id: "ask-4", expires_at: 0 }),
			],
		});
		expect(pendingInteractionsOf(waiting, NOW).map((item) => item.id)).toEqual([
			"ask-1",
			"ask-4",
		]);
		expect(pendingInteractionsOf(null, NOW)).toEqual([]);
	});

	test("asking while a question waits, streaming once text arrives, else running", () => {
		expect(
			liveStatusOf(output({ interactions: [question()], answer: "x" }), NOW),
		).toBe("asking");
		expect(liveStatusOf(output({ answer: "Invoice" }), NOW)).toBe("streaming");
		expect(liveStatusOf(output(), NOW)).toBe("running");
		expect(liveStatusOf(null, NOW)).toBe("running");
	});
});
