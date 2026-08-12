/**
 * Privacy-safe product telemetry capture. Every event routed through this module
 * is aggregate/anonymous-only: no user ids, emails, tokens, prompts, file or
 * board contents, or any other user content — only coarse event names and
 * pre-sanitized properties keyed to a random anonymous install id.
 *
 * Sampling is applied here, at capture time, so a sampled-out event never
 * reaches the pending buffer, the client queue or the desktop SQLite buffer.
 */

import { shouldSampleEvent } from "./sampling";

export interface ITelemetryCapturedEvent {
	name: string;
	props?: Record<string, unknown>;
	client_ts: string;
}

export type TelemetryEventSink = (event: ITelemetryCapturedEvent) => void;

const PENDING_TELEMETRY_EVENTS: ITelemetryCapturedEvent[] = [];
const MAX_PENDING_TELEMETRY_EVENTS = 128;

let telemetryEventSink: TelemetryEventSink | undefined;

function deliverTelemetryEvent(event: ITelemetryCapturedEvent) {
	const sink = telemetryEventSink;
	if (!sink) {
		PENDING_TELEMETRY_EVENTS.push(event);
		if (PENDING_TELEMETRY_EVENTS.length > MAX_PENDING_TELEMETRY_EVENTS) {
			PENDING_TELEMETRY_EVENTS.shift();
		}
		return;
	}
	try {
		sink(event);
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

export function captureTelemetryEvent(
	name: string,
	props?: Record<string, unknown>,
) {
	if (!shouldSampleEvent(name)) return;
	const event: ITelemetryCapturedEvent = {
		name,
		client_ts: new Date().toISOString(),
	};
	if (props) event.props = props;
	deliverTelemetryEvent(event);
}

/** Register the telemetry sink. Pending anonymous events are flushed on attach. */
export function setTelemetryEventSink(sink: TelemetryEventSink | undefined) {
	telemetryEventSink = sink;
	if (sink) {
		for (const event of PENDING_TELEMETRY_EVENTS.splice(0)) {
			deliverTelemetryEvent(event);
		}
	}
	return () => {
		if (telemetryEventSink === sink) telemetryEventSink = undefined;
	};
}
