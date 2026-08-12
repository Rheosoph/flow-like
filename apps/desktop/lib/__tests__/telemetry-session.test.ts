import {
	type ITelemetryClient,
	createTelemetryClient,
} from "@flow-like/flow-like-ui/lib/telemetry/client";
import {
	captureTelemetryError,
	setTelemetryErrorSink,
} from "@flow-like/flow-like-ui/lib/telemetry/errors";
import {
	type ITelemetryCapturedSession,
	endTelemetrySession,
	getTelemetrySessionId,
	getTelemetrySessionStatus,
	markTelemetrySessionCrashed,
	markTelemetrySessionErrored,
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

function collectSessions() {
	const received: ITelemetryCapturedSession[] = [];
	removeSink = setTelemetrySessionSink((session) => received.push(session));
	return received;
}

function drainPendingSessions() {
	const remove = setTelemetrySessionSink(() => undefined);
	remove();
}

afterEach(() => {
	endTelemetrySession();
	removeSink?.();
	removeSink = undefined;
	drainPendingSessions();
	vi.unstubAllGlobals();
});

describe("telemetry session", () => {
	test("starts an ok session and flushes it immediately", () => {
		const received = collectSessions();
		const sessionId = startTelemetrySession();

		expect(sessionId).toBeTruthy();
		expect(getTelemetrySessionId()).toBe(sessionId);
		expect(getTelemetrySessionStatus()).toBe("ok");
		expect(received).toHaveLength(1);
		expect(received[0]?.session_id).toBe(sessionId);
		expect(received[0]?.status).toBe("ok");
		expect(Date.parse(received[0]?.started_at ?? "")).not.toBeNaN();
		expect(received[0]?.duration_ms).toBeUndefined();
	});

	test("is idempotent while a session is active", () => {
		const received = collectSessions();
		const first = startTelemetrySession();
		const second = startTelemetrySession();

		expect(second).toBe(first);
		expect(received).toHaveLength(1);
	});

	test("promotes ok to errored and flushes once per change", () => {
		const received = collectSessions();
		startTelemetrySession();

		markTelemetrySessionErrored();
		markTelemetrySessionErrored();

		expect(getTelemetrySessionStatus()).toBe("errored");
		expect(received).toHaveLength(2);
		expect(received[1]?.status).toBe("errored");
		expect(received[1]?.duration_ms).toBeGreaterThanOrEqual(0);
	});

	test("promotes errored to crashed but never downgrades", () => {
		const received = collectSessions();
		startTelemetrySession();
		markTelemetrySessionErrored();
		markTelemetrySessionCrashed();
		markTelemetrySessionErrored();

		expect(getTelemetrySessionStatus()).toBe("crashed");
		expect(received.map((session) => session.status)).toEqual([
			"ok",
			"errored",
			"crashed",
		]);
	});

	test("a captured error promotes the session, a fatal one crashes it", () => {
		const received = collectSessions();
		const removeErrorSink = setTelemetryErrorSink(() => undefined);
		startTelemetrySession();

		captureTelemetryError(new Error("recoverable"));
		expect(getTelemetrySessionStatus()).toBe("errored");

		captureTelemetryError(new Error("fatal"), { level: "fatal" });
		expect(getTelemetrySessionStatus()).toBe("crashed");

		removeErrorSink();
		expect(received.map((session) => session.status)).toEqual([
			"ok",
			"errored",
			"crashed",
		]);
	});

	test("end flushes the final state with a duration and detaches", () => {
		const received = collectSessions();
		const sessionId = startTelemetrySession();
		markTelemetrySessionCrashed();
		endTelemetrySession();

		expect(getTelemetrySessionId()).toBeUndefined();
		expect(getTelemetrySessionStatus()).toBeUndefined();
		const last = received[received.length - 1];
		expect(last?.session_id).toBe(sessionId);
		expect(last?.status).toBe("crashed");
		expect(last?.duration_ms).toBeGreaterThanOrEqual(0);

		markTelemetrySessionErrored();
		expect(received[received.length - 1]).toBe(last);
	});

	test("buffers sessions without a sink and keeps only the latest state", () => {
		startTelemetrySession();
		markTelemetrySessionErrored();
		markTelemetrySessionCrashed();

		const received = collectSessions();
		expect(received).toHaveLength(1);
		expect(received[0]?.status).toBe("crashed");
	});

	test("flushes the active session on pagehide", () => {
		const listeners = new Map<string, () => void>();
		vi.stubGlobal("window", {
			addEventListener: (name: string, handler: () => void) => {
				listeners.set(name, handler);
			},
			removeEventListener: (name: string) => {
				listeners.delete(name);
			},
		});
		const received = collectSessions();
		startTelemetrySession();

		listeners.get("pagehide")?.();

		expect(received).toHaveLength(2);
		expect(received[1]?.duration_ms).toBeGreaterThanOrEqual(0);
	});
});

describe("telemetry client session transport", () => {
	let clients: ITelemetryClient[] = [];

	function capturedSession(
		sessionId: string,
		status: ITelemetryCapturedSession["status"] = "ok",
	): ITelemetryCapturedSession {
		return {
			session_id: sessionId,
			status,
			started_at: "2026-07-26T00:00:00.000Z",
		};
	}

	function createHarness(
		overrides: Partial<Parameters<typeof createTelemetryClient>[0]> = {},
	) {
		const post = vi.fn().mockResolvedValue({ accepted: 0 });
		const client = createTelemetryClient({
			apiState: { post } as unknown as IApiState,
			getProfile: () => profile,
			isEnabled: () => true,
			getAnonId: () => "anon-1234",
			source: "web",
			appVersion: "2.0.0",
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

	test("posts session batches to telemetry/sessions with the locked body", async () => {
		const { client, post } = createHarness({ release: "2.0.0-beta.1" });
		client.sessionSink(capturedSession("session-1"));
		await client.flush();

		expect(post).toHaveBeenCalledTimes(1);
		expect(post).toHaveBeenCalledWith(profile, "telemetry/sessions", {
			anon_id: "anon-1234",
			source: "web",
			release: "2.0.0-beta.1",
			platform: "linux",
			sessions: [capturedSession("session-1")],
		});
	});

	test("splits session batches at 50 per request", async () => {
		const { client, post } = createHarness();
		client.enqueueSessions(
			Array.from({ length: 60 }, (_, index) =>
				capturedSession(`session-${index}`),
			),
		);
		await client.flush();

		expect(post.mock.calls.map((call) => call[2].sessions.length)).toEqual([
			50, 10,
		]);
	});

	test("drops sessions when crash reporting is disabled", async () => {
		const { client, post } = createHarness({ isCrashEnabled: () => false });
		client.sessionSink(capturedSession("session-1"));
		await client.flush();

		expect(post).not.toHaveBeenCalled();
	});

	function stubPagehide() {
		const handlers: (() => void)[] = [];
		vi.stubGlobal("window", {
			addEventListener: (name: string, handler: () => void) => {
				if (name === "pagehide") handlers.push(handler);
			},
			removeEventListener: (name: string, handler: () => void) => {
				if (name !== "pagehide") return;
				const index = handlers.indexOf(handler);
				if (index >= 0) handlers.splice(index, 1);
			},
		});
		return handlers;
	}

	function beaconSessions(crashBeacon: ReturnType<typeof vi.fn>) {
		const bodies = crashBeacon.mock.calls.filter(
			(call) => call[1] === "telemetry/sessions",
		);
		expect(bodies).toHaveLength(1);
		return bodies[0][0].sessions as ITelemetryCapturedSession[];
	}

	function expectTerminalRecord(
		sessions: ITelemetryCapturedSession[],
		sessionId: string,
	) {
		const terminal = sessions.filter(
			(session) => session.duration_ms !== undefined,
		);
		expect(terminal).toHaveLength(1);
		expect(terminal[0]?.session_id).toBe(sessionId);
		expect(terminal[0]?.status).toBe("ok");
		expect(terminal[0]?.duration_ms).toBeGreaterThanOrEqual(0);
	}

	test("beacons the terminal record when the client listener runs first", async () => {
		const handlers = stubPagehide();
		const crashBeacon = vi.fn().mockReturnValue(true);
		const { client } = createHarness({ crashBeacon });
		removeSink = setTelemetrySessionSink(client.sessionSink);
		const sessionId = startTelemetrySession();
		expect(handlers).toHaveLength(2);

		for (const handler of [...handlers]) handler();

		const sessions = beaconSessions(crashBeacon);
		expect(sessions).toHaveLength(2);
		expectTerminalRecord(sessions, sessionId);
	});

	test("beacons a single terminal record when the session listener runs first", async () => {
		const handlers = stubPagehide();
		const sessionId = startTelemetrySession();
		const crashBeacon = vi.fn().mockReturnValue(true);
		const { client } = createHarness({ crashBeacon });
		removeSink = setTelemetrySessionSink(client.sessionSink);
		expect(handlers).toHaveLength(2);

		for (const handler of [...handlers]) handler();

		const sessions = beaconSessions(crashBeacon);
		expect(sessions).toHaveLength(2);
		expectTerminalRecord(sessions, sessionId);
	});
});
