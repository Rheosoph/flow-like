import { describe, expect, test } from "bun:test";
import { toBase64Url } from "./base64";
import {
	PAIR_LINK_PREFIX,
	formatPairingLink,
	inboxKeyId,
	pairMac,
	pairMessage,
	pairWithHost,
	pairedMessage,
	parsePairingLink,
} from "./screen-pairing";
import type { ScreenSignal } from "./screen-protocol";

const secret = new Uint8Array(32).fill(0x33);
const HOST = "0123456789abcdef0123456789abcdef";
const VIEWER = "viewer0123456789";
const PAIR_VECTOR_MESSAGE =
	"flow-like/bot-screens/pair/v1|0123456789abcdef0123456789abcdef|ik_0102030405060708|ik_a1a2a3a4a5a6a7a8|AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA|session0123456789";

const link = {
	hostPeerId: HOST,
	keyId: "ik_ae216c2ef5247a37",
	publicKey: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
	secret,
	label: "Studio Mac & Co",
};

describe("pairing link", () => {
	test("formats and parses the same fields", () => {
		const text = formatPairingLink(link);
		expect(text.startsWith(PAIR_LINK_PREFIX)).toBe(true);
		expect(text).toContain("n=Studio%20Mac%20%26%20Co");
		expect(text).toContain(`s=${toBase64Url(secret)}`);
		expect(parsePairingLink(`  ${text}\n`)).toEqual(link);
	});

	test("refuses other schemes, versions, short secrets and long labels", () => {
		const good = formatPairingLink(link);
		expect(() => parsePairingLink("https://x.test/?v=1")).toThrow(
			/pairing link/,
		);
		expect(() => parsePairingLink(good.replace("v=1", "v=2"))).toThrow(
			/newer version/,
		);
		expect(() =>
			parsePairingLink(
				formatPairingLink({ ...link, secret: new Uint8Array(8) }),
			),
		).toThrow(/pairing link/);
		expect(() =>
			parsePairingLink(formatPairingLink({ ...link, label: "x".repeat(65) })),
		).toThrow(/pairing link/);
		expect(() => parsePairingLink(good.replace(/&k=[^&]*/, ""))).toThrow(
			/pairing link/,
		);
	});
});

describe("pair mac", () => {
	test("matches the contract test vector", async () => {
		expect(
			pairMessage({
				hostPeerId: HOST,
				hostKeyId: "ik_0102030405060708",
				viewerKeyId: "ik_a1a2a3a4a5a6a7a8",
				viewerPublicKey: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
				session: "session0123456789",
			}),
		).toBe(PAIR_VECTOR_MESSAGE);
		expect(await pairMac(secret, PAIR_VECTOR_MESSAGE)).toBe(
			"s1JkC-BQQnPvhhCRptnAtNIjdcJp5M7ntT4iym56mIw",
		);
		expect(await pairMac(secret, pairedMessage("session0123456789"))).toBe(
			"IJSa8czhetKUT6kphhn_Iq0ot5nvSUhrrj11NMGEJ_Q",
		);
	});
});

function fakeBus(reply: (sent: ScreenSignal) => Promise<ScreenSignal | null>) {
	const listeners = new Set<(signal: ScreenSignal) => void>();
	const sent: ScreenSignal[] = [];
	return {
		sent,
		viewerId: VIEWER,
		subscribe(listener: (signal: ScreenSignal) => void) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		async send(message: object) {
			const signal = { ...message, at: 0 } as ScreenSignal;
			sent.push(signal);
			const answer = await reply(signal);
			if (answer) for (const listener of listeners) listener(answer);
		},
	};
}

const identity = { key_id: "ik_a1a2a3a4a5a6a7a8", public_key: "pk" };

describe("pairWithHost", () => {
	test("sends a verifiable mac and returns the pinned host", async () => {
		const bus = fakeBus(async (sent) => {
			const pair = sent as Extract<ScreenSignal, { type: "pair" }>;
			expect(pair.mac).toBe(
				await pairMac(
					secret,
					pairMessage({
						hostPeerId: HOST,
						hostKeyId: link.keyId,
						viewerKeyId: identity.key_id,
						viewerPublicKey: identity.public_key,
						session: pair.session,
					}),
				),
			);
			return {
				type: "paired",
				from: HOST,
				to: VIEWER,
				session: pair.session,
				mac: await pairMac(secret, pairedMessage(pair.session)),
				at: 0,
			};
		});
		const host = await pairWithHost(bus, link, identity, {
			label: "Phone",
			now: () => 5_000,
		});
		expect(host).toEqual({
			key_id: link.keyId,
			public_key: link.publicKey,
			label: link.label,
			host_peer_id: HOST,
			paired_at: 5,
		});
	});

	test("a bad paired mac is rejected", async () => {
		const bus = fakeBus(async (sent) => ({
			type: "paired",
			from: HOST,
			to: VIEWER,
			session: (sent as { session: string }).session,
			mac: "AAAA",
			at: 0,
		}));
		await expect(
			pairWithHost(bus, link, identity, { label: "Phone" }),
		).rejects.toThrow(/did not match/);
	});

	test("a not_paired refusal says the code expired", async () => {
		const bus = fakeBus(async (sent) => ({
			type: "refused",
			from: HOST,
			to: VIEWER,
			session: (sent as { session: string }).session,
			code: "not_paired",
			message: "",
			at: 0,
		}));
		await expect(
			pairWithHost(bus, link, identity, { label: "Phone" }),
		).rejects.toThrow(/expired or was already used/);
	});

	test("silence times out with a sentence", async () => {
		const bus = fakeBus(async () => null);
		await expect(
			pairWithHost(bus, link, identity, { label: "Phone", timeoutMs: 20 }),
		).rejects.toThrow(/did not answer the pairing/);
	});
});

describe("pairing key", () => {
	test("derives the key id like key_id_of and refuses a mismatched link", async () => {
		expect(await inboxKeyId(link.publicKey)).toBe("ik_ae216c2ef5247a37");
		const bus = {
			viewerId: "viewer0123456789",
			send: async () => {},
			subscribe: () => () => {},
		};
		await expect(
			pairWithHost(bus, { ...link, keyId: "ik_0102030405060708" }, identity, {
				label: "Phone",
			}),
		).rejects.toThrow(/does not match its key id/);
	});
});
