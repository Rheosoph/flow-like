import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const kit = await import("../device/device-test-kit");
const samples = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-models"
);

const { IDS, openDevice } = kit;
const {
	MAC_MODEL_HOST_FEATURES,
	MODEL_HOST_FEATURES,
	gpuBoxModels,
	macMiniModels,
} = samples;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

type Sample = ReturnType<typeof gpuBoxModels>;

/** The Models tab of `deviceId`, its recommendations read. */
function open(
	deviceId: string,
	sample: Sample,
	features = MODEL_HOST_FEATURES,
) {
	return openDevice(deviceId, {
		tab: "models",
		agentFeatures: { [deviceId]: features },
		modelHosts: { [deviceId]: sample },
	});
}

type View = Awaited<ReturnType<typeof open>>;

const recommendations = (view: View) =>
	view.container.querySelector("#models-attention") as HTMLElement;

async function until(view: View, done: () => boolean, what: string) {
	for (let round = 0; round < 60; round++) {
		if (done()) return;
		await view.settle();
	}
	throw new Error(
		`Timed out waiting for ${what}: ${view.container.textContent}`,
	);
}

const writes = (view: View, deviceId: string) =>
	view.fake.agent(deviceId).models.writes;

async function confirm(view: View) {
	const sheet = inPortal("alertdialog");
	await click(sheet.querySelector("[data-confirm]") as HTMLElement);
	await view.settle();
}

describe("one-click fixes on the Models tab", () => {
	test("each recommendation with a fix offers it by what it changes", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		for (const label of [
			"Use an 8-bit cache…",
			"Use 6 slots…",
			"Apply the recommended settings…",
		])
			expect(
				byRole("button", label, recommendations(view)).getAttribute(
					"aria-disabled",
				),
			).toBeNull();
	});

	test("a settings fix confirms, then configures the model at the revision the device made it for", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		await click(byRole("button", "Use an 8-bit cache…", recommendations(view)));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain(
			"Qwen3-8B Q4_K_M lets the requests in flight finish, then restarts with the new settings.",
		);
		await confirm(view);
		await until(
			view,
			() => writes(view, IDS.edge).length === 1,
			"the configure write",
		);
		expect(writes(view, IDS.edge)[0]).toMatchObject({
			kind: "configure",
			model_id: "qwen3-8b",
			expected_revision: 3,
			settings: { kv_cache_type: "q8_0", parallel: 4, ctx_per_slot: 8_192 },
			residency: { mode: "always_on" },
		});
		await until(
			view,
			() =>
				queryByRole("button", "Use 6 slots…", recommendations(view)) === null,
			"the stale fixes to go",
		);
		expect(
			queryByRole(
				"button",
				"Apply the recommended settings…",
				recommendations(view),
			),
		).not.toBeNull();
	});

	test("a runtime fix installs the pack the device names", async () => {
		const view = await open(
			IDS.studio,
			macMiniModels(),
			MAC_MODEL_HOST_FEATURES,
		);
		await click(
			byRole("button", "Install llama.cpp for Metal…", recommendations(view)),
		);
		await confirm(view);
		await until(
			view,
			() => writes(view, IDS.studio).length === 1,
			"the install write",
		);
		expect(writes(view, IDS.studio)[0]).toMatchObject({
			kind: "install_runtime",
			runtime: "llamacpp",
			backend: "metal",
		});
	});

	test("an outdated pack updates from its recommendation, without the installed build's size", async () => {
		const sample = macMiniModels();
		sample.recommendations = [
			{
				code: "runtime_outdated",
				tier: "later",
				params: { latest: "b11000" },
				fix: { kind: "install_runtime", runtime: "llamacpp", backend: "cpu" },
			},
		];
		const view = await open(IDS.studio, sample, MAC_MODEL_HOST_FEATURES);
		expect(text(recommendations(view))).toContain(
			"A newer build of llama.cpp for CPU, b11000, is available for studio-mac-mini.",
		);
		await click(
			byRole("button", "Update llama.cpp for CPU…", recommendations(view)),
		);
		const sheet = text(inPortal("alertdialog"));
		expect(sheet).toContain("Update llama.cpp for CPU?");
		expect(sheet).toContain(
			"downloads the newest signed llama.cpp for CPU runtime and checks its signature before it replaces the installed build.",
		);
		expect(sheet).not.toMatch(/MiB|GiB/);
		await confirm(view);
		await until(
			view,
			() => writes(view, IDS.studio).length === 1,
			"the update write",
		);
		expect(writes(view, IDS.studio)[0]).toMatchObject({
			kind: "install_runtime",
			runtime: "llamacpp",
			backend: "cpu",
		});
	});

	test("one outdated recommendation per pack: each names its pack and keeps its own row", async () => {
		const sample = macMiniModels();
		sample.runtimes = sample.runtimes.map((pack) => ({
			...pack,
			installed: true,
		}));
		const outdated = (backend: "cpu" | "metal") => ({
			code: "runtime_outdated" as const,
			tier: "later" as const,
			params: { latest: "b11000" },
			fix: {
				kind: "install_runtime" as const,
				runtime: "llamacpp" as const,
				backend,
			},
		});
		sample.recommendations = [outdated("cpu"), outdated("metal")];
		const warnings: string[] = [];
		const error = console.error;
		console.error = (...args: unknown[]) => {
			warnings.push(args.map(String).join(" "));
		};
		try {
			const view = await open(IDS.studio, sample, MAC_MODEL_HOST_FEATURES);
			const rows = [
				...recommendations(view).querySelectorAll("[data-attention]"),
			].map(text);
			expect(rows).toHaveLength(2);
			expect(rows[0]).toContain("A newer build of llama.cpp for CPU, b11000");
			expect(rows[1]).toContain("A newer build of llama.cpp for Metal, b11000");
			for (const label of [
				"Update llama.cpp for CPU…",
				"Update llama.cpp for Metal…",
			])
				expect(byRole("button", label, recommendations(view))).toBeTruthy();
		} finally {
			console.error = error;
		}
		expect(warnings.filter((line) => line.includes("same key"))).toEqual([]);
	});
});
