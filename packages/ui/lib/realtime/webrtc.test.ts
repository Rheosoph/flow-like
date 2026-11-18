import { afterEach, describe, expect, jest, spyOn, test } from "bun:test";
import type { IRealtimeIceServer } from "./types";
import {
	applyRealtimeIceServers,
	createAuthFailureGate,
	peerOptsForIceServers,
	realtimeIceExpiryMs,
	retireRelayedConnections,
	usesRelay,
} from "./webrtc";

const initialIceServers: IRealtimeIceServer[] = [
	{ urls: ["stun:stun.cloudflare.com:3478"] },
	{
		urls: ["turn:turn.cloudflare.com:3478?transport=udp"],
		username: "first-user",
		credential: "first-password",
	},
];

type Report = Record<string, unknown>;

/** A peer connection's statistics, as the browsers report them. */
function statsOf(reports: Report[]) {
	return {
		getStats: async () => ({
			forEach(visit: (report: Report) => void) {
				for (const report of reports) visit(report);
			},
		}),
	};
}

const pairOver = (candidateType: string): Report[] => [
	{ id: "local", type: "local-candidate", candidateType },
	{ id: "remote", type: "remote-candidate", candidateType: "srflx" },
	{
		id: "pair",
		type: "candidate-pair",
		localCandidateId: "local",
		remoteCandidateId: "remote",
		nominated: true,
		state: "succeeded",
	},
];

const chromeStats = (candidateType: string) =>
	statsOf([
		...pairOver(candidateType),
		{ id: "transport", type: "transport", selectedCandidatePairId: "pair" },
	]);

describe("realtime ICE configuration", () => {
	test("preserves simple-peer defaults when the API omits ICE servers", () => {
		expect(peerOptsForIceServers(undefined)).toEqual({});
	});

	test("keeps an explicit empty ICE override", () => {
		expect(peerOptsForIceServers([])).toEqual({
			config: { iceServers: [] },
		});
	});

	test("places API servers in the simple-peer configuration", () => {
		const source = structuredClone(initialIceServers);
		const options = peerOptsForIceServers(source);
		source[0].urls = ["stun:changed.example.com:3478"];

		expect(options).toEqual({
			config: { iceServers: initialIceServers },
		});
	});

	test("new credentials reach future connections and leave live ones running", () => {
		let destroyCalls = 0;
		const provider = {
			peerOpts: peerOptsForIceServers(initialIceServers),
			room: {
				webrtcConns: new Map([
					["peer-1", { destroy: () => destroyCalls++ }],
					["peer-2", { destroy: () => destroyCalls++ }],
				]),
			},
		};
		const refreshed = [
			initialIceServers[0],
			{
				...initialIceServers[1],
				username: "second-user",
				credential: "second-password",
			},
		];

		expect(applyRealtimeIceServers(provider, refreshed)).toBe(true);
		expect(provider.peerOpts).toEqual({ config: { iceServers: refreshed } });
		expect(destroyCalls).toBe(0);
	});

	test("an unchanged ICE configuration is reported as unchanged", () => {
		const provider = { peerOpts: peerOptsForIceServers(initialIceServers) };

		expect(
			applyRealtimeIceServers(provider, structuredClone(initialIceServers)),
		).toBe(false);
	});

	test("restores simple-peer defaults when a later response omits ICE", () => {
		let destroyCalls = 0;
		const provider = {
			peerOpts: peerOptsForIceServers(initialIceServers),
			room: {
				webrtcConns: new Map([["peer-1", { destroy: () => destroyCalls++ }]]),
			},
		};

		expect(applyRealtimeIceServers(provider, undefined)).toBe(true);
		expect(provider.peerOpts).toEqual({});
		expect(destroyCalls).toBe(0);
	});

	test("reads the ICE expiry of an access in milliseconds", () => {
		const access = { jwt: "", encryption_key: "", key_id: "" };
		expect(
			realtimeIceExpiryMs({ ...access, ice_expires_at: 1_800_000_000 }),
		).toBe(1_800_000_000_000);
		expect(realtimeIceExpiryMs(access)).toBeNull();
		expect(realtimeIceExpiryMs({ ...access, ice_expires_at: 0 })).toBeNull();
	});
});

describe("relayed connections at the end of their credential", () => {
	test("a connection is relayed when its own selected candidate is a relay", async () => {
		expect(await usesRelay(chromeStats("relay"))).toBe(true);
		expect(await usesRelay(chromeStats("host"))).toBe(false);
		expect(await usesRelay(chromeStats("srflx"))).toBe(false);
	});

	test("the other side's relay does not count", async () => {
		const stats = statsOf([
			{ id: "local", type: "local-candidate", candidateType: "host" },
			{ id: "remote", type: "remote-candidate", candidateType: "relay" },
			{
				id: "pair",
				type: "candidate-pair",
				localCandidateId: "local",
				remoteCandidateId: "remote",
			},
			{ id: "transport", type: "transport", selectedCandidatePairId: "pair" },
		]);

		expect(await usesRelay(stats)).toBe(false);
	});

	test("without a transport report the nominated or selected pair decides", async () => {
		expect(await usesRelay(statsOf(pairOver("relay")))).toBe(true);
		expect(await usesRelay(statsOf(pairOver("host")))).toBe(false);
		const firefox = pairOver("relay").map((report) =>
			report.type === "candidate-pair"
				? { ...report, nominated: undefined, state: undefined, selected: true }
				: report,
		);
		expect(await usesRelay(statsOf(firefox))).toBe(true);
		expect(await usesRelay(statsOf([]))).toBe(false);
	});

	test("only relayed connections that are still open are closed", async () => {
		const closed: string[] = [];
		const connection = (name: string, candidateType: string) => ({
			peer: { _pc: chromeStats(candidateType) },
			destroy: () => closed.push(name),
		});
		const relayed = connection("relayed", "relay");
		const direct = connection("direct", "host");
		const gone = connection("gone", "relay");
		const provider = {
			room: {
				webrtcConns: new Map([
					["relayed", relayed],
					["direct", direct],
				]),
			},
		};

		await retireRelayedConnections(provider, [relayed, direct, gone]);
		expect(closed).toEqual(["relayed"]);
	});

	test("a connection whose statistics fail is left alone", async () => {
		const warning = spyOn(console, "warn").mockImplementation(() => undefined);
		const closed: string[] = [];
		const broken = {
			peer: {
				_pc: {
					getStats: async () => {
						throw new Error("closed");
					},
				},
			},
			destroy: () => closed.push("broken"),
		};
		const relayed = {
			peer: { _pc: chromeStats("relay") },
			destroy: () => closed.push("relayed"),
		};
		const provider = {
			room: {
				webrtcConns: new Map([
					["broken", broken],
					["relayed", relayed],
				]),
			},
		};

		await retireRelayedConnections(provider, [broken, relayed]);
		expect(closed).toEqual(["relayed"]);
		expect(warning).toHaveBeenCalledTimes(1);
		warning.mockRestore();
	});
});

describe("credential refresh after a refused signaling socket", () => {
	afterEach(() => {
		jest.useRealTimers();
	});

	/** A refused socket reconnects about every 2.5 s. */
	function refusedFor(ms: number, gate: { signal(): void }): void {
		for (let elapsed = 0; elapsed < ms; elapsed += 2_500) {
			gate.signal();
			jest.advanceTimersByTime(2_500);
		}
	}

	test("the first refusal refreshes at once and repeats wait longer each time", () => {
		jest.useFakeTimers();
		let refreshes = 0;
		const gate = createAuthFailureGate(() => refreshes++);

		gate.signal();
		expect(refreshes).toBe(1);
		for (let attempt = 0; attempt < 500; attempt++) gate.signal();
		expect(refreshes).toBe(1);
		jest.advanceTimersByTime(1_999);
		expect(refreshes).toBe(1);
		jest.advanceTimersByTime(1);
		expect(refreshes).toBe(2);
		gate.signal();
		jest.advanceTimersByTime(3_999);
		expect(refreshes).toBe(2);
		jest.advanceTimersByTime(1);
		expect(refreshes).toBe(3);
	});

	test("a server that keeps refusing is asked about once a minute", () => {
		jest.useFakeTimers();
		let refreshes = 0;
		const gate = createAuthFailureGate(() => refreshes++);

		refusedFor(10 * 60_000, gate);
		expect(refreshes).toBeLessThanOrEqual(16);
		const settled = refreshes;
		refusedFor(10 * 60_000, gate);
		expect(refreshes - settled).toBeLessThanOrEqual(10);
		expect(refreshes - settled).toBeGreaterThanOrEqual(9);
	});

	test("a refusal after a quiet stretch refreshes at once again", () => {
		jest.useFakeTimers();
		let refreshes = 0;
		const gate = createAuthFailureGate(() => refreshes++);

		refusedFor(5 * 60_000, gate);
		jest.advanceTimersByTime(60 * 60_000);
		const before = refreshes;
		gate.signal();
		expect(refreshes).toBe(before + 1);
		gate.signal();
		jest.advanceTimersByTime(2_000);
		expect(refreshes).toBe(before + 2);
	});

	test("a disposed gate keeps a waiting refresh from firing", () => {
		jest.useFakeTimers();
		let refreshes = 0;
		const gate = createAuthFailureGate(() => refreshes++);

		gate.signal();
		gate.signal();
		gate.dispose();
		jest.advanceTimersByTime(60_000);
		expect(refreshes).toBe(1);
	});
});
