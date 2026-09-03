import { describe, expect, spyOn, test } from "bun:test";
import type { IRealtimeIceServer } from "./types";
import { applyRealtimeIceServers, peerOptsForIceServers } from "./webrtc";

const initialIceServers: IRealtimeIceServer[] = [
	{ urls: ["stun:stun.cloudflare.com:3478"] },
	{
		urls: ["turn:turn.cloudflare.com:3478?transport=udp"],
		username: "first-user",
		credential: "first-password",
	},
];

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

	test("updates options before recycling active peers", () => {
		let optionsAtDestroy: unknown;
		let disconnectCalls = 0;
		let connectCalls = 0;
		const provider = {
			peerOpts: peerOptsForIceServers(initialIceServers),
			room: {
				webrtcConns: new Map([
					[
						"peer-1",
						{
							destroy: () => {
								optionsAtDestroy = provider.peerOpts;
							},
						},
					],
					[
						"peer-2",
						{
							destroy: () => {
								optionsAtDestroy = provider.peerOpts;
							},
						},
					],
				]),
			},
			disconnect: () => disconnectCalls++,
			connect: () => connectCalls++,
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
		expect(optionsAtDestroy).toEqual({
			config: { iceServers: refreshed },
		});
		expect(disconnectCalls).toBe(0);
		expect(connectCalls).toBe(0);
	});

	test("continues recycling when one peer fails to close", () => {
		const errorLog = spyOn(console, "error").mockImplementation(
			() => undefined,
		);
		let destroyCalls = 0;
		const provider = {
			peerOpts: peerOptsForIceServers(initialIceServers),
			room: {
				webrtcConns: new Map([
					[
						"peer-1",
						{
							destroy: () => {
								throw new Error("already closed");
							},
						},
					],
					["peer-2", { destroy: () => destroyCalls++ }],
				]),
			},
		};
		const refreshed = structuredClone(initialIceServers);
		refreshed[1].credential = "refreshed-password";

		expect(applyRealtimeIceServers(provider, refreshed)).toBe(true);
		expect(destroyCalls).toBe(1);
		expect(errorLog).toHaveBeenCalledTimes(1);
		errorLog.mockRestore();
	});

	test("does not recycle peers when the ICE configuration is unchanged", () => {
		let destroyCalls = 0;
		const provider = {
			peerOpts: peerOptsForIceServers(initialIceServers),
			room: {
				webrtcConns: new Map([["peer-1", { destroy: () => destroyCalls++ }]]),
			},
		};

		expect(
			applyRealtimeIceServers(provider, structuredClone(initialIceServers)),
		).toBe(false);
		expect(destroyCalls).toBe(0);
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
		expect(destroyCalls).toBe(1);
	});
});
