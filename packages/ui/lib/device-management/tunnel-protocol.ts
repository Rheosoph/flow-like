import { unbase64url } from "./crypto";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export const TUNNEL_PROTOCOL = "flowlike.device-tunnel.v1";
export const TUNNEL_WINDOW = 256 * 1024;
export const TUNNEL_MAX_STREAMS = 16;
export const TUNNEL_HEADER = 20;
export const TUNNEL_MAX_DATA = 16 * 1024 - TUNNEL_HEADER;
export const TUNNEL_MAX_BUFFER = TUNNEL_WINDOW * TUNNEL_MAX_STREAMS + 65_536;
/** Largest model asset a device stores (`MODEL_ASSET_MAX_BYTES`). */
export const TUNNEL_MODEL_ASSET_MAX_BYTES = 64 * 1024 ** 3;
const JOB_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export enum TunnelKind {
	Open = 1,
	Opened = 2,
	Data = 3,
	Window = 4,
	Fin = 5,
	Reset = 6,
	Ping = 7,
	Pong = 8,
	RenewStart = 9,
	RenewReply = 10,
	RenewFinish = 11,
	Renewed = 12,
	OpenData = 13,
}

export type TunnelDataOpen =
	| {
			kind: "request";
			request: {
				operation_id: string;
				device_id: string;
				issued_at: number;
				expires_at: number;
				command: Record<string, unknown>;
			};
	  }
	| {
			kind: "artifact";
			project_id: string;
			transfer_id: string;
			file_index?: number | null;
			offset: number;
	  }
	/** Pushes the bytes of one model asset job from `offset` on. */
	| { kind: "model_asset"; job_id: string; offset: number };

export interface TunnelFrame {
	kind: TunnelKind;
	stream: number;
	sequence: bigint;
	body: Uint8Array;
}

export function encodeTunnelFrame(frame: TunnelFrame): Uint8Array {
	if (
		frame.body.length > TUNNEL_MAX_DATA ||
		frame.sequence < 0n ||
		frame.sequence > 0xffff_ffff_ffff_ffffn ||
		!Number.isInteger(frame.stream) ||
		frame.stream < 0 ||
		frame.stream > 0xffff_ffff
	)
		throw new Error("Invalid device tunnel frame.");
	const bytes = new Uint8Array(TUNNEL_HEADER + frame.body.length);
	bytes.set([70, 76, 84, 78, 1, frame.kind, 0, 0]);
	const view = new DataView(bytes.buffer);
	view.setUint32(8, frame.stream);
	view.setBigUint64(12, frame.sequence);
	bytes.set(frame.body, TUNNEL_HEADER);
	validateFrame(frame);
	return bytes;
}

export function decodeTunnelFrame(bytes: Uint8Array): TunnelFrame {
	if (
		bytes.length < TUNNEL_HEADER ||
		bytes.length > TUNNEL_HEADER + TUNNEL_MAX_DATA ||
		bytes[0] !== 70 ||
		bytes[1] !== 76 ||
		bytes[2] !== 84 ||
		bytes[3] !== 78 ||
		bytes[4] !== 1 ||
		bytes[6] !== 0 ||
		bytes[7] !== 0
	)
		throw new Error("Invalid device tunnel frame.");
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	const frame = {
		kind: bytes[5] as TunnelKind,
		stream: view.getUint32(8),
		sequence: view.getBigUint64(12),
		body: bytes.slice(TUNNEL_HEADER),
	};
	validateFrame(frame);
	return frame;
}

function validateFrame(frame: TunnelFrame): void {
	if (frame.kind < TunnelKind.Open || frame.kind > TunnelKind.OpenData)
		throw new Error("Unknown device tunnel frame.");
	const control =
		frame.kind >= TunnelKind.Ping && frame.kind <= TunnelKind.Renewed;
	if (
		control ? frame.stream !== 0 : frame.stream === 0 || frame.stream % 2 !== 1
	)
		throw new Error("Invalid device tunnel stream.");
	if (
		([TunnelKind.Opened, TunnelKind.Fin].includes(frame.kind) &&
			frame.body.length !== 0) ||
		(frame.kind === TunnelKind.Data && frame.body.length === 0) ||
		(frame.kind === TunnelKind.Window && frame.body.length !== 4) ||
		([TunnelKind.Ping, TunnelKind.Pong].includes(frame.kind) &&
			frame.body.length !== 8) ||
		([TunnelKind.RenewReply, TunnelKind.RenewFinish].includes(frame.kind) &&
			(frame.body.length === 0 || frame.body.length > 128))
	)
		throw new Error("Invalid device tunnel payload.");
	if (frame.kind === TunnelKind.Open) validateOpen(readTunnelJson(frame.body));
	if (frame.kind === TunnelKind.OpenData)
		validateDataOpen(readTunnelJson(frame.body));
	if (frame.kind === TunnelKind.Window) {
		const credit = new DataView(
			frame.body.buffer,
			frame.body.byteOffset,
			4,
		).getUint32(0);
		if (credit === 0 || credit > TUNNEL_WINDOW)
			throw new Error("Invalid device tunnel credit.");
	}
	if (frame.kind === TunnelKind.Reset) {
		const value = exactJson(frame.body, ["code", "message"]);
		if (
			typeof value.code !== "string" ||
			!/^[a-z_]{1,64}$/.test(value.code) ||
			typeof value.message !== "string" ||
			encoder.encode(value.message).length > 256
		)
			throw new Error("Invalid device tunnel reset.");
	}
	if (frame.kind === TunnelKind.RenewStart)
		validateHandshake(exactJson(frame.body, ["certificate_jws", "data"]));
	if (frame.kind === TunnelKind.Renewed) {
		const value = exactJson(frame.body, ["expires_at"]);
		if (
			typeof value.expires_at !== "number" ||
			!Number.isSafeInteger(value.expires_at) ||
			value.expires_at <= 0
		)
			throw new Error("Invalid device tunnel expiry.");
	}
}

/** A service open names its placement and service; a model gateway open names neither (Rust omits empty ids). */
function validateOpen(value: Record<string, unknown>) {
	const mode = Object.hasOwn(value, "mode") ? ["mode"] : [];
	if (Object.hasOwn(value, "target")) {
		exactFields(value, ["target", ...mode]);
		if (value.target !== "model_gateway")
			throw new Error("Invalid device tunnel target.");
	} else {
		exactFields(value, ["placement_id", "service_id", ...mode]);
		if (!identifier(value.placement_id) || !identifier(value.service_id))
			throw new Error("Invalid device tunnel service.");
	}
	if (value.mode !== undefined && value.mode !== "tcp" && value.mode !== "http")
		throw new Error("Invalid device tunnel service.");
}

function validateDataOpen(value: Record<string, unknown>): void {
	if (value.kind === "request") {
		exactFields(value, ["kind", "request"]);
		const request = value.request;
		if (!request || typeof request !== "object" || Array.isArray(request))
			throw new Error("Invalid device tunnel request.");
		const fields = request as Record<string, unknown>;
		exactFields(fields, [
			"operation_id",
			"device_id",
			"issued_at",
			"expires_at",
			"command",
		]);
		if (
			!identifier(fields.operation_id) ||
			!identifier(fields.device_id) ||
			typeof fields.issued_at !== "number" ||
			!Number.isSafeInteger(fields.issued_at) ||
			fields.issued_at <= 0 ||
			typeof fields.expires_at !== "number" ||
			!Number.isSafeInteger(fields.expires_at) ||
			fields.expires_at <= fields.issued_at ||
			fields.expires_at - fields.issued_at > 300 ||
			!fields.command ||
			typeof fields.command !== "object" ||
			Array.isArray(fields.command) ||
			typeof (fields.command as Record<string, unknown>).type !== "string" ||
			!/^[a-z_]{1,128}$/.test(
				(fields.command as Record<string, unknown>).type as string,
			)
		)
			throw new Error("Invalid device tunnel request.");
		return;
	}
	if (value.kind === "artifact") {
		exactFields(value, [
			"kind",
			"project_id",
			"transfer_id",
			"offset",
			...(Object.hasOwn(value, "file_index") ? ["file_index"] : []),
		]);
		if (
			typeof value.project_id !== "string" ||
			!/^[A-Za-z0-9_.-]{1,128}$/.test(value.project_id) ||
			value.project_id === "." ||
			value.project_id === ".." ||
			typeof value.transfer_id !== "string" ||
			!/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(
				value.transfer_id,
			) ||
			(value.file_index != null &&
				(typeof value.file_index !== "number" ||
					!Number.isSafeInteger(value.file_index) ||
					value.file_index < 0 ||
					value.file_index > 0xffff_ffff)) ||
			typeof value.offset !== "number" ||
			!Number.isSafeInteger(value.offset) ||
			value.offset < 0 ||
			value.offset > 4 * 1024 * 1024 * 1024
		)
			throw new Error("Invalid device tunnel artifact.");
		return;
	}
	if (value.kind !== "model_asset")
		throw new Error("Unknown device tunnel data stream.");
	validateModelAssetOpen(value);
}

function validateModelAssetOpen(value: Record<string, unknown>) {
	exactFields(value, ["kind", "job_id", "offset"]);
	if (
		typeof value.job_id !== "string" ||
		!JOB_ID.test(value.job_id) ||
		typeof value.offset !== "number" ||
		!Number.isSafeInteger(value.offset) ||
		value.offset < 0 ||
		value.offset > TUNNEL_MODEL_ASSET_MAX_BYTES
	)
		throw new Error("Invalid device tunnel model asset.");
}

function identifier(value: unknown): value is string {
	return typeof value === "string" && /^[A-Za-z0-9_:.-]{1,128}$/.test(value);
}

function exactJson(bytes: Uint8Array, keys: string[]): Record<string, unknown> {
	const value = readTunnelJson(bytes);
	exactFields(value, keys);
	return value;
}

function exactFields(value: Record<string, unknown>, keys: string[]): void {
	if (
		Object.keys(value).length !== keys.length ||
		keys.some((key) => !Object.hasOwn(value, key))
	)
		throw new Error("Invalid device tunnel payload fields.");
}

function validateHandshake(value: Record<string, unknown>): void {
	if (
		typeof value.certificate_jws !== "string" ||
		value.certificate_jws.length === 0 ||
		encoder.encode(value.certificate_jws).length > 8192 ||
		typeof value.data !== "string" ||
		unbase64url(value.data, 128).length === 0
	)
		throw new Error("Invalid device tunnel handshake.");
}

export function tunnelJson(value: unknown): Uint8Array {
	return encoder.encode(JSON.stringify(value));
}

export function readTunnelJson(bytes: Uint8Array): Record<string, unknown> {
	const value: unknown = JSON.parse(decoder.decode(bytes));
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new Error("Invalid device tunnel payload.");
	return value as Record<string, unknown>;
}

export type TunnelEnvelopeKind = "hello" | "handshake" | "message" | "close";
const ENVELOPE_KINDS: TunnelEnvelopeKind[] = [
	"hello",
	"handshake",
	"message",
	"close",
];

export function encodeTunnelEnvelope(
	kind: TunnelEnvelopeKind,
	sessionId: string,
	body: Uint8Array,
): Uint8Array {
	const id = encoder.encode(sessionId);
	validateEnvelopeBody(kind, sessionId, body);
	if (
		id.length === 0 ||
		id.length > 128 ||
		6 + id.length + body.length > 32_768 ||
		(kind === "close" && body.length !== 0)
	)
		throw new Error("Invalid device tunnel envelope.");
	const bytes = new Uint8Array(6 + id.length + body.length);
	bytes.set([70, 76, 84, 69, ENVELOPE_KINDS.indexOf(kind) + 1, id.length]);
	bytes.set(id, 6);
	bytes.set(body, 6 + id.length);
	return bytes;
}

export function decodeTunnelEnvelope(bytes: Uint8Array): {
	kind: TunnelEnvelopeKind;
	sessionId: string;
	body: Uint8Array;
} {
	if (
		bytes.length < 7 ||
		bytes.length > 32_768 ||
		bytes[0] !== 70 ||
		bytes[1] !== 76 ||
		bytes[2] !== 84 ||
		bytes[3] !== 69 ||
		bytes[4] < 1 ||
		bytes[4] > 4 ||
		bytes[5] === 0 ||
		bytes[5] > 128 ||
		bytes.length < 6 + bytes[5]
	)
		throw new Error("Invalid device tunnel envelope.");
	const kind = ENVELOPE_KINDS[bytes[4] - 1];
	const body = bytes.slice(6 + bytes[5]);
	const sessionId = decoder.decode(bytes.subarray(6, 6 + bytes[5]));
	validateEnvelopeBody(kind, sessionId, body);
	return {
		kind,
		sessionId,
		body,
	};
}

function validateEnvelopeBody(
	kind: TunnelEnvelopeKind,
	sessionId: string,
	body: Uint8Array,
): void {
	if (
		!identifier(sessionId) ||
		(kind === "close" && body.length !== 0) ||
		(kind === "handshake" && (body.length === 0 || body.length > 128)) ||
		(kind === "message" && (body.length < 16 || body.length > 16_400))
	)
		throw new Error("Invalid device tunnel envelope body.");
	if (kind === "hello") {
		const value = exactJson(body, ["grant_id", "certificate_jws", "data"]);
		if (!identifier(value.grant_id))
			throw new Error("Invalid device tunnel grant.");
		validateHandshake(value);
	}
}
