import { afterEach, describe, expect, test } from "bun:test";
import {
	backendWith,
	botOf,
	deviceRow,
	mountWithBackend,
} from "./testing/mount-with-backend";

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
});

async function mountWhere(bot: ReturnType<typeof botOf>, seen?: number) {
	const { WhereItRuns } = await import("./bot-details");
	const mounted = await mountWithBackend(
		<WhereItRuns bot={bot} />,
		backendWith([
			deviceRow("dev_a", {
				display_name: "Berlin box",
				...(seen === undefined ? {} : { bot_host_seen_at: seen }),
			}),
		]),
	);
	unmount = mounted.unmount;
	return mounted.container.textContent ?? "";
}

describe("Where it runs", () => {
	test("a Docker bot identifies its container host", async () => {
		const text = await mountWhere(
			botOf("b", { device_id: "dev_a", execution_mode: "docker" }),
		);
		expect(text).toContain("Docker on Berlin box");
	});
	test("a device bot names its device and the host state", async () => {
		const text = await mountWhere(
			botOf("b", { device_id: "dev_a" }),
			Date.now() - 30_000,
		);
		expect(text).toContain("On Berlin box");
		expect(text).toContain("Ready for bots");
	});

	test("a device whose agent never polled asks for an update", async () => {
		const text = await mountWhere(botOf("b", { device_id: "dev_a" }));
		expect(text).toContain("On Berlin box");
		expect(text).toContain("Update or start the agent to host bots");
	});

	test("an unknown device falls back to the generic label", async () => {
		const text = await mountWhere(botOf("b", { device_id: "dev_gone" }));
		expect(text).toContain("Device");
		expect(text).not.toContain("On Berlin box");
	});
});
