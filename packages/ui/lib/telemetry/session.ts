/**
 * Anonymous release-health sessions. A session only ever carries a random
 * session id, the install's coarse status and its duration — never user
 * identity, navigation history or content.
 */

export type TelemetrySessionStatus = "ok" | "errored" | "abnormal" | "crashed";

export interface ITelemetryCapturedSession {
	session_id: string;
	status: TelemetrySessionStatus;
	started_at: string;
	duration_ms?: number;
}

export type TelemetrySessionSink = (session: ITelemetryCapturedSession) => void;

const SESSION_STATUS_PRECEDENCE: Record<TelemetrySessionStatus, number> = {
	ok: 0,
	errored: 1,
	abnormal: 2,
	crashed: 3,
};

const MAX_PENDING_TELEMETRY_SESSIONS = 16;
const PENDING_TELEMETRY_SESSIONS: ITelemetryCapturedSession[] = [];

interface ActiveTelemetrySession {
	id: string;
	startedAt: string;
	startedAtMs: number;
	status: TelemetrySessionStatus;
}

let telemetrySessionSink: TelemetrySessionSink | undefined;
let activeSession: ActiveTelemetrySession | undefined;
let unloadHandler: (() => void) | undefined;
let unloadFlushed = false;

let sessionIdFallbackCounter = 0;

function randomSessionId(): string {
	try {
		if (
			typeof crypto !== "undefined" &&
			typeof crypto.randomUUID === "function"
		)
			return crypto.randomUUID();
		if (
			typeof crypto !== "undefined" &&
			typeof crypto.getRandomValues === "function"
		) {
			const bytes = crypto.getRandomValues(new Uint8Array(16));
			return Array.from(bytes, (byte) =>
				byte.toString(16).padStart(2, "0"),
			).join("");
		}
	} catch {
		// Fall through to the counter fallback below.
	}
	sessionIdFallbackCounter += 1;
	return `${Date.now().toString(36)}-${sessionIdFallbackCounter.toString(36)}`;
}

function deliverTelemetrySession(session: ITelemetryCapturedSession) {
	const sink = telemetrySessionSink;
	if (!sink) {
		const pendingIndex = PENDING_TELEMETRY_SESSIONS.findIndex(
			(pending) => pending.session_id === session.session_id,
		);
		if (pendingIndex >= 0) {
			PENDING_TELEMETRY_SESSIONS[pendingIndex] = session;
			return;
		}
		PENDING_TELEMETRY_SESSIONS.push(session);
		if (PENDING_TELEMETRY_SESSIONS.length > MAX_PENDING_TELEMETRY_SESSIONS) {
			PENDING_TELEMETRY_SESSIONS.shift();
		}
		return;
	}
	try {
		sink(session);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

function flushActiveSession() {
	if (!activeSession) return;
	deliverTelemetrySession({
		session_id: activeSession.id,
		status: activeSession.status,
		started_at: activeSession.startedAt,
		duration_ms: Math.max(0, Date.now() - activeSession.startedAtMs),
	});
}

/**
 * Delivers the terminal record of the active session during page unload.
 * Exported so the telemetry client can pull it before draining its beacon
 * queues: both modules register their own `pagehide` listener and listeners run
 * in registration order, so neither may depend on the other having run first.
 * Repeated calls within one unload dispatch deliver a single record; the latch
 * releases on the next microtask so a page restored from the back/forward cache
 * still reports its final duration.
 */
export function flushTelemetrySessionForUnload() {
	if (unloadFlushed) return;
	unloadFlushed = true;
	try {
		flushActiveSession();
	} finally {
		void Promise.resolve().then(() => {
			unloadFlushed = false;
		});
	}
}

function attachUnloadFlush() {
	if (unloadHandler || typeof window === "undefined") return;
	try {
		unloadHandler = flushTelemetrySessionForUnload;
		window.addEventListener("pagehide", unloadHandler);
	} catch {
		unloadHandler = undefined;
	}
}

function detachUnloadFlush() {
	if (!unloadHandler) return;
	try {
		window.removeEventListener("pagehide", unloadHandler);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
	unloadHandler = undefined;
}

function promoteSessionStatus(status: TelemetrySessionStatus) {
	if (!activeSession) return;
	if (
		SESSION_STATUS_PRECEDENCE[status] <=
		SESSION_STATUS_PRECEDENCE[activeSession.status]
	)
		return;
	activeSession.status = status;
	flushActiveSession();
}

/** Starts a session if none is active; returns the active session id. */
export function startTelemetrySession(): string {
	try {
		if (activeSession) return activeSession.id;
		const now = Date.now();
		activeSession = {
			id: randomSessionId(),
			startedAt: new Date(now).toISOString(),
			startedAtMs: now,
			status: "ok",
		};
		unloadFlushed = false;
		attachUnloadFlush();
		deliverTelemetrySession({
			session_id: activeSession.id,
			status: "ok",
			started_at: activeSession.startedAt,
		});
		return activeSession.id;
	} catch {
		return activeSession?.id ?? "";
	}
}

export function markTelemetrySessionErrored() {
	promoteSessionStatus("errored");
}

export function markTelemetrySessionCrashed() {
	promoteSessionStatus("crashed");
}

/** Flushes the final session state and detaches the active session. */
export function endTelemetrySession() {
	try {
		flushActiveSession();
	} finally {
		detachUnloadFlush();
		activeSession = undefined;
		unloadFlushed = false;
	}
}

export function getTelemetrySessionId(): string | undefined {
	return activeSession?.id;
}

export function getTelemetrySessionStatus():
	| TelemetrySessionStatus
	| undefined {
	return activeSession?.status;
}

/** Register the session sink. Pending sessions are flushed on attach. */
export function setTelemetrySessionSink(
	sink: TelemetrySessionSink | undefined,
) {
	telemetrySessionSink = sink;
	if (sink) {
		for (const session of PENDING_TELEMETRY_SESSIONS.splice(0)) {
			deliverTelemetrySession(session);
		}
	}
	return () => {
		if (telemetrySessionSink === sink) telemetrySessionSink = undefined;
	};
}
