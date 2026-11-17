import { describe, expect, test } from "bun:test";
import type {
	IBotComputerUpdate,
	IBotScreenAccess,
} from "../../state/backend-state/bot-state";
import {
	RemoteBotComputerHost,
	type RemoteHostDeps,
	type SignalingHandle,
} from "./remote-computer-host";
import type { Discovery } from "./screen-discovery";
import { formatPairingLink, pairMac, pairedMessage } from "./screen-pairing";
import type { ScreenSignal } from "./screen-protocol";
import { type ScreenConnection, ScreenNotPairedError } from "./screen-rtc";
import type { ScreenSealing } from "./screen-sealing";
import type { ScreenStream } from "./screen-stream";
import type { TrustedHost } from "./screen-trust";

const ACCESS = {} as IBotScreenAccess;
const pinnedHost: TrustedHost = {
	key_id: "ik_host",
	public_key: "pk-host",
	label: "Studio Mac",
	host_peer_id: "hostpeer0123456789",
	paired_at: 1,
};

function fakeSealing(pins: TrustedHost[]) {
	const calls = { unpinned: [] as string[], pinned: [] as TrustedHost[] };
	const sealing: ScreenSealing = {
		own: async () => ({ key_id: "ik_viewer", public_key: "pk-viewer" }),
		seal: async () => ({ v: 1, enc: "x", ct: "" }),
		open: async () => ({}),
		trusted: async () => [...pins],
		pin: async (host) => {
			calls.pinned.push(host);
			pins.push(host);
		},
		unpin: async (keyId) => {
			calls.unpinned.push(keyId);
			pins.splice(
				0,
				pins.length,
				...pins.filter((host) => host.key_id !== keyId),
			);
		},
	};
	return { sealing, calls };
}
const timing = {
	asleepRetryMs: 5,
	unreachableRetryMs: 5,
	idleCloseMs: 20,
	reconnectMs: 5,
};
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const until = async (check: () => boolean) => {
	for (let attempt = 0; attempt < 200 && !check(); attempt += 1) await wait(2);
	expect(check()).toBe(true);
};

class FakeStream implements ScreenStream {
	written: Uint8Array[] = [];
	resets = 0;
	private waiting: ((chunk: Uint8Array | null) => void) | null = null;
	read() {
		return new Promise<Uint8Array | null>((resolve) => {
			this.waiting = resolve;
		});
	}
	async write(bytes: Uint8Array) {
		this.written.push(bytes);
	}
	reset() {
		this.resets += 1;
		this.waiting?.(null);
	}
}

function harness(
	discoveries: Discovery[],
	options: {
		signalingFails?: boolean;
		excludeHost?: string | null;
		pins?: TrustedHost[];
		connectFails?: Error;
	} = {},
) {
	const { sealing, calls: sealCalls } = fakeSealing(
		options.pins ?? [pinnedHost],
	);
	const calls = {
		hostKeys: [] as string[],
		signaling: 0,
		closed: 0,
		discover: [] as (string | null | undefined)[],
		connect: [] as string[],
		trust: undefined as
			| ((keyId: string | undefined, from: string) => boolean)
			| undefined,
	};
	const streams: FakeStream[] = [];
	const ends: ((reason: string) => void)[] = [];
	let index = 0;
	const deps: RemoteHostDeps = {
		signaling: () => {
			calls.signaling += 1;
			return {
				viewerId: "viewer0123456789",
				ready: () =>
					options.signalingFails
						? Promise.reject(new Error("no signaling"))
						: Promise.resolve(),
				send: async () => {},
				subscribe: () => () => {},
				close: () => {
					calls.closed += 1;
				},
			} as SignalingHandle;
		},
		discover: async (_bus, _botId, discoverOptions) => {
			calls.discover.push(discoverOptions?.excludeHost);
			calls.trust = discoverOptions?.trusted;
			const found = discoveries[Math.min(index, discoveries.length - 1)];
			index += 1;
			return found;
		},
		connect: async (_bus, connectOptions) => {
			if (options.connectFails) throw options.connectFails;
			calls.connect.push(connectOptions.hostPeerId);
			calls.hostKeys.push(connectOptions.hostKey.key_id);
			const stream = new FakeStream();
			streams.push(stream);
			let finish: (reason: string) => void = () => {};
			const closed = new Promise<string>((resolve) => {
				finish = resolve;
				ends.push(resolve);
			});
			return {
				stream,
				closed,
				close: () => finish("closed"),
			} as ScreenConnection;
		},
	};
	const host = new RemoteBotComputerHost({
		access: async () => ACCESS,
		sealing,
		excludeHost: async () => options.excludeHost ?? null,
		deps,
		timing,
	});
	const updates: IBotComputerUpdate[] = [];
	const watch = (botId = "bot-1") =>
		host.watch(botId, null, (update) => updates.push(update));
	return { host, calls, sealCalls, streams, ends, updates, watch };
}

const nobody: Discovery = {
	hosts: new Map(),
	keyIds: new Map(),
	active: null,
	strangers: 0,
	strangerActive: false,
};
const asleep: Discovery = {
	hosts: new Map([["h", false]]),
	keyIds: new Map([["h", "ik_host"]]),
	active: null,
	strangers: 0,
	strangerActive: false,
};
const strangers: Discovery = { ...nobody, strangers: 1 };
const busyStranger: Discovery = { ...strangers, strangerActive: true };
const live: Discovery = {
	hosts: new Map([["hostpeer0123456789", true]]),
	keyIds: new Map([["hostpeer0123456789", "ik_host"]]),
	active: "hostpeer0123456789",
	strangers: 0,
	strangerActive: false,
};
const statuses = (updates: IBotComputerUpdate[]) =>
	updates.map((update) => update.status);

describe("RemoteBotComputerHost", () => {
	test("reports unreachable when nobody answers", async () => {
		const { host, updates, watch } = harness([nobody]);
		const off = watch();
		await until(() => statuses(updates).includes("unreachable"));
		await expect(host.view("bot-1")).rejects.toThrow();
		off();
	});

	test("an unreachable signaling server carries its reason", async () => {
		const { updates, watch } = harness([nobody], { signalingFails: true });
		const off = watch();
		await until(() =>
			updates.some((update) => update.error === "no signaling"),
		);
		off();
	});

	test("trusts a pinned key only from the peer it was paired with", async () => {
		const { calls, watch } = harness([asleep]);
		const off = watch();
		await until(() => calls.trust !== undefined);
		expect(calls.trust?.("ik_host", "hostpeer0123456789")).toBe(true);
		expect(calls.trust?.("ik_host", "impostor01234567")).toBe(false);
		expect(calls.trust?.(undefined, "hostpeer0123456789")).toBe(false);
		off();
	});

	test("reports asleep when hosts answer without a picture", async () => {
		const { host, updates, watch } = harness([asleep]);
		const off = watch();
		await until(() => statuses(updates).includes("asleep"));
		expect((await host.view("bot-1")).state).toBe("asleep");
		off();
	});

	test("goes live, sends controls to the stream and returns to discovery when it closes", async () => {
		const { host, calls, streams, ends, updates, watch } = harness([live], {
			excludeHost: "own",
		});
		const off = watch();
		await until(() => statuses(updates).includes("live"));
		expect(calls.connect).toEqual(["hostpeer0123456789"]);
		expect(calls.hostKeys).toEqual(["ik_host"]);
		expect(calls.discover[0]).toBe("own");
		await host.takeOver("bot-1");
		await host
			.input("bot-1", { type: "key", key: "a" } as never)
			.catch(() => {});
		expect(streams[0].written.length).toBeGreaterThan(0);
		const before = calls.discover.length;
		ends[0]("channel closed");
		await until(() => calls.discover.length > before);
		await until(() => calls.connect.length === 2);
		off();
	});

	test("watchers of one bot share one session", async () => {
		const { calls, updates, watch } = harness([live]);
		const first = watch();
		const second = watch();
		await until(
			() => statuses(updates).filter((status) => status === "live").length >= 2,
		);
		expect(calls.signaling).toBe(1);
		expect(calls.connect.length).toBe(1);
		first();
		second();
	});

	test("closes the session shortly after the last watcher leaves", async () => {
		const { calls, streams, watch } = harness([live]);
		const off = watch();
		await until(() => streams.length === 1);
		off();
		await until(() => calls.closed === 1);
		expect(streams[0].resets).toBeGreaterThan(0);
		const again = watch();
		await until(() => calls.signaling === 2);
		again();
	});

	test("a device bot is refused; controls without a session fail", async () => {
		const { host } = harness([nobody]);
		const seen: IBotComputerUpdate[] = [];
		host.watch("bot-1", "device-1", (update) => seen.push(update));
		expect(seen[0].status).toBe("unreachable");
		await expect(host.view("bot-1", "device-1")).rejects.toThrow();
		await expect(host.stop("nope")).rejects.toThrow();
	});
});

describe("pairing", () => {
	test("hosts that answered but are not trusted report unpaired", async () => {
		const { updates, watch } = harness([strangers], { pins: [] });
		const off = watch();
		await until(() => statuses(updates).includes("unpaired"));
		expect(updates.find((update) => update.status === "unpaired")).toEqual({
			status: "unpaired",
			host_label: undefined,
		});
		off();
	});

	test("an idle stranger is not a reason to pair once this device paired another host", async () => {
		const { updates, watch } = harness([strangers]);
		const off = watch();
		await until(() => statuses(updates).includes("unreachable"));
		expect(statuses(updates)).not.toContain("unpaired");
		off();
	});

	test("an app that hosts screens itself is not asked to pair for an idle stranger", async () => {
		const { updates, watch } = harness([strangers], {
			pins: [],
			excludeHost: "ownhost0123456789",
		});
		const off = watch();
		await until(() => statuses(updates).includes("unreachable"));
		expect(statuses(updates)).not.toContain("unpaired");
		off();
	});

	test("a stranger with the live picture asks to pair even with other pins", async () => {
		const { updates, watch } = harness([busyStranger]);
		const off = watch();
		await until(() => statuses(updates).includes("unpaired"));
		off();
	});

	test("a not_paired refusal drops the pin and reports unpaired with its label", async () => {
		const { updates, sealCalls, watch } = harness([live], {
			connectFails: new ScreenNotPairedError(),
		});
		const off = watch();
		await until(() => statuses(updates).includes("unpaired"));
		expect(sealCalls.unpinned).toEqual(["ik_host"]);
		expect(updates.at(-1)?.host_label).toBe("Studio Mac");
		off();
	});

	const secret = new Uint8Array(32).fill(0x33);
	const link = formatPairingLink({
		hostPeerId: "hostpeer0123456789",
		keyId: "ik_ae216c2ef5247a37",
		publicKey: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
		secret,
		label: "Studio Mac",
	});

	function pairingHost(pins: TrustedHost[], tamper = false) {
		const { sealing, calls } = fakeSealing(pins);
		const sent: ScreenSignal[] = [];
		const host = new RemoteBotComputerHost({
			access: async () => ACCESS,
			sealing,
			deviceLabel: () => "Test phone",
			deps: {
				signaling: () => {
					const listeners = new Set<(signal: ScreenSignal) => void>();
					return {
						viewerId: "viewer0123456789",
						ready: async () => {},
						close: () => {},
						subscribe: (listener: (signal: ScreenSignal) => void) => {
							listeners.add(listener);
							return () => listeners.delete(listener);
						},
						send: async (message: { type: string; session?: string }) => {
							sent.push(message as never);
							const mac = await pairMac(
								secret,
								pairedMessage(message.session ?? ""),
							);
							for (const listener of listeners)
								listener({
									type: "paired",
									from: "hostpeer0123456789",
									to: "viewer0123456789",
									session: message.session ?? "",
									mac: tamper ? `${mac.slice(1)}A` : mac,
									at: Date.now(),
								});
						},
					} as unknown as SignalingHandle;
				},
			},
			timing,
		});
		return { host, calls, sent };
	}

	test("pairScreen sends the pair message, verifies the proof and pins the host", async () => {
		const pins: TrustedHost[] = [];
		const { host, calls, sent } = pairingHost(pins);
		await host.pairScreen("bot-1", link);
		expect(sent[0]).toMatchObject({
			type: "pair",
			to: "hostpeer0123456789",
			key_id: "ik_viewer",
			public_key: "pk-viewer",
			label: "Test phone",
		});
		expect(calls.pinned.map((entry) => entry.key_id)).toEqual([
			"ik_ae216c2ef5247a37",
		]);
		expect(calls.pinned[0].label).toBe("Studio Mac");
	});

	test("a wrong proof pins nothing and surfaces an error", async () => {
		const { host, calls } = pairingHost([], true);
		await expect(host.pairScreen("bot-1", link)).rejects.toThrow(
			/pairing proof did not match/,
		);
		expect(calls.pinned).toEqual([]);
	});

	test("a link of another kind is refused before anything is sent", async () => {
		const { host, sent } = pairingHost([]);
		await expect(
			host.pairScreen("bot-1", "https://example.com/pair"),
		).rejects.toThrow(/pairing link/);
		expect(sent).toEqual([]);
	});
});
