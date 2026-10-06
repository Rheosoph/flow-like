import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import type { ILogMetadata } from "../../../../lib/schema/flow/log-metadata";
import type {
	OutcomeInput,
	OutcomeOf,
	RunSettlement,
	TerminalSignals,
} from "../contracts";
import { NO_TERMINAL_SIGNALS } from "./events";
import {
	NO_PLACE_RESOURCE,
	classifyError,
	isNoPlaceError,
	outcomeOf,
} from "./outcome";

const _contract: OutcomeOf = outcomeOf;

const meta = (log_level: number): ILogMetadata => ({
	app_id: "app-1",
	board_id: "board-1",
	end: 2,
	event_id: "event-1",
	log_level,
	node_id: "node-1",
	payload: [],
	run_id: "run-7",
	start: 1,
	version: "1",
});

const resolved = (value: ILogMetadata | null = null): RunSettlement => ({
	kind: "resolved",
	meta: value,
});
const rejected = (error: unknown): RunSettlement => ({
	kind: "rejected",
	error,
});

const input = (
	settlement: RunSettlement,
	terminal: Partial<TerminalSignals> = {},
	extra: Partial<Omit<OutcomeInput, "settlement" | "terminal">> = {},
): OutcomeInput => ({
	settlement,
	terminal: { ...NO_TERMINAL_SIGNALS, ...terminal },
	eventCount: terminal.runInitiated ? 3 : 0,
	stopRequested: false,
	host: "app",
	...extra,
});

const started = { runInitiated: true } as const;

const quotaError = (resource: string, status = 402) =>
	new ApiResponseError({
		status,
		code: "PLAN_LIMIT_EXCEEDED",
		errorId: "err-1",
		message: `Your FREE plan does not have enough ${resource.replace(/_/g, " ")} available for this action.`,
		quota: {
			resource,
			scope: "account",
			payerId: "user-1",
			plan: "FREE",
			used: 2,
			reserved: 0,
			limit: 2,
			unit: "executions",
		},
	});

describe("outcomeOf, one test per row of PLAN §3.5", () => {
	test("1 · a cancelled runtime-variables prompt never started the run", () => {
		expect(
			_contract(
				input(
					rejected(
						new Error("Execution cancelled: runtime variables not configured"),
					),
				),
			),
		).toEqual({ kind: "notStarted", reason: "promptCancelled" });
		expect(
			_contract(
				input(
					rejected(
						new Error(
							"Execution cancelled: the execution service unmounted while the run waited for a prompt",
						),
					),
				),
			),
		).toEqual({ kind: "notStarted", reason: "promptCancelled" });
	});

	test("2 · resolved undefined with no events: consent declined, or stopped before it began", () => {
		expect(_contract(input(resolved()))).toEqual({
			kind: "notStarted",
			reason: "declined",
		});
		expect(_contract(input(resolved(), {}, { stopRequested: true }))).toEqual({
			kind: "notStarted",
			reason: "removedFromQueue",
		});
	});

	test("3 · a refused place is no failure; a stopped run is never queued again", () => {
		const error = quotaError(NO_PLACE_RESOURCE);
		expect(_contract(input(rejected(error)))).toEqual({ kind: "noPlace" });
		expect(
			_contract(input(rejected(error), {}, { stopRequested: true })),
		).toEqual({
			kind: "notStarted",
			reason: "removedFromQueue",
		});
	});

	test("4 · a stop request wins over whatever the stream said", () => {
		expect(
			_contract(
				input(
					resolved(meta(4)),
					{ ...started, completedStatus: "failed" },
					{ stopRequested: true },
				),
			),
		).toEqual({ kind: "stopped" });
		const hostedDetach = new DOMException(
			"The operation was aborted.",
			"AbortError",
		);
		expect(
			_contract(
				input(rejected(hostedDetach), started, {
					stopRequested: true,
					host: "hosted",
				}),
			),
		).toEqual({ kind: "stopped" });
	});

	test("5 · completed cancelled is a stop (also the tray's Stop all runs on a device run)", () => {
		expect(
			_contract(
				input(resolved(meta(4)), { ...started, completedStatus: "cancelled" }),
			),
		).toEqual({ kind: "stopped" });
		expect(
			_contract(
				input(resolved(), { ...started, completedStatus: "cancelled" }),
			),
		).toEqual({ kind: "stopped" });
	});

	test("6 · a rejected:<stage> step fails as rejected, before any other failure", () => {
		expect(
			_contract(
				input(resolved(meta(4)), {
					...started,
					completedStatus: "failed",
					rejectedStage: "setup",
					errorMessage: "Missing WASM package artifacts",
					logLevel: 4,
				}),
			),
		).toEqual({
			kind: "failed",
			failure: "rejected",
			message: "Missing WASM package artifacts",
			detail:
				"Missing WASM package artifacts · status failed · rejected:setup · log level 4",
		});
	});

	test("7 · completed timeout", () => {
		expect(
			_contract(
				input(resolved(), {
					...started,
					completedStatus: "timeout",
					errorMessage: "Execution timeout",
				}),
			),
		).toEqual({
			kind: "failed",
			failure: "timeout",
			message: "Execution timeout",
			detail: "Execution timeout · status timeout",
		});
	});

	test("8 · a stream error or completed failed is a flow failure", () => {
		expect(
			_contract(
				input(resolved(), { ...started, errorMessage: "Execution failed" }),
			),
		).toEqual({
			kind: "failed",
			failure: "flow",
			message: "Execution failed",
			detail: "Execution failed",
		});
		expect(
			_contract(
				input(resolved(meta(4)), {
					...started,
					completedStatus: "failed",
					logLevel: 4,
				}),
			),
		).toEqual({
			kind: "failed",
			failure: "flow",
			message: null,
			detail: "status failed · log level 4",
		});
		const hostedThrow = new Error("Execution failed");
		expect(
			_contract(
				input(rejected(hostedThrow), {
					...started,
					errorMessage: "Execution failed",
				}),
			),
		).toMatchObject({
			kind: "failed",
			failure: "flow",
			message: "Execution failed",
		});
		expect(
			_contract(input(resolved(), { ...started, errorMessage: "" })),
		).toEqual({
			kind: "failed",
			failure: "flow",
			message: null,
			detail: null,
		});
	});

	test("9 · rejections: quota, permission, sign-in, network and abort", () => {
		expect(_contract(input(rejected(quotaError("cloud_runtime_ms"))))).toEqual({
			kind: "failed",
			failure: "quota",
			message:
				"Your FREE plan does not have enough cloud runtime ms available for this action.",
			detail:
				"[PLAN_LIMIT_EXCEEDED; ref err-1] Your FREE plan does not have enough cloud runtime ms available for this action.",
		});
		const forbidden = new ApiResponseError({
			status: 403,
			code: "FORBIDDEN",
			message: "You may not run this event.",
		});
		expect(_contract(input(rejected(forbidden)))).toMatchObject({
			failure: "permission",
			message: "You may not run this event.",
		});
		const oauth = Object.assign(
			new Error("Missing OAuth authorization for: Google"),
			{
				isOAuthError: true,
				missingProviders: [],
			},
		);
		expect(_contract(input(rejected(oauth)))).toMatchObject({
			failure: "oauth",
			message: "Missing OAuth authorization for: Google",
		});
		expect(
			_contract(input(rejected(new TypeError("Failed to fetch")))),
		).toMatchObject({
			failure: "network",
			message: "Failed to fetch",
		});
		const aborted = new DOMException(
			"signal is aborted without reason",
			"AbortError",
		);
		expect(_contract(input(rejected(aborted)))).toMatchObject({
			failure: "network",
		});
	});

	test("10 · any other rejection fails the flow, with the text a native error carries", () => {
		expect(
			_contract(
				input(
					rejected(
						new Error("The workflow failed or exceeded its request deadline."),
					),
				),
			),
		).toEqual({
			kind: "failed",
			failure: "flow",
			message: "The workflow failed or exceeded its request deadline.",
			detail: "The workflow failed or exceeded its request deadline.",
		});
		expect(
			_contract(
				input(rejected({ error: "Board board-1 could not be resolved: gone" })),
			),
		).toMatchObject({
			failure: "flow",
			message: "Board board-1 could not be resolved: gone",
		});
		expect(_contract(input(rejected(undefined)))).toEqual({
			kind: "failed",
			failure: "flow",
			message: null,
			detail: null,
		});
	});

	test("11 · completed completed succeeds, whatever the log level says", () => {
		expect(
			_contract(
				input(resolved(meta(4)), { ...started, completedStatus: "completed" }),
			),
		).toEqual({ kind: "succeeded" });
	});

	test("12 · without a terminal event a device run's log level decides (fallback)", () => {
		expect(_contract(input(resolved(meta(4)), started))).toEqual({
			kind: "failed",
			failure: "flow",
			message: null,
			detail: "log level 4 · run run-7",
		});
		expect(_contract(input(resolved(meta(2)), started))).toEqual({
			kind: "succeeded",
		});
	});

	test("13 · resolved undefined after run_initiated without a terminal event is unknown", () => {
		expect(_contract(input(resolved(), started))).toEqual({ kind: "unknown" });
		expect(_contract(input(resolved(), {}, { eventCount: 4 }))).toEqual({
			kind: "unknown",
		});
	});

	test("14 · otherwise the run succeeded", () => {
		expect(_contract(input(resolved(meta(0))))).toEqual({ kind: "succeeded" });
		expect(_contract(input(resolved(meta(0)), started))).toEqual({
			kind: "succeeded",
		});
	});
});

describe("classifyError and isNoPlaceError", () => {
	test("only the concurrency resource refuses a place", () => {
		expect(isNoPlaceError(quotaError(NO_PLACE_RESOURCE))).toBe(true);
		expect(isNoPlaceError(quotaError("cloud_starts"))).toBe(false);
		expect(isNoPlaceError(new Error("nope"))).toBe(false);
		expect(isNoPlaceError(null)).toBe(false);
		expect(
			isNoPlaceError({ status: 429, quota: { resource: NO_PLACE_RESOURCE } }),
		).toBe(false);
	});

	test("a declined computer-automation consent never started", () => {
		expect(
			classifyError(
				new Error("Computer automation was not approved for this event."),
			),
		).toEqual({ kind: "notStarted", reason: "declined" });
	});

	test("a 401 is a permission failure, a 402 purchase a quota one", () => {
		expect(
			classifyError(
				new ApiResponseError({ status: 401, message: "Sign in again." }),
			),
		).toMatchObject({ failure: "permission", message: "Sign in again." });
		expect(
			classifyError(
				new ApiResponseError({
					status: 402,
					code: "PURCHASE_REQUIRED",
					message: "Buy this app first.",
				}),
			),
		).toMatchObject({ failure: "quota", message: "Buy this app first." });
	});

	test("a request timeout is a network failure", () => {
		const timeout = Object.assign(new Error("Request timed out"), {
			name: "RequestTimeoutError",
		});
		expect(classifyError(timeout)).toMatchObject({ failure: "network" });
	});
});
