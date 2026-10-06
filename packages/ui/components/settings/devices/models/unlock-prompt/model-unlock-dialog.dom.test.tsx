import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { LocalDeviceVault } from "../../../../../lib/device-management/storage";
import {
	allByRole,
	byRole,
	byText,
	click,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../../testing/dom-harness";
import type {
	DeviceUnlockRequest,
	ModelUnlockPrompt,
	VaultLookup,
} from "./model-unlock";
import type { ModelUnlockBridge } from "./use-model-unlock";

const dom = installDom();
const { act } = await import("react");
const { ModelUnlockPrompts } = await import("./model-unlock-dialog");

afterEach(dom.cleanup);
afterAll(dom.restore);

const SCOPE = {
	issuer: "https://issuer.flow-like.test",
	account: "user-1",
	apiOrigin: "https://api.flow-like.test",
	profileId: "default",
};

function encode(value: unknown) {
	return btoa(JSON.stringify(value))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/u, "");
}

function vault(overrides: Partial<LocalDeviceVault> = {}): LocalDeviceVault {
	return {
		deviceId: "device-1",
		controllerPublic: {
			device_id: "device-1",
		} as LocalDeviceVault["controllerPublic"],
		controllerVault: Uint8Array.from({ length: 64 }, (_, index) => index),
		manifestJws: `${encode({ alg: "EdDSA" })}.${encode({ device_id: "device-1", name: "GPU box" })}.signature`,
		grantId: "owner",
		...overrides,
	};
}

function prompt(overrides: Partial<ModelUnlockPrompt> = {}): ModelUnlockPrompt {
	return {
		id: "prompt-1",
		deviceId: "device-1",
		modelName: "Qwen3 8B",
		runId: "run-1",
		runName: "Daily summary",
		expiresAt: Date.now() + 120_000,
		...overrides,
	};
}

class FakeBridge implements ModelUnlockBridge {
	early: ModelUnlockPrompt[] = [];
	lookups = new Map<string, VaultLookup>();
	unlocks: DeviceUnlockRequest[] = [];
	declines: string[] = [];
	refusal?: string;
	private requested?: (prompt: ModelUnlockPrompt) => void;
	private closed?: (promptId: string) => void;

	async pending() {
		return this.early;
	}

	async listen(
		onRequested: (prompt: ModelUnlockPrompt) => void,
		onClosed: (promptId: string) => void,
	) {
		this.requested = onRequested;
		this.closed = onClosed;
		return () => {
			this.requested = undefined;
			this.closed = undefined;
		};
	}

	async vault(deviceId: string): Promise<VaultLookup> {
		return (
			this.lookups.get(deviceId) ?? {
				kind: "ready",
				scope: SCOPE,
				vault: vault(),
			}
		);
	}

	async unlock(request: DeviceUnlockRequest) {
		this.unlocks.push(request);
		if (this.refusal) throw this.refusal;
	}

	async decline(promptId: string) {
		this.declines.push(promptId);
	}

	async show(next: ModelUnlockPrompt) {
		await act(async () => this.requested?.(next));
		await settle();
	}

	async end(promptId: string) {
		await act(async () => this.closed?.(promptId));
		await settle();
	}
}

async function mount(bridge: FakeBridge) {
	await dom.render(<ModelUnlockPrompts bridge={bridge} />);
	await settle();
	await settle();
}

const dialog = () => queryByRole("dialog");
const passwordField = () => byRole("textbox", "Device password");
const unlockButton = () => byRole("button", "Unlock");

async function unlockWith(password: string) {
	await typeInto(passwordField(), password);
	await click(unlockButton());
	await settle();
}

describe("ModelUnlockPrompts", () => {
	test("names the run, the model and the device and keeps the device unlocked by default", async () => {
		const bridge = new FakeBridge();
		bridge.early = [prompt()];
		await mount(bridge);

		expect(byRole("heading", "Unlock GPU box?")).toBeTruthy();
		expect(
			byText("“Daily summary” wants to use Qwen3 8B, which runs on GPU box."),
		).toBeTruthy();
		const keep = byRole("switch", /Keep unlocked until I quit/);
		expect(keep.getAttribute("aria-checked")).toBe("true");
		expect(unlockButton().getAttribute("aria-disabled")).toBe("true");
	});

	test("hands over only the password and the encrypted vault, then closes", async () => {
		const bridge = new FakeBridge();
		await mount(bridge);
		expect(dialog()).toBeNull();

		await bridge.show(prompt({ runName: null }));
		expect(
			byText("A run wants to use Qwen3 8B, which runs on GPU box."),
		).toBeTruthy();
		await unlockWith("correct horse");

		expect(bridge.unlocks).toEqual([
			{
				deviceId: "device-1",
				password: "correct horse",
				controllerVault: Array.from(vault().controllerVault),
				manifestJws: vault().manifestJws,
				grantId: "owner",
				apiOrigin: SCOPE.apiOrigin,
				account: SCOPE.account,
				keepUnlocked: true,
			},
		]);
		expect(dialog()).toBeNull();
	});

	test("a refused password keeps the prompt open with the reason", async () => {
		const bridge = new FakeBridge();
		bridge.refusal = "wrong_password: the controller vault did not open";
		bridge.early = [prompt()];
		await mount(bridge);

		await unlockWith("wrong");
		expect(
			byText("That password didn't open the keys. Try again."),
		).toBeTruthy();
		expect(dialog()).not.toBeNull();

		bridge.refusal = undefined;
		await click(byRole("switch", /Keep unlocked until I quit/));
		await unlockWith("correct horse");
		expect(bridge.unlocks.at(-1)?.keepUnlocked).toBe(false);
		expect(dialog()).toBeNull();
	});

	test("Not now declines and the next device asks in turn", async () => {
		const bridge = new FakeBridge();
		bridge.early = [
			prompt(),
			prompt({ id: "prompt-2", deviceId: "device-2", deviceName: "Mac mini" }),
		];
		await mount(bridge);

		await click(byRole("button", "Not now"));
		await settle();
		expect(bridge.declines).toEqual(["prompt-1"]);
		expect(byRole("heading", "Unlock Mac mini?")).toBeTruthy();
		expect(allByRole("dialog")).toHaveLength(1);
	});

	test("closes when the desktop ends the prompt", async () => {
		const bridge = new FakeBridge();
		await mount(bridge);
		await bridge.show(prompt());
		expect(dialog()).not.toBeNull();

		await bridge.end("prompt-1");
		expect(dialog()).toBeNull();
		expect(bridge.declines).toEqual([]);
	});

	test("without keys on this computer only Not now is offered", async () => {
		const bridge = new FakeBridge();
		bridge.lookups.set("device-1", { kind: "blocked", block: "no_vault" });
		bridge.early = [prompt()];
		await mount(bridge);

		expect(
			byText(
				"This computer holds no keys for device-1. Add it in Devices first; until then runs go on without it.",
			),
		).toBeTruthy();
		expect(queryByRole("button", "Unlock")).toBeNull();
		expect(queryByRole("textbox", "Device password")).toBeNull();
		await click(byRole("button", "Not now"));
		expect(bridge.declines).toEqual(["prompt-1"]);
	});
});
