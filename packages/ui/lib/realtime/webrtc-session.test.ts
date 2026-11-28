import {
	afterAll,
	afterEach,
	describe,
	expect,
	jest,
	mock,
	test,
} from "bun:test";
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

interface FakeConnection {
	peer?: unknown;
	destroy: () => void;
}

class FakeWebrtcProvider {
	static instances: FakeWebrtcProvider[] = [];
	awareness = new FakeAwareness();
	peerOpts: Record<string, unknown>;
	signalingConns = [{ connected: true }];
	room = {
		webrtcConns: new Map<string, FakeConnection>(),
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

// bun keeps a module mock for every later file in the process, so the real module is
// captured first and put back in afterAll.
const actualYWebrtc = { ...(await import("y-webrtc")) };
mock.module("y-webrtc", () => ({
	...actualYWebrtc,
	WebrtcProvider: FakeWebrtcProvider,
}));
afterAll(() => {
	mock.module("y-webrtc", () => actualYWebrtc);
});
afterEach(() => {
	jest.useRealTimers();
});

const { createRealtimeSession } = await import("./webrtc");

const signalingServers = ["wss://signaling.example.com"];

function access(credential: string, iceExpiresAt?: number): IRealtimeAccess {
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
		...(iceExpiresAt === undefined ? {} : { ice_expires_at: iceExpiresAt }),
	};
}

/** A connection whose statistics name the kind of candidate it sends from. */
function connection(candidateType: string, onDestroy: () => void) {
	const reports = [
		{ id: "local", type: "local-candidate", candidateType },
		{ id: "pair", type: "candidate-pair", localCandidateId: "local" },
		{ id: "transport", type: "transport", selectedCandidatePairId: "pair" },
	];
	return {
		peer: {
			_pc: {
				getStats: async () => ({
					forEach(visit: (report: Record<string, unknown>) => void) {
						for (const report of reports) visit(report);
					},
				}),
			},
		},
		destroy: onDestroy,
	};
}

async function settled(): Promise<void> {
	for (let turn = 0; turn < 20; turn++) await Promise.resolve();
}

function latestProvider(): FakeWebrtcProvider {
	const provider = FakeWebrtcProvider.instances.at(-1);
	if (!provider) throw new Error("No provider was created");
	return provider;
}

describe("shared realtime session ICE lifecycle", () => {
	test("a second consumer's credentials leave the room's live connections running", async () => {
		const room = `realtime-test-${crypto.randomUUID()}`;
		const providersBefore = FakeWebrtcProvider.instances.length;
		const first = await createRealtimeSession({
			room,
			access: access("first-password"),
			signalingServers,
		});
		const provider = latestProvider();
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("first-password").ice_servers },
		});

		let closedPeers = 0;
		provider.room.webrtcConns.set("peer-1", { destroy: () => closedPeers++ });
		provider.room.webrtcConns.set("peer-2", { destroy: () => closedPeers++ });

		const second = await createRealtimeSession({
			room,
			access: access("second-password"),
			signalingServers,
		});
		expect(FakeWebrtcProvider.instances).toHaveLength(providersBefore + 1);
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("second-password").ice_servers },
		});
		expect(closedPeers).toBe(0);

		first.dispose();
		first.dispose();
		expect(provider.destroyCalls).toBe(0);
		second.refreshAccess(access("third-password"));
		expect(provider.peerOpts).toEqual({
			config: { iceServers: access("third-password").ice_servers },
		});
		expect(closedPeers).toBe(0);

		second.dispose();
		expect(provider.disconnectCalls).toBe(1);
		expect(provider.destroyCalls).toBe(1);
	});

	test("a relayed connection is replaced a minute before its credential lapses", async () => {
		jest.useFakeTimers();
		const room = `realtime-relay-test-${crypto.randomUUID()}`;
		const lapsesAt = Math.floor(Date.now() / 1000) + 10 * 60;
		const session = await createRealtimeSession({
			room,
			access: access("first-password", lapsesAt),
			signalingServers,
		});
		const provider = latestProvider();
		const closed: string[] = [];
		provider.room.webrtcConns.set(
			"relayed",
			connection("relay", () => closed.push("relayed")),
		);
		provider.room.webrtcConns.set(
			"direct",
			connection("host", () => closed.push("direct")),
		);

		session.refreshAccess(access("second-password", lapsesAt + 4 * 60 * 60));
		provider.room.webrtcConns.set(
			"later",
			connection("relay", () => closed.push("later")),
		);
		jest.advanceTimersByTime(9 * 60_000 - 1_000);
		await settled();
		expect(closed).toEqual([]);

		jest.advanceTimersByTime(2_000);
		await settled();
		expect(closed).toEqual(["relayed"]);
		session.dispose();
	});

	test("a released room cancels the replacement it had planned", async () => {
		jest.useFakeTimers();
		const room = `realtime-release-test-${crypto.randomUUID()}`;
		const lapsesAt = Math.floor(Date.now() / 1000) + 10 * 60;
		const session = await createRealtimeSession({
			room,
			access: access("first-password", lapsesAt),
			signalingServers,
		});
		const provider = latestProvider();
		let closed = 0;
		provider.room.webrtcConns.set(
			"relayed",
			connection("relay", () => closed++),
		);
		session.refreshAccess(access("second-password", lapsesAt + 4 * 60 * 60));
		session.dispose();

		jest.advanceTimersByTime(20 * 60_000);
		await settled();
		expect(closed).toBe(0);
	});

	test("two consumers opening a room at once share one provider", async () => {
		const room = `realtime-race-test-${crypto.randomUUID()}`;
		const providersBefore = FakeWebrtcProvider.instances.length;
		const [first, second] = await Promise.all(
			["aaa.bbb.first", "aaa.bbb.second"].map((jwt) =>
				createRealtimeSession({
					room,
					access: { ...access("password"), jwt },
					signalingServers,
				}),
			),
		);

		expect(FakeWebrtcProvider.instances).toHaveLength(providersBefore + 1);
		expect(second.provider).toBe(first.provider);
		first.dispose();
		expect(latestProvider().destroyCalls).toBe(0);
		second.dispose();
		expect(latestProvider().destroyCalls).toBe(1);
	});

	test("a failed opening does not block the next one", async () => {
		const room = `realtime-retry-test-${crypto.randomUUID()}`;
		const refused = createRealtimeSession({
			room,
			access: { ...access("password"), jwt: "aaa.bbb.ccc" },
		});
		const opened = createRealtimeSession({
			room,
			access: access("password"),
			signalingServers,
		});

		expect(refused).rejects.toThrow("No signaling servers are configured");
		(await opened).dispose();
	});

	const nextDay = (credential: string): IRealtimeAccess => ({
		...access(credential),
		key_id: "2026-09-03",
	});

	test("a consumer with a newer room key takes the room over once its holder lets go", async () => {
		const room = `realtime-rekey-test-${crypto.randomUUID()}`;
		const holder: { session?: { dispose(): void } } = {};
		const stale = mock(() => holder.session?.dispose());
		holder.session = await createRealtimeSession({
			room,
			access: access("first-password"),
			signalingServers,
			onAuthFailure: stale,
		});
		const old = latestProvider();

		const arrival = await createRealtimeSession({
			room,
			access: nextDay("second-password"),
			signalingServers,
		});
		expect(stale).toHaveBeenCalledTimes(1);
		expect(old.destroyCalls).toBe(1);
		expect(arrival.provider).not.toBe(old);
		expect(arrival.provider.peerOpts).toEqual({
			config: { iceServers: nextDay("second-password").ice_servers },
		});

		const rejoined = await createRealtimeSession({
			room,
			access: nextDay("third-password"),
			signalingServers,
		});
		expect(rejoined.provider).toBe(arrival.provider);
		rejoined.dispose();
		arrival.dispose();
	});

	test("a holder that keeps the room refuses a consumer on another key", async () => {
		jest.useFakeTimers();
		const room = `realtime-key-test-${crypto.randomUUID()}`;
		const stale = mock(() => {});
		const first = await createRealtimeSession({
			room,
			access: access("first-password"),
			signalingServers,
			onAuthFailure: stale,
		});

		const refusal = createRealtimeSession({
			room,
			access: nextDay("second-password"),
			signalingServers,
		}).then(
			() => null,
			(error: Error) => error,
		);
		await settled();
		expect(stale).toHaveBeenCalledTimes(1);
		jest.advanceTimersByTime(3_000);
		expect((await refusal)?.message).toContain("older encryption key");
		first.dispose();
	});
});
