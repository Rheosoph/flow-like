import { afterEach, describe, expect, test } from "bun:test";
import {
	backendWith,
	botOf,
	mountWithBackend,
} from "../../../bots-tab/testing/mount-with-backend";

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
});

const PAGE = { deviceId: "dev_a", name: "Berlin box" };

async function mountSection(
	bots: ReturnType<typeof botOf>[],
	presence: Record<string, { state: string }> = {},
) {
	const { BotsOnDevice } = await import("./bots-on-device");
	const mounted = await mountWithBackend(
		<BotsOnDevice page={PAGE as never} />,
		backendWith([], {
			listBots: async () => bots,
			getPresence: async () => presence,
		}),
		{ route: true },
	);
	unmount = mounted.unmount;
	return mounted.container;
}

describe("Bots on this device", () => {
	test("lists only the bots bound to this device, with presence and an open link", async () => {
		const body = await mountSection(
			[
				botOf("bot_here", { name: "Scout", device_id: "dev_a" }),
				botOf("bot_there", { name: "Elsewhere", device_id: "dev_b" }),
				botOf("bot_cloud", { name: "Cloudy", location: "server" }),
				botOf("bot_local", { name: "Mine", device_id: "dev_a", local: true }),
			],
			{ bot_here: { state: "online" } },
		);
		const rows = [...body.querySelectorAll("[data-bot-on-device]")];
		expect(rows.map((row) => row.getAttribute("data-bot-on-device"))).toEqual([
			"bot_here",
		]);
		expect(body.textContent).toContain("Bots on this device");
		expect(body.textContent).toContain("Create a bot on this device");
		expect(rows[0]?.textContent).toContain("Scout");
		expect(rows[0]?.textContent).toContain("Online");
		expect(rows[0]?.querySelector("a")?.getAttribute("href")).toBe(
			"/bots?bot=bot_here",
		);
	});

	test("a bot nobody has heard from reads as not seen yet", async () => {
		const body = await mountSection([
			botOf("bot_here", { name: "Scout", device_id: "dev_a" }),
		]);
		expect(
			body
				.querySelector("[data-bot-presence]")
				?.getAttribute("data-bot-presence"),
		).toBe("unknown");
	});

	test("without bots it says so", async () => {
		const body = await mountSection([
			botOf("bot_there", { name: "Elsewhere", device_id: "dev_b" }),
		]);
		expect(body.querySelector("[data-bot-on-device]")).toBeNull();
		expect(body.textContent).toContain("No bots live on Berlin box yet");
		expect(body.textContent).toContain("Create a bot on this device");
	});
});
