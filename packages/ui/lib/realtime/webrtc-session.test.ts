import { describe, expect, mock, test } from "bun:test";
import type { IRealtimeAccess } from "./types";

class FakeAwareness {
	clientID = 1;
	private localState: Record<string, unknown> | null = {};

	setLocalStateField(key: string, value: unknown): void {
		this.localState = { ...(this.localState ?? {}), [key]: value };
	}

	setLocalState(value: Record<string, unknown>): void {
		this.localState = value;
	}

	getLocalState(): Record<string, unknown> | null {
		return this.localState;
	}

	getStates(): Map<number, Record<string, unknown>> {
		return new Map([[this.clientID, this.localState ?? {}]]);
	}
}

class FakeWebrtcProvider {
	static instances: FakeWebrtcProvider[] = [];
	awareness = new FakeAwareness();
	peerOpts: Record<string, unknown>;
	signalingConns = [{ connected: true }];
	room = {
		webrtcConns: new Map<string, { destroy: () => void }>(),
	};
	disconnectCalls = 0;
	connectCalls = 0;
	destroyCalls = 0;

	constructor(
		_room: string,
		_doc: unknown,
		options: { peerOpts: Record<string, unknown> },
	) {
		this.peerOpts = options.peerOpts;
		FakeWebrtcProvider.instances.push(this);
	}

	disconnect(): void {
		this.disconnectCalls++;
	}

	connect(): void {
		this.connectCalls++;
	}

	destroy(): void {
		this.destroyCalls++;
	}
}

mock.module("y-webrtc", () => ({ WebrtcProvider: FakeWebrtcProvider }));

const { createRealtimeSession } = await import("./webrtc");

function access(credential: string): IRealtimeAccess {
	return {
		jwt: "",
		encryption_key: "room-key",
		key_id: "2026-09-02",
		ice_servers: [
			{ urls: ["stun:stun.cloudflare.com:3478"] },
			{
				urls: ["turn:turn.cloudflare.com:3478?transport=udp"],
				username: "temporary-user",
				credential,
			},
		],
	};
}

describe("shared realtime session ICE lifecycle", () => {
	test("wires initial ICE and refreshes a reused room", async () => {
		const room = `realtime-test-${crypto.randomUUID()}`;
		const providersBefore = FakeWebrtcProvider.instances.length;
		const first = await createRealtimeSession({
			room,
			access: access("first-password"),
			signalingServers: ["wss://signaling.example.com"],
		});
		const provider = FakeWebrtcProvider.instances.at(-1)!;
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("first-password").ice_servers },
		});

		let recycledPeers = 0;
		provider.room.webrtcConns.set("peer-1", {
			destroy: () => recycledPeers++,
		});
		provider.room.webrtcConns.set("peer-2", {
			destroy: () => recycledPeers++,
		});

		const second = await createRealtimeSession({
			room,
			access: access("second-password"),
			signalingServers: ["wss://signaling.example.com"],
		});
		expect(FakeWebrtcProvider.instances).toHaveLength(providersBefore + 1);
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("second-password").ice_servers },
		});
		expect(recycledPeers).toBe(2);

		first.dispose();
		first.dispose();
		expect(provider.destroyCalls).toBe(0);
		second.refreshAccess(access("third-password"));
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("third-password").ice_servers },
		});

		second.dispose();
		expect(provider.disconnectCalls).toBe(1);
		expect(provider.destroyCalls).toBe(1);
	});

	test("rejects reuse when the room encryption key changed", async () => {
		const room = `realtime-key-test-${crypto.randomUUID()}`;
		const first = await createRealtimeSession({
			room,
			access: access("first-password"),
			signalingServers: ["wss://signaling.example.com"],
		});
		const changedKey = {
			...access("second-password"),
			key_id: "2026-09-03",
		};

		expect(
			createRealtimeSession({
				room,
				access: changedKey,
				signalingServers: ["wss://signaling.example.com"],
			}),
		).rejects.toThrow("older encryption key");
		first.dispose();
	});
});
