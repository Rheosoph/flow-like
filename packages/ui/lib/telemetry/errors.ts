/**
 * Anonymous crash and error capture. Every report routed through this module is
 * keyed to the random anonymous install id only: no user id, no account data
 * and no user content beyond a sanitized exception value, a stack trace and
 * pre-sanitized breadcrumbs.
 */

import {
	type ITelemetryCapturedBreadcrumb,
	getTelemetryBreadcrumbs,
	sanitizeTelemetryMessage,
} from "./breadcrumbs";
import {
	getTelemetrySessionId,
	markTelemetrySessionCrashed,
	markTelemetrySessionErrored,
} from "./session";

export type TelemetryErrorLevel = "error" | "fatal" | "warning";

export interface ITelemetryCapturedFrame {
	function?: string;
	file?: string;
	lineno?: number;
	colno?: number;
	in_app?: boolean;
}

export interface ITelemetryCapturedError {
	kind: string;
	value: string;
	level?: TelemetryErrorLevel;
	culprit?: string;
	stacktrace?: ITelemetryCapturedFrame[];
	breadcrumbs?: ITelemetryCapturedBreadcrumb[];
	context?: Record<string, unknown>;
	client_ts: string;
}

export interface ITelemetryErrorOptions {
	level?: TelemetryErrorLevel;
	culprit?: string;
	context?: Record<string, unknown>;
}

export interface INormalizedTelemetryError {
	kind: string;
	value: string;
	stack?: string;
}

export type TelemetryErrorSink = (error: ITelemetryCapturedError) => void;

const MAX_STACK_FRAMES = 100;
const MAX_KIND_LENGTH = 128;
const MAX_VALUE_LENGTH = 512;
const MAX_CULPRIT_LENGTH = 200;
const MAX_FUNCTION_LENGTH = 200;
const MAX_FILE_LENGTH = 256;
const MAX_CONTEXT_DEPTH = 3;
const MAX_CONTEXT_ENTRIES = 30;
const MAX_CONTEXT_ARRAY_ITEMS = 20;
const MAX_PENDING_TELEMETRY_ERRORS = 32;

const V8_FRAME = /^at\s+(.*?)\s*(?:\((.*)\))?$/;
const MOZ_FRAME = /^(.*?)@(.*)$/;
const LOCATION_WITH_POSITION = /^(.*?):(\d+):(\d+)$/;
const LOCATION_WITH_LINE = /^(.*?):(\d+)$/;
const IDENTIFIER_LIKE = /^[A-Za-z_$][\w$.]*$/;

const NATIVE_LOCATIONS = new Set([
	"native",
	"[native code]",
	"<anonymous>",
	"unknown location",
	"[wasm code]",
	"eval",
	"module code",
]);

const SECRET_CONTEXT_KEYS = [
	"password",
	"passwd",
	"pwd",
	"secret",
	"token",
	"apikey",
	"api_key",
	"authorization",
	"cookie",
	"credential",
	"signature",
];

const PENDING_TELEMETRY_ERRORS: ITelemetryCapturedError[] = [];

let telemetryErrorSink: TelemetryErrorSink | undefined;

function safeRead(target: unknown, key: string): unknown {
	try {
		return (target as Record<string, unknown>)[key];
	} catch {
		return undefined;
	}
}

function safeString(value: unknown): string {
	try {
		if (typeof value === "string") return value;
		if (typeof value === "object" && value !== null) {
			const json = JSON.stringify(value);
			if (typeof json === "string") return json;
		}
		return String(value);
	} catch {
		return "[unserializable]";
	}
}

function basename(file: string): string {
	const parts = file.split(/[/\\]/);
	return parts[parts.length - 1] || file;
}

function sanitizeFrameFile(file: string): string {
	return (file.split(/[?#]/, 1)[0] ?? file).trim().slice(0, MAX_FILE_LENGTH);
}

function isNativeLocation(file: string): boolean {
	return NATIVE_LOCATIONS.has(file.trim().toLowerCase());
}

/** In-app frames are the ones owned by the product, not dependencies or the engine. */
function isInAppFile(file: string | undefined): boolean {
	if (!file || file.length === 0) return false;
	if (isNativeLocation(file)) return false;
	return !file.includes("node_modules/") && !file.includes("node_modules\\");
}

function parseFrameLocation(raw: string): {
	file?: string;
	lineno?: number;
	colno?: number;
} {
	const trimmed = raw.trim();
	if (trimmed.length === 0) return {};
	const withPosition = LOCATION_WITH_POSITION.exec(trimmed);
	if (withPosition?.[1]) {
		return {
			file: sanitizeFrameFile(withPosition[1]),
			lineno: Number(withPosition[2]),
			colno: Number(withPosition[3]),
		};
	}
	const withLine = LOCATION_WITH_LINE.exec(trimmed);
	if (withLine?.[1]) {
		return {
			file: sanitizeFrameFile(withLine[1]),
			lineno: Number(withLine[2]),
		};
	}
	return { file: sanitizeFrameFile(trimmed) };
}

function normalizeFunctionName(raw: string | undefined): string | undefined {
	if (!raw) return undefined;
	const name = raw.replace(/^async\s+/, "").trim();
	if (
		name.length === 0 ||
		name === "<anonymous>" ||
		name === "Object.<anonymous>"
	)
		return undefined;
	return name.slice(0, MAX_FUNCTION_LENGTH);
}

function looksLikeLocation(candidate: string): boolean {
	const trimmed = candidate.trim();
	if (trimmed.length === 0) return false;
	if (isNativeLocation(trimmed)) return true;
	return (
		trimmed.includes("/") ||
		trimmed.includes("\\") ||
		trimmed.includes("://") ||
		LOCATION_WITH_POSITION.test(trimmed) ||
		LOCATION_WITH_LINE.test(trimmed)
	);
}

function buildFrame(
	functionName: string | undefined,
	location: string | undefined,
): ITelemetryCapturedFrame | undefined {
	const parsed = location ? parseFrameLocation(location) : {};
	const fn = normalizeFunctionName(functionName);
	if (!fn && !parsed.file) return undefined;
	const frame: ITelemetryCapturedFrame = { in_app: isInAppFile(parsed.file) };
	if (fn) frame.function = fn;
	if (parsed.file) frame.file = parsed.file;
	if (parsed.lineno !== undefined) frame.lineno = parsed.lineno;
	if (parsed.colno !== undefined) frame.colno = parsed.colno;
	return frame;
}

function parseV8Frame(line: string): ITelemetryCapturedFrame | undefined {
	const match = V8_FRAME.exec(line);
	if (!match) return undefined;
	const head = match[1] ?? "";
	const parenthesized = match[2];
	if (parenthesized !== undefined) return buildFrame(head, parenthesized);
	return looksLikeLocation(head)
		? buildFrame(undefined, head)
		: buildFrame(head, undefined);
}

function parseMozFrame(line: string): ITelemetryCapturedFrame | undefined {
	const match = MOZ_FRAME.exec(line);
	if (!match) return undefined;
	const location = match[2] ?? "";
	if (!looksLikeLocation(location)) return undefined;
	return buildFrame(match[1], location);
}

/** Parses V8 ("at fn (file:1:2)") and Firefox/Safari ("fn@file:1:2") stacks. */
export function parseErrorFrames(
	stack: string | undefined,
): ITelemetryCapturedFrame[] {
	const frames: ITelemetryCapturedFrame[] = [];
	try {
		if (typeof stack !== "string" || stack.length === 0) return frames;
		for (const rawLine of stack.split("\n")) {
			if (frames.length >= MAX_STACK_FRAMES) break;
			const line = rawLine.trim();
			if (line.length === 0) continue;
			const frame = parseV8Frame(line) ?? parseMozFrame(line);
			if (frame) frames.push(frame);
		}
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
	return frames;
}

function normalizeKind(raw: unknown, fallback: string): string {
	const kind = typeof raw === "string" ? raw.trim() : "";
	return (kind.length > 0 ? kind : fallback).slice(0, MAX_KIND_LENGTH);
}

function constructorName(error: unknown): string | undefined {
	const name = safeRead(safeRead(error, "constructor"), "name");
	return typeof name === "string" &&
		IDENTIFIER_LIKE.test(name) &&
		name !== "Object"
		? name
		: undefined;
}

/** Turns anything a `catch` can yield into a stable kind/value/stack triple. */
export function normalizeError(error: unknown): INormalizedTelemetryError {
	try {
		if (typeof error === "string") {
			return { kind: "Error", value: sanitizeValue(error) };
		}
		if (
			error instanceof Error ||
			typeof safeRead(error, "message") === "string"
		) {
			const stack = safeRead(error, "stack");
			return {
				kind: normalizeKind(
					safeRead(error, "name"),
					constructorName(error) ?? "Error",
				),
				value: sanitizeValue(safeString(safeRead(error, "message"))),
				stack: typeof stack === "string" ? stack : undefined,
			};
		}
		if (error !== null && typeof error === "object") {
			return {
				kind: normalizeKind(
					safeRead(error, "name"),
					constructorName(error) ?? "UnknownError",
				),
				value: sanitizeValue(`Non-Error exception: ${safeString(error)}`),
			};
		}
		return {
			kind: "UnknownError",
			value: sanitizeValue(`Non-Error exception: ${safeString(error)}`),
		};
	} catch {
		return { kind: "UnknownError", value: "Unparseable exception" };
	}
}

function sanitizeValue(value: string): string {
	return sanitizeTelemetryMessage(value, MAX_VALUE_LENGTH);
}

function isSecretKey(key: string): boolean {
	const lowered = key.toLowerCase();
	return SECRET_CONTEXT_KEYS.some((secret) => lowered.includes(secret));
}

function sanitizeContextValue(value: unknown, depth: number): unknown {
	if (value === null) return null;
	if (typeof value === "string") return sanitizeTelemetryMessage(value);
	if (typeof value === "number")
		return Number.isFinite(value) ? value : undefined;
	if (typeof value === "boolean") return value;
	if (Array.isArray(value)) {
		if (depth >= MAX_CONTEXT_DEPTH) return undefined;
		return value
			.slice(0, MAX_CONTEXT_ARRAY_ITEMS)
			.map((item) => sanitizeContextValue(item, depth + 1))
			.filter((item) => item !== undefined);
	}
	if (typeof value === "object") {
		if (depth >= MAX_CONTEXT_DEPTH) return undefined;
		return sanitizeContextObject(value as Record<string, unknown>, depth + 1);
	}
	return undefined;
}

/** Bounded, secret-dropping copy of caller-supplied context. Never throws. */
export function sanitizeTelemetryContext(
	context: Record<string, unknown>,
): Record<string, unknown> {
	return sanitizeContextObject(context, 0);
}

function sanitizeContextObject(
	context: Record<string, unknown>,
	depth: number,
): Record<string, unknown> {
	const sanitized: Record<string, unknown> = {};
	let keys: string[] = [];
	try {
		keys = Object.keys(context).slice(0, MAX_CONTEXT_ENTRIES);
	} catch {
		return sanitized;
	}
	for (const key of keys) {
		if (isSecretKey(key)) continue;
		const value = sanitizeContextValue(safeRead(context, key), depth);
		if (value !== undefined) sanitized[key] = value;
	}
	return sanitized;
}

function deriveCulprit(frames: ITelemetryCapturedFrame[]): string | undefined {
	const frame = frames.find((candidate) => candidate.in_app) ?? frames[0];
	if (!frame) return undefined;
	const file = frame.file ? basename(frame.file) : undefined;
	const culprit =
		frame.function && file
			? `${frame.function} (${file})`
			: (frame.function ?? file);
	return culprit?.slice(0, MAX_CULPRIT_LENGTH);
}

function deliverTelemetryError(error: ITelemetryCapturedError) {
	const sink = telemetryErrorSink;
	if (!sink) {
		PENDING_TELEMETRY_ERRORS.push(error);
		if (PENDING_TELEMETRY_ERRORS.length > MAX_PENDING_TELEMETRY_ERRORS) {
			PENDING_TELEMETRY_ERRORS.shift();
		}
		return;
	}
	try {
		sink(error);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/** Captures an anonymous crash report. Never throws into the application path. */
export function captureTelemetryError(
	error: unknown,
	options?: ITelemetryErrorOptions,
) {
	try {
		const normalized = normalizeError(error);
		const level = options?.level ?? "error";
		const frames = parseErrorFrames(normalized.stack);
		const breadcrumbs = getTelemetryBreadcrumbs();
		const captured: ITelemetryCapturedError = {
			kind: normalized.kind,
			value: normalized.value,
			level,
			client_ts: new Date().toISOString(),
		};

		const culprit =
			typeof options?.culprit === "string" && options.culprit.length > 0
				? sanitizeTelemetryMessage(options.culprit, MAX_CULPRIT_LENGTH)
				: deriveCulprit(frames);
		if (culprit) captured.culprit = culprit;
		if (frames.length > 0) captured.stacktrace = frames;
		if (breadcrumbs.length > 0) captured.breadcrumbs = breadcrumbs;

		const context = options?.context
			? sanitizeTelemetryContext(options.context)
			: {};
		const sessionId = getTelemetrySessionId();
		if (sessionId) context.session_id = sessionId;
		if (Object.keys(context).length > 0) captured.context = context;

		if (level === "fatal") markTelemetrySessionCrashed();
		else markTelemetrySessionErrored();

		deliverTelemetryError(captured);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/** Register the crash report sink. Pending reports are flushed on attach. */
export function setTelemetryErrorSink(sink: TelemetryErrorSink | undefined) {
	telemetryErrorSink = sink;
	if (sink) {
		for (const error of PENDING_TELEMETRY_ERRORS.splice(0)) {
			deliverTelemetryError(error);
		}
	}
	return () => {
		if (telemetryErrorSink === sink) telemetryErrorSink = undefined;
	};
}
