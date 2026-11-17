import type { IBotInputEvent } from "../../state/backend-state/bot-state";

export const PLAYGROUND_HEADER_LEN = 5;
export const PLAYGROUND_MAX_JSON = 16 * 1024;
export const PLAYGROUND_MAX_JPEG = 2 * 1024 * 1024;
export const PLAYGROUND_MAX_TEXT = 1024;
export const PLAYGROUND_MAX_CHORD = 64;

const KIND_STATE = 1;
const KIND_FRAME = 2;
const KIND_ERROR = 3;
const KIND_CONTROL = 16;

export type PlaygroundControl =
	| { type: "take_over" }
	| { type: "hand_back" }
	| { type: "stop" }
	| { type: "input"; event: IBotInputEvent };

export type PlaygroundDisplay = "virtual" | "session" | "none";
export type PlaygroundFrameKind = "browser" | "computer";

export interface PlaygroundState {
	taken_over: boolean;
	asleep: boolean;
	display: PlaygroundDisplay;
	kind?: PlaygroundFrameKind;
}

export interface PlaygroundFrameMeta {
	width: number;
	height: number;
	kind: PlaygroundFrameKind;
	caption: string;
	age_ms: number;
	observation?: unknown;
}

export type PlaygroundEvent =
	| { type: "state"; state: PlaygroundState }
	| { type: "frame"; meta: PlaygroundFrameMeta; jpeg: Uint8Array }
	| { type: "error"; code: string; message: string };

export class PlaygroundProtocolError extends Error {
	constructor(what: string) {
		super(`Invalid playground ${what}`);
		this.name = "PlaygroundProtocolError";
	}
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

function wrap(kind: number, payload: Uint8Array): Uint8Array {
	const bytes = new Uint8Array(PLAYGROUND_HEADER_LEN + payload.length);
	bytes[0] = kind;
	new DataView(bytes.buffer).setUint32(1, payload.length);
	bytes.set(payload, PLAYGROUND_HEADER_LEN);
	return bytes;
}

const finite = (...values: number[]) =>
	values.every((value) => Number.isFinite(value));

function cleanInput(event: IBotInputEvent): IBotInputEvent {
	switch (event.type) {
		case "pointer_move":
			if (!finite(event.x, event.y)) break;
			return { type: event.type, x: event.x, y: event.y };
		case "pointer_down":
		case "pointer_up":
			if (!finite(event.x, event.y)) break;
			return { type: event.type, x: event.x, y: event.y, button: event.button };
		case "scroll":
			if (!finite(event.x, event.y, event.dx, event.dy)) break;
			return {
				type: event.type,
				x: event.x,
				y: event.y,
				dx: Math.round(event.dx),
				dy: Math.round(event.dy),
			};
		case "key": {
			const length = encoder.encode(event.chord).length;
			if (length === 0 || length > PLAYGROUND_MAX_CHORD) break;
			return { type: event.type, chord: event.chord };
		}
		case "text": {
			const length = encoder.encode(event.text).length;
			if (length === 0 || length > PLAYGROUND_MAX_TEXT) break;
			return { type: event.type, text: event.text };
		}
	}
	throw new PlaygroundProtocolError("input");
}

export function encodeControl(control: PlaygroundControl): Uint8Array {
	const body =
		control.type === "input"
			? { type: "input", event: cleanInput(control.event) }
			: { type: control.type };
	const payload = encoder.encode(JSON.stringify(body));
	if (payload.length > PLAYGROUND_MAX_JSON)
		throw new PlaygroundProtocolError("message size");
	return wrap(KIND_CONTROL, payload);
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

function parseJson(payload: Uint8Array): Record<string, unknown> {
	try {
		const value: unknown = JSON.parse(decoder.decode(payload));
		if (isRecord(value)) return value;
	} catch {
		// falls through to the protocol error
	}
	throw new PlaygroundProtocolError("message");
}

const DISPLAYS: readonly string[] = ["virtual", "session", "none"];
const KINDS: readonly string[] = ["browser", "computer"];

function parseState(payload: Uint8Array): PlaygroundState {
	const json = parseJson(payload);
	if (
		typeof json.taken_over !== "boolean" ||
		typeof json.asleep !== "boolean" ||
		typeof json.display !== "string" ||
		!DISPLAYS.includes(json.display) ||
		(json.kind !== undefined &&
			(typeof json.kind !== "string" || !KINDS.includes(json.kind)))
	)
		throw new PlaygroundProtocolError("state");
	return {
		taken_over: json.taken_over,
		asleep: json.asleep,
		display: json.display as PlaygroundDisplay,
		...(json.kind === undefined
			? {}
			: { kind: json.kind as PlaygroundFrameKind }),
	};
}

function parseError(payload: Uint8Array): PlaygroundEvent {
	const json = parseJson(payload);
	if (typeof json.code !== "string" || typeof json.message !== "string")
		throw new PlaygroundProtocolError("error");
	return { type: "error", code: json.code, message: json.message };
}

function parseFrame(payload: Uint8Array): PlaygroundEvent {
	if (payload.length < 2) throw new PlaygroundProtocolError("frame");
	const metaLength = (payload[0] << 8) | payload[1];
	if (
		metaLength === 0 ||
		metaLength > PLAYGROUND_MAX_JSON ||
		payload.length <= 2 + metaLength
	)
		throw new PlaygroundProtocolError("frame");
	const json = parseJson(payload.subarray(2, 2 + metaLength));
	const jpeg = payload.slice(2 + metaLength);
	if (
		jpeg.length > PLAYGROUND_MAX_JPEG ||
		typeof json.width !== "number" ||
		typeof json.height !== "number" ||
		typeof json.caption !== "string" ||
		typeof json.age_ms !== "number" ||
		typeof json.kind !== "string" ||
		!KINDS.includes(json.kind)
	)
		throw new PlaygroundProtocolError("frame");
	return {
		type: "frame",
		meta: {
			width: json.width,
			height: json.height,
			kind: json.kind as PlaygroundFrameKind,
			caption: json.caption,
			age_ms: json.age_ms,
			...(json.observation == null ? {} : { observation: json.observation }),
		},
		jpeg,
	};
}

/**
 * Reassembles device messages from stream chunks. Malformed input throws and the stream must be
 * closed; nothing is buffered beyond the size caps.
 */
export class PlaygroundDecoder {
	private chunks: Uint8Array[] = [];
	private size = 0;

	push(bytes: Uint8Array): void {
		if (bytes.length === 0) return;
		this.chunks.push(bytes);
		this.size += bytes.length;
	}

	private take(count: number): Uint8Array {
		const out = new Uint8Array(count);
		let filled = 0;
		while (filled < count) {
			const head = this.chunks[0];
			const used = Math.min(head.length, count - filled);
			out.set(head.subarray(0, used), filled);
			filled += used;
			if (used === head.length) this.chunks.shift();
			else this.chunks[0] = head.subarray(used);
		}
		this.size -= count;
		return out;
	}

	private peekHeader(): Uint8Array {
		const header = new Uint8Array(PLAYGROUND_HEADER_LEN);
		let filled = 0;
		for (const chunk of this.chunks) {
			const used = Math.min(chunk.length, PLAYGROUND_HEADER_LEN - filled);
			header.set(chunk.subarray(0, used), filled);
			filled += used;
			if (filled === PLAYGROUND_HEADER_LEN) break;
		}
		return header;
	}

	next(): PlaygroundEvent | null {
		if (this.size < PLAYGROUND_HEADER_LEN) return null;
		const header = this.peekHeader();
		const kind = header[0];
		const length = new DataView(header.buffer).getUint32(1);
		let limit: number;
		if (kind === KIND_FRAME)
			limit = 2 + PLAYGROUND_MAX_JSON + PLAYGROUND_MAX_JPEG;
		else if (kind === KIND_STATE || kind === KIND_ERROR)
			limit = PLAYGROUND_MAX_JSON;
		else throw new PlaygroundProtocolError("message kind");
		if (length > limit) throw new PlaygroundProtocolError("message size");
		if (this.size < PLAYGROUND_HEADER_LEN + length) return null;
		this.take(PLAYGROUND_HEADER_LEN);
		const payload = this.take(length);
		if (kind === KIND_STATE)
			return { type: "state", state: parseState(payload) };
		if (kind === KIND_ERROR) return parseError(payload);
		return parseFrame(payload);
	}
}
