import { fromBase64Url, randomHex, toBase64Url } from "./base64";

export interface ScreenKey {
	id: string;
	key: string;
}

export interface ScreenEnvelope {
	v: 1;
	key_id: string;
	iv: string;
	ct: string;
}

export type OpenedEnvelope =
	| { status: "ok"; text: string }
	| { status: "unknown_key"; keyId: string }
	| { status: "invalid" };

const encoder = new TextEncoder();

const aad = (room: string, keyId: string) =>
	encoder.encode(`flow-like/bot-screens/v1|${room}|${keyId}`);

function importKey(key: string, usage: KeyUsage): Promise<CryptoKey> {
	return crypto.subtle.importKey(
		"raw",
		fromBase64Url(key) as BufferSource,
		"AES-GCM",
		false,
		[usage],
	);
}

export async function sealEnvelope(
	key: ScreenKey,
	room: string,
	plaintext: string,
	iv: Uint8Array = crypto.getRandomValues(new Uint8Array(12)),
): Promise<ScreenEnvelope> {
	const sealed = await crypto.subtle.encrypt(
		{
			name: "AES-GCM",
			iv: iv as BufferSource,
			additionalData: aad(room, key.id) as BufferSource,
		},
		await importKey(key.key, "encrypt"),
		encoder.encode(plaintext),
	);
	return {
		v: 1,
		key_id: key.id,
		iv: toBase64Url(iv),
		ct: toBase64Url(new Uint8Array(sealed)),
	};
}

function asEnvelope(data: unknown): ScreenEnvelope | null {
	if (typeof data !== "object" || data === null) return null;
	const { v, key_id, iv, ct } = data as Record<string, unknown>;
	if (
		v !== 1 ||
		typeof key_id !== "string" ||
		typeof iv !== "string" ||
		typeof ct !== "string" ||
		key_id.length > 64 ||
		ct.length > 64 * 1024
	)
		return null;
	return { v, key_id, iv, ct };
}

export async function openEnvelope(
	data: unknown,
	room: string,
	keys: readonly ScreenKey[],
): Promise<OpenedEnvelope> {
	const envelope = asEnvelope(data);
	if (!envelope) return { status: "invalid" };
	const key = keys.find((candidate) => candidate.id === envelope.key_id);
	if (!key) return { status: "unknown_key", keyId: envelope.key_id };
	try {
		const iv = fromBase64Url(envelope.iv);
		if (iv.length !== 12) return { status: "invalid" };
		const opened = await crypto.subtle.decrypt(
			{
				name: "AES-GCM",
				iv: iv as BufferSource,
				additionalData: aad(room, key.id) as BufferSource,
			},
			await importKey(key.key, "decrypt"),
			fromBase64Url(envelope.ct) as BufferSource,
		);
		return {
			status: "ok",
			text: new TextDecoder("utf-8", { fatal: true }).decode(opened),
		};
	} catch {
		return { status: "invalid" };
	}
}

export const newPeerId = () => randomHex(16);
