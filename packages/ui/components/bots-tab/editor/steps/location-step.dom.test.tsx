import { afterEach, describe, expect, test } from "bun:test";
import {
	backendWith,
	deviceRow,
	mountWithBackend,
} from "../../testing/mount-with-backend";

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
});

const NOTICE = "isn't hosting bots right now";

async function mountPicker(deviceId: string, seen: Record<string, unknown>) {
	const { LocationStep } = await import("./location-step");
	const mounted = await mountWithBackend(
		<LocationStep
			location="device"
			deviceId={deviceId}
			removed={[]}
			restored={[]}
			onLocationChange={() => {}}
			onDeviceChange={() => {}}
		/>,
		backendWith([
			deviceRow("dev_ready", { display_name: "Berlin box", ...seen }),
		]),
	);
	unmount = mounted.unmount;
	return mounted.container;
}

describe("device picker host state", () => {
	test("a device that polled a minute ago is ready: no notice", async () => {
		const body = await mountPicker("dev_ready", {
			bot_host_seen_at: Date.now() - 60_000,
		});
		expect(body.textContent).toContain("Berlin box");
		expect(body.textContent).not.toContain(NOTICE);
	});

	test("a device last seen an hour ago gets the notice", async () => {
		const body = await mountPicker("dev_ready", {
			bot_host_seen_at: Date.now() - 3_600_000,
		});
		expect(body.textContent).toContain(NOTICE);
	});

	test("an older hub that omits the field reads as never seen", async () => {
		const body = await mountPicker("dev_ready", {});
		expect(body.textContent).toContain(NOTICE);
	});

	test("no device chosen yet: no notice", async () => {
		const body = await mountPicker("", {});
		expect(body.textContent).not.toContain(NOTICE);
	});
});

describe("device option status line", () => {
	async function option(seen: number | null | undefined) {
		const { BotDeviceOption } = await import("./location-step");
		const mounted = await mountWithBackend(
			<BotDeviceOption
				device={
					{
						device_id: "d",
						name: "box",
						display_name: null,
						bot_host_seen_at: seen,
					} as never
				}
			/>,
			backendWith([]),
		);
		unmount = mounted.unmount;
		return mounted.container.querySelector("[data-host-state]");
	}

	test("ready within three minutes", async () => {
		const line = await option(Date.now() - 2 * 60_000);
		expect(line?.getAttribute("data-host-state")).toBe("ready");
		expect(line?.textContent).toBe("Ready for bots");
	});

	test("offline names the last seen time", async () => {
		const line = await option(Date.now() - 4 * 60_000);
		expect(line?.getAttribute("data-host-state")).toBe("offline");
		expect(line?.textContent).toContain("Bot host offline — last seen");
		expect(line?.textContent).toContain("minutes ago");
	});

	test("never seen asks to update or start the agent", async () => {
		const line = await option(null);
		expect(line?.getAttribute("data-host-state")).toBe("never");
		expect(line?.textContent).toBe("Update or start the agent to host bots");
	});
});
