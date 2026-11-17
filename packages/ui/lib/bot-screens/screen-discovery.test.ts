import { describe, expect, test } from "bun:test";
import { type SignalBus, discoverHosts } from "./screen-discovery";
import type { ScreenSignal } from "./screen-protocol";

function bus(answers: Partial<Extract<ScreenSignal, { type: "here" }>>[]) {
	const listeners = new Set<(signal: ScreenSignal) => void>();
	const fake: SignalBus = {
		viewerId: "viewer0123456789",
		subscribe: (listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		send: async () => {
			for (const answer of answers)
				for (const listener of listeners)
					listener({
						type: "here",
						to: "viewer0123456789",
						bot_id: "bot-1",
						active: false,
						at: 0,
						from: "hostpeer0123456789",
						...answer,
					});
		},
	};
	return fake;
}

describe("discoverHosts", () => {
	test("offers only to hosts with a pinned key and counts the rest as strangers", async () => {
		const found = await discoverHosts(
			bus([
				{ from: "stranger01234567", key_id: "ik_other", active: true },
				{ from: "nokey0123456789", active: true },
				{ from: "trusted012345678", key_id: "ik_host", active: true },
			]),
			"bot-1",
			{ trusted: (keyId) => keyId === "ik_host", windowMs: 50 },
		);
		expect(found.active).toBe("trusted012345678");
		expect(found.keyIds.get("trusted012345678")).toBe("ik_host");
		expect(found.strangers).toBe(2);
	});

	test("names the stranger that has the picture", async () => {
		const found = await discoverHosts(
			bus([
				{
					from: "idle012345678901",
					key_id: "ik_a",
					active: false,
					label: "Old iMac",
				},
				{
					from: "busy012345678901",
					key_id: "ik_b",
					active: true,
					label: "Studio Mac",
				},
			]),
			"bot-1",
			{ trusted: () => false, windowMs: 50 },
		);
		expect(found.strangerActive).toBe(true);
		expect(found.strangerLabel).toBe("Studio Mac");
	});

	test("with nobody trusted there is no active host", async () => {
		const found = await discoverHosts(
			bus([{ key_id: "ik_other", active: true }]),
			"bot-1",
			{ trusted: () => false, windowMs: 20 },
		);
		expect(found.active).toBeNull();
		expect(found.hosts.size).toBe(0);
		expect(found.strangers).toBe(1);
	});
});
