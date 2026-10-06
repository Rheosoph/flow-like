import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { NativeKeyWatcher } from "../../../../../lib/device-management/native-client";
import { type IBit, IBitTypes } from "../../../../../lib/schema/bit/bit";
import type { ISettingsProfile } from "../../../../../types";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../../testing/dom-harness";
import type { FakeWorkspace } from "../../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { SAMPLE_IDS } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { MODEL_HOST_FEATURES, PRE_MODEL_FEATURES } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { ApiResponseError } = await import("../../../../../lib/api-error");
const { UseModelButton } = await import("./use-model-sheet");
const { DeviceModelBadge } = await import("./device-model-badge");
const { createHeldDevices } = await import("./device-key-state");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge } = SAMPLE_IDS;
const DEVICE = "edge-berlin-01";

const QWEN = {
	id: "qwen3-8b-q4",
	display_name: "Qwen3-8B Q4_K_M",
	kind: "chat" as const,
	settings: { ctx_per_slot: 16_384 },
};
const NOMIC = {
	id: "nomic-embed",
	display_name: "nomic-embed-text v1.5",
	kind: "embedding" as const,
	settings: {},
};

/** R3: no wire value or provider name reaches the sheet. */
const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|chat_completions|custom:/;

interface Options {
	features?: typeof MODEL_HOST_FEATURES;
	refuse?: unknown;
	platform?: "desktop" | "web";
}

function fakeBits(refuse: unknown) {
	const saved: IBit[] = [];
	const added: string[] = [];
	return {
		saved,
		added,
		bitState: {
			async upsertCustomBit(bit: IBit) {
				if (refuse) throw refuse;
				saved.push(bit);
				return bit;
			},
			async addBit(bit: IBit, profile: ISettingsProfile) {
				added.push(`${bit.id} → ${profile.hub_profile.id}`);
			},
			async listCustomBits() {
				return saved;
			},
			async getProfileBits() {
				return [];
			},
		},
	};
}

function userState(fake: FakeWorkspace) {
	return {
		async getProfile() {
			return fake.profile;
		},
		async getInfo() {
			return { id: fake.hub.me, dev_mode: false };
		},
		async updateUser() {},
		async getSettingsProfile() {
			return { hub_profile: { id: "profile-1", bits: [] } };
		},
	};
}

async function mount(model: typeof QWEN | typeof NOMIC, options: Options = {}) {
	const fake = await createFakeWorkspace(undefined, {
		agentFeatures: { [edge]: options.features ?? MODEL_HOST_FEATURES },
		...(options.platform ? { platform: options.platform } : {}),
	});
	const bits = fakeBits(options.refuse);
	const mounted = await mountDevices(
		<UseModelButton deviceId={edge} model={model} />,
		{
			fake,
			backend: {
				bitState: bits.bitState,
				userState: userState(fake),
			} as never,
		},
	);
	return { mounted, fake, bits };
}

type Mounted = Awaited<ReturnType<typeof mount>>;

function sheet() {
	return inPortal("dialog");
}

function text(root: ParentNode) {
	return (root.textContent ?? "").replace(/\s+/g, " ");
}

function field(label: string) {
	return byRole("textbox", label, sheet()) as HTMLInputElement;
}

function keepBox() {
	return byRole("checkbox", /^Keep unlocked for model access/, sheet());
}

async function openSheet({ mounted }: Mounted) {
	await click(byRole("button", "Use from my apps…", mounted.container));
	await mounted.settle();
	return sheet();
}

async function submit({ mounted }: Mounted) {
	await click(byRole("button", "Add to my models", sheet()));
	await mounted.settle();
}

function textOf(element: Element) {
	return element.textContent;
}

function badgeTexts(root: ParentNode) {
	return [...root.querySelectorAll("[data-device-model]")].map(textOf);
}

/** The desktop app's held keys, as the test sets them. */
function fakeNativeKeys() {
	let watcher: NativeKeyWatcher | undefined;
	let watches = 0;
	let stops = 0;
	const stop = () => {
		stops += 1;
	};
	const held = createHeldDevices({
		watch(next) {
			watches += 1;
			watcher = next;
			return stop;
		},
	});
	return {
		held,
		watches() {
			return watches;
		},
		stops() {
			return stops;
		},
		hold(deviceId: string) {
			watcher?.held([
				{
					deviceId,
					apiOrigin: "https://api.flow-like.test",
					account: "usr_me",
					kept: true,
					area: false,
				},
			]);
		},
		lockAll() {
			watcher?.lockedAll();
		},
	};
}

describe("use from my apps", () => {
	test("adds the hosted model to the user's models as a device Bit and keeps the device unlocked", async () => {
		const view = await mount(QWEN, { platform: "desktop" });
		await openSheet(view);
		expect(text(sheet())).toContain("Use Qwen3-8B Q4_K_M from your apps");
		expect(field("Name in your models").value).toBe(
			`Qwen3-8B Q4_K_M on ${DEVICE}`,
		);
		expect(field("Context length").value).toBe("16384");
		expect(text(sheet())).toContain("From the model's settings on the device.");
		expect(keepBox().getAttribute("aria-checked")).toBe("true");
		expect(text(sheet())).toContain(
			"Your flows on this computer can call its models until you lock it or quit Flow-Like.",
		);
		expect(text(sheet())).toContain(
			`Adds “Qwen3-8B Q4_K_M on ${DEVICE}” to your models, served by Qwen3-8B Q4_K_M on ${DEVICE}.`,
		);
		expect(text(sheet())).toContain("Cloud runs skip it");
		expect(text(sheet())).not.toMatch(MACHINE_WORDS);

		await typeInto(field("Name in your models"), "Edge Qwen");
		const commands = view.fake.api.commands.length;
		const writes = view.fake.api.writes().length;
		await submit(view);
		const [bit] = view.bits.saved;
		expect(view.bits.saved).toHaveLength(1);
		expect(bit.type).toBe(IBitTypes.Llm);
		expect(bit.meta.en.name).toBe("Edge Qwen");
		expect(bit.parameters).toMatchObject({
			context_length: 16_384,
			provider: {
				provider_name: "device",
				params: {
					device_id: edge,
					model: "qwen3-8b-q4",
					kind: "chat",
					api_surface: "chat_completions",
				},
			},
		});
		expect(view.bits.added).toEqual([`${bit.id} → profile-1`]);
		expect(text(sheet())).toContain(
			"“Edge Qwen” is in your models. Pick it in any model picker.",
		);
		expect(view.fake.workspace.keys.snapshot(edge).keepUnlocked).toBe(true);
		// Nothing goes to the device or its hub rows: the Bit lives in the user's models.
		expect(view.fake.api.commands).toHaveLength(commands);
		expect(view.fake.api.writes()).toHaveLength(writes);

		await click(byRole("button", "Done", sheet()));
		await view.mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
	});

	test("an embedding model needs its vector length before anything is saved", async () => {
		const view = await mount(NOMIC, { platform: "web" });
		await openSheet(view);
		expect(field("Input length").value).toBe("512");
		expect(text(sheet())).toContain(
			"The device picks it; enter what the model was loaded with.",
		);
		expect(text(sheet())).toContain(
			"Its models stay reachable from this window until you lock it or close the window.",
		);
		await submit(view);
		expect(view.bits.saved).toEqual([]);
		expect(text(sheet())).toContain(
			"Enter how many numbers the model returns per text, e.g. 768.",
		);
		await typeInto(field("Vector length"), "768");
		await submit(view);
		expect(view.bits.saved[0]?.type).toBe(IBitTypes.Embedding);
		expect(view.bits.saved[0]?.parameters).toMatchObject({
			vector_length: 768,
			input_length: 512,
			provider: { params: { model: "nomic-embed", kind: "embedding" } },
		});
		// A browser can't reach devices: its pickers leave the Bit out, so the sheet points to the desktop app.
		expect(text(sheet())).toContain(
			`Your flows and chats in the Flow-Like desktop app pick it like any other model, and its calls go to ${DEVICE}.`,
		);
		expect(text(sheet())).toContain(
			`“nomic-embed-text v1.5 on ${DEVICE}” is in your models. Pick it in the Flow-Like desktop app: only its runs reach your devices, so this browser's model pickers leave it out.`,
		);
		expect(text(sheet())).toContain(
			`Remove it from your models in the Flow-Like desktop app any time; the model stays on ${DEVICE}.`,
		);
		expect(text(sheet())).not.toContain("Pick it in any model picker");
	});

	test("adding the same model again updates its Bit instead of adding a twin", async () => {
		const view = await mount(QWEN);
		await openSheet(view);
		expect(text(sheet())).toContain(
			`Remove it from your models any time; the model stays on ${DEVICE}.`,
		);
		await submit(view);
		await click(byRole("button", "Done", sheet()));
		await view.mounted.settle();

		await openSheet(view);
		await typeInto(field("Name in your models"), "Edge Qwen");
		await submit(view);
		const [first, second] = view.bits.saved;
		expect(view.bits.saved).toHaveLength(2);
		expect(second.id).toBe(first.id);
		expect(second.meta.en.name).toBe("Edge Qwen");
	});

	test("a hub before device models refuses the Bit: the sheet says so and nothing else changes", async () => {
		const view = await mount(QWEN, {
			refuse: new ApiResponseError({
				status: 400,
				message:
					"Custom bit provider must be 'custom:<provider>' (remote backend), 'Local' (GGUF), or 'MLX'",
			}),
		});
		await openSheet(view);
		await submit(view);
		expect(view.bits.added).toEqual([]);
		const result = sheet().querySelector("[data-result=critical]");
		expect(result?.textContent).toBe(
			"This hub doesn't take models from devices yet, so nothing was added. Add it from the Flow-Like desktop app, or once the hub is updated.",
		);
		expect(text(sheet())).not.toMatch(MACHINE_WORDS);
		expect(view.fake.workspace.keys.snapshot(edge).keepUnlocked).toBe(false);
		// The form stays, so it can be sent again.
		expect(byRole("button", "Add to my models", sheet())).toBeTruthy();
	});

	test("unticking Keep unlocked leaves the idle lock on", async () => {
		const view = await mount(QWEN);
		await openSheet(view);
		await click(keepBox());
		await submit(view);
		expect(view.bits.saved).toHaveLength(1);
		expect(view.fake.workspace.keys.snapshot(edge)).toMatchObject({
			keepUnlocked: false,
			state: "unlocked",
		});
	});

	test("an agent that doesn't host models: the action is gated and opens nothing", async () => {
		const view = await mount(QWEN, { features: PRE_MODEL_FEATURES });
		const button = byRole(
			"button",
			"Use from my apps…",
			view.mounted.container,
		);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await click(button);
		await view.mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(view.bits.saved).toEqual([]);
	});
});

describe("device badge", () => {
	test("names the device and says where it runs", async () => {
		const { container } = await mountDevices(<DeviceModelBadge />);
		const badge = container.querySelector("[data-device-model]");
		expect(badge?.textContent).toBe("Device");
		expect(badge?.getAttribute("title")).toBe(
			"Runs on one of your devices. Desktop runs reach it through an encrypted tunnel; cloud runs skip it.",
		);
	});

	test("says whether a desktop run reaches the device without asking for its password", async () => {
		const native = fakeNativeKeys();
		const mounted = await mountDevices(
			<>
				<DeviceModelBadge deviceId="gpu-box" held={native.held} />
				<DeviceModelBadge deviceId="laptop" held={native.held} />
			</>,
		);
		expect(badgeTexts(mounted.container)).toEqual(["Device", "Device"]);
		expect(native.watches()).toBe(1);

		await act(async () => {
			native.hold("gpu-box");
		});
		expect(badgeTexts(mounted.container)).toEqual([
			"Device · unlocked",
			"Device · locked",
		]);
		const locked = mounted.container.querySelectorAll("[data-device-model]")[1];
		expect(locked?.getAttribute("data-device-state")).toBe("locked");
		expect(locked?.getAttribute("title")).toBe(
			"Runs on one of your devices, which is locked in this app: a desktop run asks for its password first. Cloud runs skip it.",
		);

		await act(async () => {
			native.lockAll();
		});
		expect(badgeTexts(mounted.container)).toEqual([
			"Device · locked",
			"Device · locked",
		]);
		await mounted.unmount();
		expect(native.stops()).toBe(1);
	});
});
