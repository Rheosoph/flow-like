import { afterEach, describe, expect, test } from "bun:test";
import type { IInboxSealedEnvelope } from "../../state/backend-state/inbox-state";
import { fromBase64Url, toBase64Url } from "./base64";
import type { ScreenSignal, ScreenSignalDraft } from "./screen-protocol";
import { RecordCipher, deriveStreamKeys } from "./screen-records";
import {
	type ChannelLike,
	DataChannelStream,
	ScreenNotPairedError,
	connectScreen,
} from "./screen-rtc";
import { ScreenNoKeyError, type ScreenSealing } from "./screen-sealing";

class FakeChannel implements ChannelLike {
	binaryType = "blob";
	readyState = "open";
	bufferedAmount = 0;
	bufferedAmountLowThreshold = 0;
	onmessage: ChannelLike["onmessage"] = null;
	onclose: ChannelLike["onclose"] = null;
	onerror: ChannelLike["onerror"] = null;
	onbufferedamountlow: ChannelLike["onbufferedamountlow"] = null;
	sent: Uint8Array[] = [];
	closed = false;
	private readonly listeners = new Map<string, (() => void)[]>();
	send(data: Uint8Array) {
		this.sent.push(data);
	}
	close() {
		this.closed = true;
		this.readyState = "closed";
	}
	addEventListener(type: string, listener: () => void) {
		this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
	}
	emit(type: string) {
		for (const listener of this.listeners.get(type) ?? []) listener();
	}
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("DataChannelStream", () => {
	test("cuts writes into messages of at most 16 KiB", async () => {
		const channel = new FakeChannel();
		const stream = new DataChannelStream(channel);
		await stream.write(new Uint8Array(40_000));
		expect(channel.binaryType).toBe("arraybuffer");
		expect(channel.sent.map((chunk) => chunk.length)).toEqual([
			16_384, 16_384, 7232,
		]);
	});

	test("waits while the channel buffer is full", async () => {
		const channel = new FakeChannel();
		const stream = new DataChannelStream(channel);
		channel.bufferedAmount = 2 * 1024 * 1024;
		let done = false;
		const write = stream.write(new Uint8Array(10)).then(() => {
			done = true;
		});
		await tick();
		expect(done).toBe(false);
		channel.bufferedAmount = 0;
		channel.onbufferedamountlow?.();
		await write;
		expect(channel.sent.length).toBe(1);
	});

	test("reads queued and later messages, then null once closed", async () => {
		const channel = new FakeChannel();
		const stream = new DataChannelStream(channel);
		channel.onmessage?.({ data: new Uint8Array([1]).buffer });
		expect(Array.from((await stream.read()) ?? [])).toEqual([1]);
		const pending = stream.read();
		channel.onmessage?.({ data: new Uint8Array([2, 3]) });
		expect(Array.from((await pending) ?? [])).toEqual([2, 3]);
		const last = stream.read();
		channel.onclose?.();
		expect(await last).toBeNull();
		expect(await stream.read()).toBeNull();
	});

	test("a write on a closed channel fails and reset closes it", async () => {
		const channel = new FakeChannel();
		const stream = new DataChannelStream(channel);
		stream.reset();
		expect(channel.closed).toBe(true);
		expect(await stream.read()).toBeNull();
		await expect(stream.write(new Uint8Array(1))).rejects.toThrow();
	});
});

const HOST_KEY = { key_id: "ik_host", public_key: "pk-host" };

const sealing = (
	sealedTo: { key_id: string; public_key: string }[] = [],
): ScreenSealing => ({
	own: async () => ({ key_id: "ik_viewer", public_key: "pk-viewer" }),
	trusted: async () => [],
	pin: async () => {},
	unpin: async () => {},
	seal: async (_scope, body, recipient) => {
		sealedTo.push(recipient);
		return { v: 1, enc: "fake", ct: JSON.stringify(body) };
	},
	open: async (_scope, envelope) => {
		if (envelope.enc !== "fake") throw new ScreenNoKeyError();
		return JSON.parse(envelope.ct);
	},
});

class FakePeer {
	iceGatheringState = "complete";
	connectionState = "new";
	localDescription: { sdp: string } | null = null;
	channel = new FakeChannel();
	closed = false;
	remote: { type: string; sdp: string } | null = null;
	createDataChannel() {
		return this.channel;
	}
	async createOffer() {
		return { type: "offer", sdp: "offer-sdp" };
	}
	async setLocalDescription(description: { sdp: string }) {
		this.localDescription = description;
	}
	async setRemoteDescription(description: { type: string; sdp: string }) {
		this.remote = description;
		queueMicrotask(() => this.channel.emit("open"));
	}
	addEventListener() {}
	removeEventListener() {}
	close() {
		this.closed = true;
	}
}

const original = globalThis.RTCPeerConnection;
function installPeer(peer: FakePeer) {
	globalThis.RTCPeerConnection = function RTCPeerConnection() {
		return peer;
	} as never;
}
afterEach(() => {
	globalThis.RTCPeerConnection = original;
});

function bus(
	reply: (offer: Extract<ScreenSignal, { type: "offer" }>) => ScreenSignal,
) {
	const listeners = new Set<(signal: ScreenSignal) => void>();
	const sent: ScreenSignalDraft[] = [];
	return {
		sent,
		viewerId: VIEWER,
		subscribe(listener: (signal: ScreenSignal) => void) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		async send(message: ScreenSignalDraft) {
			sent.push(message);
			if (message.type === "offer") {
				const signal = reply({ ...message, at: 0 } as never);
				queueMicrotask(() => {
					for (const listener of [...listeners]) listener(signal);
				});
			}
		},
	};
}

const HOST = "hostpeer0123456789";
const VIEWER = "viewer0123456789";
const answerTo = (
	offer: Extract<ScreenSignal, { type: "offer" }>,
	secret: Uint8Array,
): ScreenSignal => ({
	type: "answer",
	from: HOST,
	to: VIEWER,
	session: offer.session,
	sealed: {
		v: 1,
		enc: "fake",
		ct: JSON.stringify({ sdp: "answer-sdp", secret: toBase64Url(secret) }),
	},
	at: 0,
});

describe("connectScreen", () => {
	test("seals the offer, derives the stream keys and confirms the key first", async () => {
		const peer = new FakePeer();
		installPeer(peer);
		const hostSecret = new Uint8Array(32).fill(0x22);
		let viewerSecret: Uint8Array = new Uint8Array(0);
		let session = "";
		const sealedTo: { key_id: string; public_key: string }[] = [];
		const fake = bus((offer) => {
			const body = JSON.parse((offer.sealed as IInboxSealedEnvelope).ct);
			expect(body.sdp).toBe("offer-sdp");
			expect(body.bot_id).toBe("bot-1");
			expect(body.key_id).toBe("ik_viewer");
			viewerSecret = fromBase64Url(body.secret);
			session = offer.session;
			return answerTo(offer, hostSecret);
		});
		const connection = await connectScreen(fake as never, {
			hostPeerId: HOST,
			hostKey: HOST_KEY,
			botId: "bot-1",
			sealing: sealing(sealedTo),
		});
		expect(sealedTo).toEqual([HOST_KEY]);
		expect(peer.remote?.sdp).toBe("answer-sdp");
		const offer = fake.sent[0] as { sdp?: string; sealed: unknown };
		expect(offer.sdp).toBeUndefined();
		expect(viewerSecret.length).toBe(32);
		await tick();
		const keys = await deriveStreamKeys(viewerSecret, hostSecret, session);
		const first = new RecordCipher(keys.viewerToHost);
		expect((await first.open(peer.channel.sent[0])).length).toBe(0);
		const host = new RecordCipher(keys.hostToViewer);
		peer.channel.onmessage?.({
			data: (await host.seal(new Uint8Array([9]))).buffer,
		});
		expect(Array.from((await connection.stream.read()) ?? [])).toEqual([9]);
		connection.close();
		expect(peer.closed).toBe(true);
	});

	test("a no_key refusal fails with a clear reason", async () => {
		const peer = new FakePeer();
		installPeer(peer);
		const fake = bus((offer) => ({
			type: "refused",
			from: HOST,
			to: VIEWER,
			session: offer.session,
			code: "no_key",
			message: "",
			at: 0,
		}));
		await expect(
			connectScreen(fake as never, {
				hostPeerId: HOST,
				hostKey: HOST_KEY,
				botId: "bot-1",
				sealing: sealing(),
			}),
		).rejects.toThrow(/could not open the end-to-end encrypted offer/);
		expect(peer.closed).toBe(true);
	});

	test("a not_paired refusal throws ScreenNotPairedError", async () => {
		const peer = new FakePeer();
		installPeer(peer);
		const fake = bus((offer) => ({
			type: "refused",
			from: HOST,
			to: VIEWER,
			session: offer.session,
			code: "not_paired",
			message: "",
			at: 0,
		}));
		await expect(
			connectScreen(fake as never, {
				hostPeerId: HOST,
				hostKey: HOST_KEY,
				botId: "bot-1",
				sealing: sealing(),
			}),
		).rejects.toBeInstanceOf(ScreenNotPairedError);
		expect(peer.closed).toBe(true);
	});
});
