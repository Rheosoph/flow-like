import type { IInboxSealedEnvelope } from "../../state/backend-state/inbox-state";

export const SCREEN_CHANNEL = "flowlike.bot-screen.v1";
export const MAX_SDP_BYTES = 32 * 1024;
export const MAX_SEALED_BYTES = 48 * 1024;
export const MAX_SKEW_MS = 300_000;
export const MAX_MESSAGE_BYTES = 16 * 1024;

const PEER_ID = /^[A-Za-z0-9_-]{8,64}$/;
const BOT_ID = /^[A-Za-z0-9_.-]{1,128}$/;
const KEY_ID = /^[A-Za-z0-9_.-]{1,64}$/;
const B64URL = /^[A-Za-z0-9_-]{1,1024}$/;
export const MAX_LABEL_CHARS = 64;

/** Cuts a label to `MAX_LABEL_CHARS` UTF-16 units without splitting a surrogate pair. */
export function clampLabel(label: string): string {
	let clamped = "";
	for (const char of label) {
		if (clamped.length + char.length > MAX_LABEL_CHARS) break;
		clamped += char;
	}
	return clamped;
}

export type ScreenSignal =
	| { type: "hello"; from: string; bot_id: string; at: number }
	| {
			type: "here";
			from: string;
			to: string;
			bot_id: string;
			active: boolean;
			/** The host's person key id; absent from hosts that predate pairing. */
			key_id?: string;
			/** The host computer's name, as its pairing code shows it. */
			label?: string;
			at: number;
	  }
	| {
			type: "pair";
			from: string;
			to: string;
			session: string;
			key_id: string;
			public_key: string;
			label: string;
			mac: string;
			at: number;
	  }
	| {
			type: "paired";
			from: string;
			to: string;
			session: string;
			mac: string;
			at: number;
	  }
	| {
			type: "offer";
			from: string;
			to: string;
			bot_id: string;
			session: string;
			sealed: IInboxSealedEnvelope;
			at: number;
	  }
	| {
			type: "answer";
			from: string;
			to: string;
			session: string;
			sealed: IInboxSealedEnvelope;
			at: number;
	  }
	| {
			type: "refused";
			from: string;
			to: string;
			session: string;
			code: "busy" | "failed" | "no_key" | "not_paired";
			message: string;
			at: number;
	  };

export type ScreenSignalDraft = ScreenSignal extends infer M
	? M extends { at: number }
		? Omit<M, "at">
		: never
	: never;

const peer = (value: unknown): value is string =>
	typeof value === "string" && PEER_ID.test(value);
const bot = (value: unknown): value is string =>
	typeof value === "string" && BOT_ID.test(value);
const keyId = (value: unknown): value is string =>
	typeof value === "string" && KEY_ID.test(value);
const b64url = (value: unknown): value is string =>
	typeof value === "string" && B64URL.test(value);
const sealed = (value: unknown): value is IInboxSealedEnvelope => {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return false;
	const { v, enc, ct } = value as Record<string, unknown>;
	return (
		v === 1 &&
		typeof enc === "string" &&
		typeof ct === "string" &&
		new TextEncoder().encode(JSON.stringify(value)).length <= MAX_SEALED_BYTES
	);
};

/** Null for anything that is not a well-formed message of the contract or is outside the clock window. */
export function parseSignal(text: string, now: number): ScreenSignal | null {
	let json: Record<string, unknown>;
	try {
		const value: unknown = JSON.parse(text);
		if (typeof value !== "object" || value === null || Array.isArray(value))
			return null;
		json = value as Record<string, unknown>;
	} catch {
		return null;
	}
	const { type, from, to, at } = json;
	if (typeof at !== "number" || !Number.isFinite(at)) return null;
	if (Math.abs(now - at) > MAX_SKEW_MS || !peer(from)) return null;
	if (type === "hello")
		return bot(json.bot_id) ? { type, from, bot_id: json.bot_id, at } : null;
	if (!peer(to)) return null;
	if (type === "here")
		return bot(json.bot_id) && typeof json.active === "boolean"
			? {
					type,
					from,
					to,
					bot_id: json.bot_id,
					active: json.active,
					...(keyId(json.key_id) ? { key_id: json.key_id } : {}),
					...(typeof json.label === "string" &&
					json.label.length <= MAX_LABEL_CHARS
						? { label: json.label }
						: {}),
					at,
				}
			: null;
	if (!peer(json.session)) return null;
	const session = json.session;
	if (type === "pair")
		return keyId(json.key_id) &&
			b64url(json.public_key) &&
			b64url(json.mac) &&
			typeof json.label === "string" &&
			json.label.length <= MAX_LABEL_CHARS
			? {
					type,
					from,
					to,
					session,
					key_id: json.key_id,
					public_key: json.public_key,
					label: json.label,
					mac: json.mac,
					at,
				}
			: null;
	if (type === "paired")
		return b64url(json.mac)
			? { type, from, to, session, mac: json.mac, at }
			: null;
	if (type === "offer")
		return bot(json.bot_id) && sealed(json.sealed)
			? {
					type,
					from,
					to,
					bot_id: json.bot_id,
					session,
					sealed: json.sealed,
					at,
				}
			: null;
	if (type === "answer")
		return sealed(json.sealed)
			? { type, from, to, session, sealed: json.sealed, at }
			: null;
	if (type === "refused")
		return (json.code === "busy" ||
			json.code === "failed" ||
			json.code === "no_key" ||
			json.code === "not_paired") &&
			typeof json.message === "string"
			? {
					type,
					from,
					to,
					session,
					code: json.code,
					message: json.message.slice(0, 500),
					at,
				}
			: null;
	return null;
}
