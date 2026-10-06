import { afterAll, afterEach, expect, test } from "bun:test";
import { act } from "react";
import type { DeviceServiceStream } from "../../../../lib/device-management/tunnel";
import type { DevicePortBridge } from "../../../../lib/device-management/tunnel-forward";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { ServiceTunnels } = await import("./tunnels-block");
const { until } = await import("./config-test-kit");
afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	localStorage.clear();
});
afterAll(dom.restore);

class DesktopBridge implements DevicePortBridge {
	lease = { id: "forward", token: "owner", port: 41234 };
	listens: number[] = [];
	closes = 0;
	private rejectAccept?: (error: Error) => void;
	async listen(port: number) {
		this.listens.push(port);
		return this.lease;
	}
	accept(): Promise<string | null> {
		return new Promise((_, reject) => {
			this.rejectAccept = reject;
		});
	}
	async read() {
		return new Uint8Array();
	}
	async write() {}
	async finish() {}
	async disconnect() {}
	async touch() {}
	async close() {
		this.closes++;
		this.rejectAccept?.(new Error("Listener closed."));
	}
}

async function mountForward() {
	const fake = await createFakeWorkspace(undefined, { platform: "desktop" });
	const bridge = new DesktopBridge();
	let probes = 0;
	let resets = 0;
	const modes: (string | undefined)[] = [];
	fake.workspace.live.openService = async (
		_device,
		_placement,
		_service,
		options,
	) => {
		probes++;
		modes.push(options?.mode);
		return {
			reset() {
				resets++;
			},
		} as unknown as DeviceServiceStream;
	};
	const view = await mountDevices(
		<ServiceTunnels
			deviceId={SAMPLE_IDS.studio}
			serviceId="field-notes"
			portBridge={bridge}
		/>,
		{ fake },
	);
	await until(
		() => queryByRole("button", "Open local port", view.container) !== null,
	);
	return {
		view,
		fake,
		bridge,
		probes: () => probes,
		resets: () => resets,
		modes,
	};
}

test("desktop publishes the loopback address only after a service probe and Stop closes it", async () => {
	const { view, bridge, probes, resets, modes } = await mountForward();
	expect(bridge.listens).toEqual([]);
	await click(byRole("button", "Open local port", view.container));
	await until(
		() => view.container.querySelector("[data-tunnel-local-port]") !== null,
	);
	expect(probes()).toBe(1);
	expect(resets()).toBe(1);
	expect(modes).toEqual(["tcp"]);
	expect(bridge.listens).toEqual([0]);
	expect(
		view.container.querySelector("[data-tunnel-local-port]")?.textContent,
	).toBe("127.0.0.1:41234");
	await click(byRole("button", "Close local port", view.container));
	await until(
		() => queryByRole("button", "Open local port", view.container) !== null,
	);
	expect(bridge.closes).toBe(1);
	expect(view.container.querySelector("[data-tunnel-local-port]")).toBeNull();
});

test("locking the device closes the desktop port and removes its address", async () => {
	const { view, fake, bridge } = await mountForward();
	await click(byRole("button", "Open local port", view.container));
	await until(
		() => view.container.querySelector("[data-tunnel-local-port]") !== null,
	);
	await act(async () => {
		fake.workspace.keys.lock(SAMPLE_IDS.studio);
		await fake.settle();
	});
	await until(() => bridge.closes === 1);
	expect(view.container.querySelector("[data-tunnel-local-port]")).toBeNull();
});

test("leaving the desktop service page closes the listener", async () => {
	const { view, bridge } = await mountForward();
	await click(byRole("button", "Open local port", view.container));
	await until(
		() => view.container.querySelector("[data-tunnel-local-port]") !== null,
	);
	await view.unmount();
	expect(bridge.closes).toBe(1);
});

test("denied service access never opens a native listener", async () => {
	const { view, fake, bridge } = await mountForward();
	fake.workspace.live.openService = async () => {
		throw new Error("Service access denied.");
	};
	await click(byRole("button", "Open local port", view.container));
	await until(
		() =>
			view.container.textContent?.includes("Service access denied.") ?? false,
	);
	expect(bridge.listens).toEqual([]);
});
