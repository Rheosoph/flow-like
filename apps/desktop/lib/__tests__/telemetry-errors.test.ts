import {
	addTelemetryBreadcrumb,
	clearTelemetryBreadcrumbs,
	getTelemetryBreadcrumbs,
	sanitizeTelemetryMessage,
} from "@flow-like/flow-like-ui/lib/telemetry/breadcrumbs";
import {
	type ITelemetryClient,
	createTelemetryClient,
} from "@flow-like/flow-like-ui/lib/telemetry/client";
import {
	type ITelemetryCapturedError,
	captureTelemetryError,
	normalizeError,
	parseErrorFrames,
	sanitizeTelemetryContext,
	setTelemetryErrorSink,
} from "@flow-like/flow-like-ui/lib/telemetry/errors";
import {
	endTelemetrySession,
	setTelemetrySessionSink,
	startTelemetrySession,
} from "@flow-like/flow-like-ui/lib/telemetry/session";
import type { IApiState } from "@flow-like/flow-like-ui/state/backend-state/api-state";
import type { IProfile } from "@flow-like/flow-like-ui/types";
import { afterEach, describe, expect, test, vi } from "vitest";

const profile = {
	bits: [],
	created: "",
	updated: "",
	name: "test",
} as IProfile;

let removeSink: (() => void) | undefined;

function drainPendingErrors() {
	const drained: ITelemetryCapturedError[] = [];
	const remove = setTelemetryErrorSink((error) => drained.push(error));
	remove();
	return drained;
}

function collectErrors() {
	const received: ITelemetryCapturedError[] = [];
	removeSink = setTelemetryErrorSink((error) => received.push(error));
	return received;
}

afterEach(() => {
	removeSink?.();
	removeSink = undefined;
	drainPendingErrors();
	clearTelemetryBreadcrumbs();
	const removeSessionSink = setTelemetrySessionSink(() => undefined);
	endTelemetrySession();
	removeSessionSink();
});

describe("parseErrorFrames", () => {
	const v8Stack = [
		"Error: boom",
		"    at doWork (/home/app/src/work.ts:12:9)",
		"    at async run (/home/app/src/run.ts:3:1)",
		"    at /home/app/node_modules/lib/index.js:7:2",
		"    at Object.<anonymous> (/home/app/main.js:1:1)",
		"    at native",
	].join("\n");

	const mozStack = [
		"doWork@http://localhost:3000/_next/static/chunks/app.js:12:9",
		"@http://localhost:3000/_next/static/chunks/app.js:20:1",
		"global code@[native code]",
	].join("\n");

	test("parses V8 frames and skips the message line", () => {
		const frames = parseErrorFrames(v8Stack);

		expect(frames).toHaveLength(5);
		expect(frames[0]).toEqual({
			function: "doWork",
			file: "/home/app/src/work.ts",
			lineno: 12,
			colno: 9,
			in_app: true,
		});
		expect(frames[1]?.function).toBe("run");
		expect(frames[2]).toMatchObject({
			file: "/home/app/node_modules/lib/index.js",
			in_app: false,
		});
		expect(frames[2]?.function).toBeUndefined();
		expect(frames[3]).toMatchObject({
			file: "/home/app/main.js",
			in_app: true,
		});
		expect(frames[3]?.function).toBeUndefined();
		expect(frames[4]).toMatchObject({ file: "native", in_app: false });
	});

	test("parses Firefox/Safari frames", () => {
		const frames = parseErrorFrames(mozStack);

		expect(frames).toHaveLength(3);
		expect(frames[0]).toEqual({
			function: "doWork",
			file: "http://localhost:3000/_next/static/chunks/app.js",
			lineno: 12,
			colno: 9,
			in_app: true,
		});
		expect(frames[1]?.function).toBeUndefined();
		expect(frames[1]?.lineno).toBe(20);
		expect(frames[2]).toMatchObject({
			function: "global code",
			file: "[native code]",
			in_app: false,
		});
	});

	test("strips query strings from frame files", () => {
		const frames = parseErrorFrames(
			"    at fetchIt (https://app.test/static/main.js?token=secret:5:3)",
		);

		expect(frames[0]).toEqual({
			function: "fetchIt",
			file: "https://app.test/static/main.js",
			lineno: 5,
			colno: 3,
			in_app: true,
		});
	});

	test("caps the stack at 100 frames and tolerates empty stacks", () => {
		const deep = Array.from(
			{ length: 150 },
			(_, index) => `    at fn${index} (/home/app/src/a.ts:${index}:1)`,
		).join("\n");

		expect(parseErrorFrames(deep)).toHaveLength(100);
		expect(parseErrorFrames(undefined)).toEqual([]);
		expect(parseErrorFrames("")).toEqual([]);
		expect(parseErrorFrames("not a stack at all")).toEqual([]);
	});
});

describe("normalizeError", () => {
	test("normalizes Error instances and subclasses", () => {
		const error = new TypeError("bad input");
		const normalized = normalizeError(error);

		expect(normalized.kind).toBe("TypeError");
		expect(normalized.value).toBe("bad input");
		expect(normalized.stack).toContain("bad input");
	});

	test("normalizes strings", () => {
		expect(normalizeError("plain failure")).toEqual({
			kind: "Error",
			value: "plain failure",
		});
	});

	test("normalizes DOMException-like values by name", () => {
		expect(normalizeError({ name: "AbortError", message: "aborted" })).toEqual({
			kind: "AbortError",
			value: "aborted",
			stack: undefined,
		});
	});

	test("normalizes objects with a message but no name", () => {
		const normalized = normalizeError({ message: "custom failure" });

		expect(normalized.kind).toBe("Error");
		expect(normalized.value).toBe("custom failure");
	});

	test("normalizes fully unknown values", () => {
		expect(normalizeError(42)).toEqual({
			kind: "UnknownError",
			value: "Non-Error exception: 42",
		});
		expect(normalizeError(null).kind).toBe("UnknownError");
		expect(normalizeError({ code: 500 }).value).toBe(
			'Non-Error exception: {"code":500}',
		);
	});
});

describe("telemetry breadcrumbs", () => {
	test("keeps at most 30 breadcrumbs, dropping the oldest", () => {
		for (let index = 0; index < 35; index++) {
			addTelemetryBreadcrumb({ category: "nav", message: `step ${index}` });
		}
		const breadcrumbs = getTelemetryBreadcrumbs();

		expect(breadcrumbs).toHaveLength(30);
		expect(breadcrumbs[0]?.message).toBe("step 5");
		expect(breadcrumbs[29]?.message).toBe("step 34");
		expect(Date.parse(breadcrumbs[0]?.ts ?? "")).not.toBeNaN();
	});

	test("sanitizes URLs and secrets in messages", () => {
		expect(
			sanitizeTelemetryMessage(
				"GET https://api.test/v1/apps/0a1b2c3d4e5f6a7b8c?token=secret",
			),
		).toBe("GET https://api.test/v1/apps/:id");
		expect(sanitizeTelemetryMessage("login failed password=hunter2")).toBe(
			"login failed password=[REDACTED]",
		);
		expect(
			sanitizeTelemetryMessage("header authorization: Bearer abcdef123456"),
		).toBe("header authorization=[REDACTED] [REDACTED]");
		expect(
			sanitizeTelemetryMessage(
				"open /apps/6f9619ff-8b86-4d01-b42d-00cf4fc964ff",
			),
		).toBe("open /apps/:id");
	});

	test("truncates long messages and drops unknown levels", () => {
		addTelemetryBreadcrumb({
			message: "x".repeat(400),
			level: "warning",
		});
		addTelemetryBreadcrumb({
			message: "second",
			level: "shout" as unknown as "warning",
		});
		const breadcrumbs = getTelemetryBreadcrumbs();

		expect(breadcrumbs[0]?.message).toHaveLength(257);
		expect(breadcrumbs[0]?.level).toBe("warning");
		expect(breadcrumbs[1]?.level).toBeUndefined();
	});

	test("clear empties the trail", () => {
		addTelemetryBreadcrumb({ message: "gone" });
		clearTelemetryBreadcrumbs();
		expect(getTelemetryBreadcrumbs()).toEqual([]);
	});
});

describe("captureTelemetryError", () => {
	test("buffers reports without a sink and flushes them on attach", () => {
		captureTelemetryError(new Error("early"));
		const received = collectErrors();

		expect(received).toHaveLength(1);
		expect(received[0]?.kind).toBe("Error");
		expect(received[0]?.value).toBe("early");
		expect(received[0]?.level).toBe("error");
		expect(Date.parse(received[0]?.client_ts ?? "")).not.toBeNaN();
	});

	test("drops the oldest pending reports beyond the buffer cap", () => {
		for (let index = 0; index < 40; index++) {
			captureTelemetryError(new Error(`error_${index}`));
		}
		const received = collectErrors();

		expect(received).toHaveLength(32);
		expect(received[0]?.value).toBe("error_8");
	});

	test("attaches breadcrumbs, culprit and the session id", () => {
		const removeSessionSink = setTelemetrySessionSink(() => undefined);
		const sessionId = startTelemetrySession();
		addTelemetryBreadcrumb({ category: "ui", message: "clicked run" });
		const received = collectErrors();

		const error = new Error("kaput");
		error.stack = [
			"Error: kaput",
			"    at runFlow (/home/app/src/flow.ts:4:2)",
		].join("\n");
		captureTelemetryError(error, { context: { board: "abc" } });
		removeSessionSink();

		expect(received).toHaveLength(1);
		expect(received[0]?.breadcrumbs?.[0]?.message).toBe("clicked run");
		expect(received[0]?.culprit).toBe("runFlow (flow.ts)");
		expect(received[0]?.context).toEqual({
			board: "abc",
			session_id: sessionId,
		});
	});

	test("never throws on hostile input", () => {
		const received = collectErrors();
		const hostile = {
			get message() {
				throw new Error("nope");
			},
			get name() {
				throw new Error("nope");
			},
			get stack() {
				throw new Error("nope");
			},
		};

		expect(() => captureTelemetryError(hostile)).not.toThrow();
		expect(() =>
			captureTelemetryError(new Error("with hostile context"), {
				context: {
					get boom() {
						throw new Error("nope");
					},
					token: "super-secret",
					keep: 1,
				},
			}),
		).not.toThrow();

		expect(received).toHaveLength(2);
		expect(received[0]?.kind).toBe("UnknownError");
		expect(received[1]?.context).toEqual({ keep: 1 });
	});

	test("swallows sink errors", () => {
		removeSink = setTelemetryErrorSink(() => {
			throw new Error("sink failure");
		});
		expect(() => captureTelemetryError(new Error("explodes"))).not.toThrow();
	});

	test("sanitizes context recursively and drops secret keys", () => {
		expect(
			sanitizeTelemetryContext({
				access_token: "nope",
				nested: { apiKey: "nope", path: "/apps/42?token=x", keep: true },
				list: [1, "two", { cookie: "nope", ok: 3 }],
				fn: () => undefined,
			}),
		).toEqual({
			nested: { path: "/apps/:id", keep: true },
			list: [1, "two", { ok: 3 }],
		});
	});
});

describe("telemetry client crash transport", () => {
	let clients: ITelemetryClient[] = [];

	function capturedError(value: string): ITelemetryCapturedError {
		return { kind: "Error", value, client_ts: "2026-07-26T00:00:00.000Z" };
	}

	function createHarness(
		overrides: Partial<Parameters<typeof createTelemetryClient>[0]> = {},
	) {
		const post = vi.fn().mockResolvedValue({ accepted: 0, issues: 0 });
		const client = createTelemetryClient({
			apiState: { post } as unknown as IApiState,
			getProfile: () => profile,
			isEnabled: () => true,
			getAnonId: () => "anon-1234",
			source: "desktop",
			appVersion: "1.2.3",
			platform: "linux",
			...overrides,
		});
		clients.push(client);
		return { client, post };
	}

	afterEach(() => {
		for (const client of clients) client.dispose();
		clients = [];
	});

	test("posts error batches to telemetry/errors with the locked body", async () => {
		const { client, post } = createHarness();
		client.errorSink(capturedError("boom"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post).toHaveBeenCalledWith(profile, "telemetry/errors", {
			anon_id: "anon-1234",
			source: "desktop",
			app_version: "1.2.3",
			release: "1.2.3",
			platform: "linux",
			errors: [capturedError("boom")],
		});
	});

	test("splits error batches at 20 per request", async () => {
		const { client, post } = createHarness();
		client.enqueueErrors(
			Array.from({ length: 45 }, (_, index) => capturedError(`e_${index}`)),
		);
		await client.flush();

		expect(post.mock.calls.map((call) => call[2].errors.length)).toEqual([
			20, 20, 5,
		]);
	});

	test("crash reporting has its own consent gate", async () => {
		const disabled = { value: false };
		const { client, post } = createHarness({
			isEnabled: () => false,
			isCrashEnabled: () => !disabled.value,
		});
		client.errorSink(capturedError("still sent"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post.mock.calls[0]?.[1]).toBe("telemetry/errors");

		disabled.value = true;
		client.errorSink(capturedError("dropped"));
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);
	});

	test("clear keeps crash reports, clearCrashReports drops them", async () => {
		const { client, post } = createHarness();
		client.errorSink(capturedError("kept"));
		client.clear();
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);

		client.errorSink(capturedError("dropped"));
		client.clearCrashReports();
		await client.flush();
		expect(post).toHaveBeenCalledTimes(1);
	});

	test("hands crash reports to the crash beacon on pagehide", async () => {
		const listeners = new Map<string, () => void>();
		vi.stubGlobal("window", {
			addEventListener: (name: string, handler: () => void) => {
				listeners.set(name, handler);
			},
			removeEventListener: (name: string) => {
				listeners.delete(name);
			},
		});
		const crashBeacon = vi.fn().mockReturnValue(true);
		const { client, post } = createHarness({ crashBeacon });
		client.errorSink(capturedError("unloading"));

		listeners.get("pagehide")?.();

		expect(crashBeacon).toHaveBeenCalledTimes(1);
		expect(crashBeacon.mock.calls[0]?.[1]).toBe("telemetry/errors");
		await client.flush();
		expect(post).not.toHaveBeenCalled();
		vi.unstubAllGlobals();
	});
});
