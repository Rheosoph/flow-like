import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClientProvider } from "@tanstack/react-query";
import type { IBotComputerView } from "../../state/backend-state/bot-state";
import {
	byRole,
	click,
	installDom,
	settle,
} from "../settings/devices/testing/dom-harness";

const dom = installDom();
const harness = await import("../settings/devices/workspace/test-harness");
const { useOverlayStore } = await import(
	"../settings/devices/workspace/overlay-store"
);
const registry = await import("../../lib/device-management/workspace/registry");
const { DeviceComputerPane } = await import("./device-computer-pane");
const restoreBackend = harness.installTestBackend();

afterEach(async () => {
	await dom.cleanup();
	await registry.disposeAllDeviceWorkspaces();
	useOverlayStore.getState().close();
});
afterAll(() => {
	restoreBackend();
	dom.restore();
});

const active: IBotComputerView = {
	state: "active",
	kind: "browser",
	caption: "https://shop.example/",
	image: "data:image/jpeg;base64,AAAA",
	age_ms: 100,
	can_control: true,
};

type Control = { type: string };

class FakeViewer {
	sent: Control[] = [];
	closed = false;
	view: IBotComputerView = active;
	private end!: (reason: string) => void;
	readonly ended = new Promise<string>((resolve) => {
		this.end = resolve;
	});
	subscribe() {
		return () => {};
	}
	async send(control: Control) {
		this.sent.push(control);
	}
	close() {
		this.closed = true;
	}
	finish() {
		this.end("ended");
	}
}

function fakeStream() {
	return {
		closed: new Promise<void>(() => {}),
		read: async () => null,
		write: async () => {},
		reset: () => {},
	};
}

function mount(
	test: ReturnType<typeof harness.createTestWorkspace>,
	options: {
		signedOut?: boolean;
		viewers?: FakeViewer[];
		opened?: string[];
	} = {},
) {
	const live = test.workspace.live as unknown as Record<string, unknown>;
	live.openBotPlayground = async (deviceId: string, botId: string) => {
		options.opened?.push(`${deviceId}/${botId}`);
		return fakeStream();
	};
	const auth = options.signedOut
		? { ...harness.TEST_AUTH, signedIn: false, account: "" }
		: harness.TEST_AUTH;
	return dom.render(
		<QueryClientProvider client={test.queryClient}>
			<DeviceComputerPane
				bot={{ id: "bot-1" }}
				name="Scout"
				deviceId="dev-1"
				overrides={{
					auth,
					profile: harness.TEST_PROFILE,
					now: test.deps.now,
					crypto: test.deps.crypto,
					platform: "desktop",
					workspace: () => test.workspace,
				}}
				makeViewer={() => {
					const viewer = new FakeViewer();
					options.viewers?.push(viewer);
					return viewer as never;
				}}
			/>
		</QueryClientProvider>,
	);
}

describe("DeviceComputerPane", () => {
	test("a locked device offers the unlock flow", async () => {
		const test = harness.createTestWorkspace();
		const view = await mount(test);
		await test.idle();
		expect(view.container.textContent).toContain(
			"Unlock this device to watch Scout",
		);
		await click(byRole("button", "Unlock…", view.container));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: "dev-1",
		});
	});

	test("signed out says so and offers sign in", async () => {
		const test = harness.createTestWorkspace();
		const view = await mount(test, { signedOut: true });
		expect(view.container.textContent).toContain("Sign in to watch Scout");
	});

	test("an unlocked device opens the stream and drives it from the stage", async () => {
		const test = harness.createTestWorkspace();
		await test.unlock("dev-1");
		const viewers: FakeViewer[] = [];
		const opened: string[] = [];
		const view = await mount(test, { viewers, opened });
		await test.idle();
		expect(opened).toEqual(["dev-1/bot-1"]);
		expect(viewers).toHaveLength(1);
		expect(view.container.querySelector("img")).not.toBeNull();
		await click(byRole("button", "Take over", view.container));
		await click(byRole("button", "Stop", view.container));
		expect(viewers[0].sent).toEqual([{ type: "take_over" }, { type: "stop" }]);
		await dom.cleanup();
		expect(viewers[0].closed).toBe(true);
	});

	test("a stream that ends is reopened after a pause", async () => {
		const test = harness.createTestWorkspace();
		await test.unlock("dev-1");
		const viewers: FakeViewer[] = [];
		const opened: string[] = [];
		await mount(test, { viewers, opened });
		await test.idle();
		viewers[0].finish();
		for (let round = 0; round < 30 && viewers.length < 2; round++) {
			await new Promise((resolve) => setTimeout(resolve, 100));
			await settle();
		}
		expect(viewers).toHaveLength(2);
		expect(viewers[0].closed).toBe(true);
		expect(opened).toHaveLength(2);
	});
});
