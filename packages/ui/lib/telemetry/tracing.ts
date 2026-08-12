/**
 * Anonymous client tracing. A span only ever carries a random trace/span id,
 * an operation name, timings and sanitized attributes — never user identity,
 * prompts, board contents or any other user content.
 */

import { sanitizeTelemetryContext } from "./errors";

export type TelemetrySpanStatus = "ok" | "error";

export type TelemetrySpanKind =
	| "server"
	| "client"
	| "internal"
	| "producer"
	| "consumer";

export interface ITelemetryCapturedSpan {
	trace_id: string;
	span_id: string;
	parent_span_id?: string;
	name: string;
	kind: TelemetrySpanKind;
	started_at: string;
	duration_ms: number;
	status: TelemetrySpanStatus;
	attributes?: Record<string, unknown>;
}

export type TelemetrySpanSink = (span: ITelemetryCapturedSpan) => void;

export interface ITelemetryTraceContext {
	traceId: string;
	spanId: string;
	sampled: boolean;
}

export interface ITelemetrySpanOptions {
	kind?: TelemetrySpanKind;
	/** Continue an existing trace given as a W3C `traceparent` header. */
	traceparent?: string;
	traceId?: string;
	parentSpanId?: string;
	attributes?: Record<string, unknown>;
	/** Overrides head sampling for this trace; children inherit the decision. */
	sampled?: boolean;
}

export interface ITelemetryActiveSpan extends ITelemetryTraceContext {
	end(status?: TelemetrySpanStatus, attributes?: Record<string, unknown>): void;
}

const TRACE_ID_LENGTH = 32;
const SPAN_ID_LENGTH = 16;
const MAX_SPAN_NAME_LENGTH = 256;
const MAX_SPAN_ATTRIBUTES_BYTES = 8 * 1024;
const MAX_ACTIVE_SPANS = 64;
const MAX_PENDING_TELEMETRY_SPANS = 64;
/** Mirrors the backend `FLOW_LIKE_TRACE_SAMPLE_RATE` default. */
const DEFAULT_TRACE_SAMPLE_RATE = 0.05;

const SPAN_KINDS: readonly TelemetrySpanKind[] = [
	"server",
	"client",
	"internal",
	"producer",
	"consumer",
];

const ZERO_HEX = /^0+$/;
const TRACEPARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;

const PENDING_TELEMETRY_SPANS: ITelemetryCapturedSpan[] = [];
const ACTIVE_SPANS: ITelemetryTraceContext[] = [];

let telemetrySpanSink: TelemetrySpanSink | undefined;
let traceSampleRate = DEFAULT_TRACE_SAMPLE_RATE;

function monotonicNow(): number {
	try {
		if (
			typeof performance !== "undefined" &&
			typeof performance.now === "function"
		)
			return performance.now();
	} catch {
		// Fall through to the wall clock below.
	}
	return Date.now();
}

function randomHex(length: number): string {
	let hex = "";
	try {
		if (
			typeof crypto !== "undefined" &&
			typeof crypto.getRandomValues === "function"
		) {
			const bytes = new Uint8Array(length / 2);
			crypto.getRandomValues(bytes);
			for (const byte of bytes) hex += byte.toString(16).padStart(2, "0");
		}
	} catch {
		hex = "";
	}
	while (hex.length < length)
		hex += Math.floor(Math.random() * 16).toString(16);
	hex = hex.slice(0, length);
	return ZERO_HEX.test(hex) ? `${hex.slice(0, -1)}1` : hex;
}

function normalizeHex(
	value: string | undefined,
	length: number,
): string | undefined {
	if (typeof value !== "string") return undefined;
	const lowered = value.trim().toLowerCase();
	if (lowered.length !== length) return undefined;
	if (!/^[0-9a-f]+$/.test(lowered) || ZERO_HEX.test(lowered)) return undefined;
	return lowered;
}

export function formatTraceparent(
	traceId: string,
	spanId: string,
	sampled: boolean,
): string {
	return `00-${traceId}-${spanId}-${sampled ? "01" : "00"}`;
}

export function parseTraceparent(
	header: string | undefined,
): ITelemetryTraceContext | undefined {
	try {
		if (typeof header !== "string") return undefined;
		const match = TRACEPARENT.exec(header.trim().toLowerCase());
		if (!match) return undefined;
		const traceId = normalizeHex(match[1], TRACE_ID_LENGTH);
		const spanId = normalizeHex(match[2], SPAN_ID_LENGTH);
		if (!traceId || !spanId) return undefined;
		return {
			traceId,
			spanId,
			sampled: (Number.parseInt(match[3] ?? "00", 16) & 0x01) === 0x01,
		};
	} catch {
		return undefined;
	}
}

/** Head-based sampling: the rate is consulted once per trace root. */
export function setTelemetryTraceSampleRate(rate: number) {
	if (typeof rate !== "number" || !Number.isFinite(rate)) return;
	traceSampleRate = Math.min(1, Math.max(0, rate));
}

export function getTelemetryTraceSampleRate(): number {
	return traceSampleRate;
}

function shouldSampleTrace(): boolean {
	if (traceSampleRate >= 1) return true;
	if (traceSampleRate <= 0) return false;
	return Math.random() < traceSampleRate;
}

export function getActiveTraceContext(): ITelemetryTraceContext | undefined {
	const active = ACTIVE_SPANS[ACTIVE_SPANS.length - 1];
	return active ? { ...active } : undefined;
}

/** W3C header for the given (or active) context, for outgoing API calls. */
export function getTelemetryTraceparent(
	context?: ITelemetryTraceContext,
): string | undefined {
	const resolved = context ?? getActiveTraceContext();
	if (!resolved) return undefined;
	return formatTraceparent(resolved.traceId, resolved.spanId, resolved.sampled);
}

function deliverSpan(span: ITelemetryCapturedSpan) {
	const sink = telemetrySpanSink;
	if (!sink) {
		PENDING_TELEMETRY_SPANS.push(span);
		if (PENDING_TELEMETRY_SPANS.length > MAX_PENDING_TELEMETRY_SPANS) {
			PENDING_TELEMETRY_SPANS.shift();
		}
		return;
	}
	try {
		sink(span);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

function normalizeName(name: string): string {
	const trimmed = typeof name === "string" ? name.trim() : "";
	return (trimmed.length > 0 ? trimmed : "unnamed").slice(
		0,
		MAX_SPAN_NAME_LENGTH,
	);
}

function normalizeKind(kind: TelemetrySpanKind | undefined): TelemetrySpanKind {
	return kind && SPAN_KINDS.includes(kind) ? kind : "client";
}

function normalizeStatus(
	status: TelemetrySpanStatus | undefined,
): TelemetrySpanStatus {
	return status === "error" ? "error" : "ok";
}

/** Bounded, secret-free attributes; dropped entirely when over the ingest cap. */
function normalizeAttributes(
	base: Record<string, unknown> | undefined,
	extra: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
	try {
		if (!base && !extra) return undefined;
		const merged = sanitizeTelemetryContext({
			...(base ?? {}),
			...(extra ?? {}),
		});
		if (Object.keys(merged).length === 0) return undefined;
		const encoded = JSON.stringify(merged);
		if (
			typeof encoded !== "string" ||
			encoded.length > MAX_SPAN_ATTRIBUTES_BYTES
		)
			return undefined;
		return merged;
	} catch {
		return undefined;
	}
}

interface IParentSpanContext {
	traceId: string;
	spanId?: string;
	sampled?: boolean;
}

function resolveParent(
	options: ITelemetrySpanOptions | undefined,
): IParentSpanContext | undefined {
	const fromHeader = parseTraceparent(options?.traceparent);
	if (fromHeader) return fromHeader;
	const traceId = normalizeHex(options?.traceId, TRACE_ID_LENGTH);
	if (traceId) {
		return {
			traceId,
			spanId: normalizeHex(options?.parentSpanId, SPAN_ID_LENGTH),
		};
	}
	return ACTIVE_SPANS[ACTIVE_SPANS.length - 1];
}

function inertSpan(): ITelemetryActiveSpan {
	return {
		traceId: "",
		spanId: "",
		sampled: false,
		end: () => undefined,
	};
}

/**
 * Starts a client span. Child spans inherit the trace and the head-sampling
 * decision of the currently active span. Never throws into the application
 * path; `end()` is idempotent and only emits when the trace is sampled.
 */
export function startTelemetrySpan(
	name: string,
	options?: ITelemetrySpanOptions,
): ITelemetryActiveSpan {
	try {
		const parent = resolveParent(options);
		const traceId = parent?.traceId ?? randomHex(TRACE_ID_LENGTH);
		const spanId = randomHex(SPAN_ID_LENGTH);
		const parentSpanId =
			parent?.spanId && parent.spanId.length > 0 ? parent.spanId : undefined;
		const sampled = options?.sampled ?? parent?.sampled ?? shouldSampleTrace();
		const context: ITelemetryTraceContext = { traceId, spanId, sampled };

		ACTIVE_SPANS.push(context);
		if (ACTIVE_SPANS.length > MAX_ACTIVE_SPANS) ACTIVE_SPANS.shift();

		const startedAt = new Date().toISOString();
		const startedMs = monotonicNow();
		let ended = false;

		return {
			traceId,
			spanId,
			sampled,
			end: (status, attributes) => {
				try {
					if (ended) return;
					ended = true;
					const index = ACTIVE_SPANS.indexOf(context);
					if (index >= 0) ACTIVE_SPANS.splice(index, 1);
					if (!sampled) return;
					const captured: ITelemetryCapturedSpan = {
						trace_id: traceId,
						span_id: spanId,
						name: normalizeName(name),
						kind: normalizeKind(options?.kind),
						started_at: startedAt,
						duration_ms: Math.max(0, Math.round(monotonicNow() - startedMs)),
						status: normalizeStatus(status),
					};
					if (parentSpanId) captured.parent_span_id = parentSpanId;
					const normalized = normalizeAttributes(
						options?.attributes,
						attributes,
					);
					if (normalized) captured.attributes = normalized;
					deliverSpan(captured);
				} catch {
					// Telemetry is best-effort and must never affect the application path.
				}
			},
		};
	} catch {
		return inertSpan();
	}
}

/** Drops any spans still marked active, e.g. after a navigation. */
export function clearActiveTelemetrySpans() {
	ACTIVE_SPANS.length = 0;
}

/** Register the span sink. Pending spans are flushed on attach. */
export function setTelemetrySpanSink(sink: TelemetrySpanSink | undefined) {
	telemetrySpanSink = sink;
	if (sink) {
		for (const span of PENDING_TELEMETRY_SPANS.splice(0)) {
			deliverSpan(span);
		}
	}
	return () => {
		if (telemetrySpanSink === sink) telemetrySpanSink = undefined;
	};
}
