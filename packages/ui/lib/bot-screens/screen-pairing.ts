import { fromBase64Url, randomHex, toBase64Url } from "./base64";
import type { SignalBus } from "./screen-discovery";
import { MAX_LABEL_CHARS, clampLabel } from "./screen-protocol";
import type { TrustedHost } from "./screen-trust";

export const PAIR_LINK_PREFIX = "flowlike://pair-screen?";
export const PAIR_TIMEOUT_MS = 10_000;

export interface PairingLink {
	hostPeerId: string;
	keyId: string;
	publicKey: string;
	secret: Uint8Array;
	label: string;
}

const PEER_ID = /^[A-Za-z0-9_-]{8,64}$/;
const KEY_ID = /^[A-Za-z0-9_.-]{1,64}$/;
const BAD_LINK =
	"This is not a Flow-Like screen pairing link; scan the code the desktop app shows under Watch on another device";

/** The pairing link of a host, as the QR code carries it. */
export function formatPairingLink(link: PairingLink): string {
	const query = [
		["v", "1"],
		["h", link.hostPeerId],
		["k", link.keyId],
		["p", link.publicKey],
		["s", toBase64Url(link.secret)],
		["n", link.label],
	]
		.map(([key, value]) => `${key}=${encodeURIComponent(value)}`)
		.join("&");
	return `${PAIR_LINK_PREFIX}${query}`;
}

/** Throws a sentence for anything that is not a well-formed `flowlike://pair-screen?` link. */
export function parsePairingLink(text: string): PairingLink {
	const trimmed = text.trim();
	if (!trimmed.startsWith(PAIR_LINK_PREFIX)) throw new Error(BAD_LINK);
	const query = new URLSearchParams(trimmed.slice(PAIR_LINK_PREFIX.length));
	if (query.get("v") !== "1")
		throw new Error(
			"This pairing link comes from a newer version of Flow-Like; update this app and scan it again",
		);
	const hostPeerId = query.get("h") ?? "";
	const keyId = query.get("k") ?? "";
	const publicKey = query.get("p") ?? "";
	const label = (query.get("n") ?? "").trim();
	let secret: Uint8Array;
	try {
		secret = fromBase64Url(query.get("s") ?? "");
		if (fromBase64Url(publicKey).length !== 32) throw new Error(BAD_LINK);
	} catch {
		throw new Error(BAD_LINK);
	}
	if (
		!PEER_ID.test(hostPeerId) ||
		!KEY_ID.test(keyId) ||
		secret.length !== 32 ||
		label.length > MAX_LABEL_CHARS
	)
		throw new Error(BAD_LINK);
	return { hostPeerId, keyId, publicKey, secret, label };
}

const encoder = new TextEncoder();

/** `flow_like_device_crypto::inbox::key_id_of`: `ik_` and the first 8 bytes of SHA-256 in hex. */
export async function inboxKeyId(publicKey: string): Promise<string> {
	const digest = new Uint8Array(
		await crypto.subtle.digest(
			"SHA-256",
			fromBase64Url(publicKey) as Uint8Array<ArrayBuffer>,
		),
	);
	const hex = Array.from(digest.slice(0, 8), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");
	return `ik_${hex}`;
}

export const pairMessage = (parts: {
	hostPeerId: string;
	hostKeyId: string;
	viewerKeyId: string;
	viewerPublicKey: string;
	session: string;
}) =>
	`flow-like/bot-screens/pair/v1|${parts.hostPeerId}|${parts.hostKeyId}|${parts.viewerKeyId}|${parts.viewerPublicKey}|${parts.session}`;

export const pairedMessage = (session: string) =>
	`flow-like/bot-screens/paired/v1|${session}`;

export async function pairMac(
	secret: Uint8Array,
	message: string,
): Promise<string> {
	const key = await crypto.subtle.importKey(
		"raw",
		secret as Uint8Array<ArrayBuffer>,
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	return toBase64Url(
		new Uint8Array(
			await crypto.subtle.sign("HMAC", key, encoder.encode(message)),
		),
	);
}

export interface PairIdentity {
	key_id: string;
	public_key: string;
}

export interface PairOptions {
	label: string;
	now?: () => number;
	timeoutMs?: number;
	signal?: AbortSignal;
}

const refusalText = (code: string, message: string) =>
	code === "not_paired"
		? "The desktop app refused the pairing: the code expired or was already used. Show a new code and scan it again"
		: `The desktop app refused the pairing: ${message}`;

/**
 * Proves knowledge of the QR secret to the host and, when the host proves it back, returns the
 * host as a trusted key. Nothing is pinned here.
 */
export async function pairWithHost(
	bus: SignalBus,
	link: PairingLink,
	identity: PairIdentity,
	options: PairOptions,
): Promise<TrustedHost> {
	if ((await inboxKeyId(link.publicKey)) !== link.keyId)
		throw new Error(
			"The key in this pairing link does not match its key id; show a new code on the desktop app and scan it again",
		);
	const session = randomHex(16);
	const now = options.now ?? Date.now;
	const label = clampLabel(options.label);
	let off = () => {};
	const reply = new Promise<string>((resolve, reject) => {
		const timer = setTimeout(
			() =>
				reject(
					new Error(
						"The desktop app did not answer the pairing; check that it is open and online, then show a new code",
					),
				),
			options.timeoutMs ?? PAIR_TIMEOUT_MS,
		);
		const abort = () => reject(new Error("closed"));
		options.signal?.addEventListener("abort", abort);
		off = bus.subscribe((signal) => {
			if (
				(signal.type !== "paired" && signal.type !== "refused") ||
				signal.to !== bus.viewerId ||
				signal.from !== link.hostPeerId ||
				signal.session !== session
			)
				return;
			if (signal.type === "paired") resolve(signal.mac);
			else reject(new Error(refusalText(signal.code, signal.message)));
		});
		const stop = off;
		off = () => {
			clearTimeout(timer);
			options.signal?.removeEventListener("abort", abort);
			stop();
		};
	});
	reply.catch(() => {});
	try {
		const mac = await pairMac(
			link.secret,
			pairMessage({
				hostPeerId: link.hostPeerId,
				hostKeyId: link.keyId,
				viewerKeyId: identity.key_id,
				viewerPublicKey: identity.public_key,
				session,
			}),
		);
		await bus.send({
			type: "pair",
			from: bus.viewerId,
			to: link.hostPeerId,
			session,
			key_id: identity.key_id,
			public_key: identity.public_key,
			label,
			mac,
		});
		const received = await reply;
		const expected = await pairMac(link.secret, pairedMessage(session));
		if (received !== expected)
			throw new Error(
				"The desktop app's pairing proof did not match the code; nothing was paired",
			);
	} finally {
		off();
	}
	return {
		key_id: link.keyId,
		public_key: link.publicKey,
		label: link.label,
		host_peer_id: link.hostPeerId,
		paired_at: Math.floor(now() / 1000),
	};
}
