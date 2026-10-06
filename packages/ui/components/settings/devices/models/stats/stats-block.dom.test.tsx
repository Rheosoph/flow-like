import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	settle,
} from "../../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
await preloadDevices();
const samples = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { SAMPLE_IDS } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { useObserveTarget } = await import("../../observe/use-observe-target");
const { useDevicesRoute } = await import("../../routing/use-devices-route");
const { useLiveSession } = await import("../../workspace");
const { useModelsOverview } = await import("../use-models");
const { ModelStatsBlock } = await import("./stats-block");

const { MODEL_HOST_FEATURES, emptyModels, gpuBoxModels } = samples;
const { edge } = SAMPLE_IDS;
const HOUR = 3_600;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

/** R3: wire values and parameter keys never reach the screen. */
const WIRE =
	/placement|grant|model_id|ttft_p|queue_wait|decode_ms|prompt_tokens|completion_tokens|llamacpp|\{\{|undefined|NaN/;

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

/** The Models tab's place for the block: the device's overview, live. */
function Harness({ deviceId }: Readonly<{ deviceId: string }>) {
	const { scope } = useDevicesRoute();
	useLiveSession(deviceId, { demand: true });
	const target = useObserveTarget(deviceId, null, scope);
	const overview = useModelsOverview(deviceId);
	if (!target.known || !overview.data) return <p data-waiting="" />;
	return <ModelStatsBlock target={target} overview={overview.data} />;
}

type View = Awaited<ReturnType<typeof mountDevices>>;

async function until(view: View, done: () => boolean, what: string) {
	for (let round = 0; round < 60; round++) {
		if (done()) return;
		await view.settle();
	}
	throw new Error(`Timed out waiting for ${what}: ${text(view.container)}`);
}

async function mount(sample = gpuBoxModels()) {
	const view = await mountDevices(<Harness deviceId={edge} />, {
		agentFeatures: { [edge]: MODEL_HOST_FEATURES },
		modelHosts: { [edge]: sample },
	});
	await until(
		view,
		() => !view.container.querySelector("[data-waiting]"),
		"the overview",
	);
	return view;
}

const block = (view: View) =>
	view.container.querySelector("#models-stats") as HTMLElement;

const chart = (view: View, title: string) =>
	[...view.container.querySelectorAll("figure[data-chart]")].find(
		(figure) => figure.querySelector("figcaption span")?.textContent === title,
	) as HTMLElement | undefined;

const statsRequests = (view: View) =>
	view.fake.api.commands
		.filter(([, type]) => type === "models")
		.map(([, , command]) => command.request as Record<string, unknown>)
		.filter((request) => request.kind === "stats");

async function charted(view: View) {
	await until(view, () => !!chart(view, "Tokens"), "the charts");
}

async function untilSaid(view: View, sentence: string) {
	const said = () => text(block(view)).includes(sentence);
	await until(view, said, sentence);
}

async function untilRead(view: View, modelId: string) {
	const read = () => statsRequests(view).at(-1)?.model_id === modelId;
	await until(view, read, `the read of ${modelId}`);
}

async function pickModel(view: View, name: string) {
	await click(byRole("combobox", "Model", block(view)));
	await click(byRole("option", name));
	await view.settle();
}

describe("usage and performance", () => {
	test("all models over the last day: tokens, requests, timings and who called, with the read's stamp", async () => {
		const view = await mount();
		await charted(view);
		const root = block(view);
		expect(text(root)).toContain("Usage and performance");
		expect(root.querySelector("[data-stamp]")).not.toBeNull();
		const tokens = chart(view, "Tokens") as HTMLElement;
		expect(text(tokens)).toContain("1.7M in all");
		expect(text(tokens.querySelector("[data-legend]") as Element)).toBe(
			"In1.3MOut424.3K",
		);
		expect(tokens.querySelectorAll("[data-column]")).toHaveLength(24);
		expect(text(chart(view, "Requests") as HTMLElement)).toContain(
			"4.4K in all",
		);
		expect(
			text(
				(chart(view, "Requests") as HTMLElement).querySelector(
					"[data-legend]",
				) as Element,
			),
		).toContain("Failed6");
		expect(text(chart(view, "Time to first token") as HTMLElement)).toContain(
			"up to 1.9 s",
		);
		expect(chart(view, "Generation speed")).toBeDefined();
		expect(text(chart(view, "Queue wait") as HTMLElement)).toContain(
			"up to 1.8 s",
		);
		const [request] = statsRequests(view);
		expect(request).toMatchObject({ model_id: null, step: "hour" });
		expect(((request?.to as number) - (request?.from as number)) / HOUR).toBe(
			24,
		);
		const callers = byRole("table", "Requests by caller", root);
		const names = [...callers.querySelectorAll("tbody tr td:first-child")].map(
			(cell) => cell.textContent,
		);
		expect(names).toEqual(["invoice-extractor", "You", "One person"]);
		expect(text(root)).not.toMatch(WIRE);
	});

	test("a longer range asks for its hours and sums them into wider columns", async () => {
		const view = await mount();
		await charted(view);
		await click(byRole("button", "7 d", block(view)));
		await until(
			view,
			() =>
				chart(view, "Tokens")?.querySelectorAll("[data-column]").length === 28,
			"six-hour columns",
		);
		const request = statsRequests(view).at(-1);
		expect(((request?.to as number) - (request?.from as number)) / HOUR).toBe(
			168,
		);
		expect(
			byRole("button", "7 d", block(view)).getAttribute("aria-pressed"),
		).toBe("true");
	});

	test("one model's statistics: an embedding model has no speed chart", async () => {
		const view = await mount();
		await charted(view);
		await pickModel(view, "Nomic Embed v1.5");
		await until(
			view,
			() => statsRequests(view).at(-1)?.model_id === "nomic-embed-v1.5",
			"the model's read",
		);
		await until(view, () => !chart(view, "Generation speed"), "no speed chart");
		expect(chart(view, "Time to first token")).toBeDefined();
		expect(text(chart(view, "Tokens") as HTMLElement)).toContain("Out0");
	});

	test("a model nobody called says so", async () => {
		const view = await mount();
		await charted(view);
		await pickModel(view, "Gemma 3 4B Q4_K_M");
		await until(
			view,
			() =>
				text(block(view)).includes(
					"No requests to Gemma 3 4B Q4_K_M in the last 24 hours.",
				),
			"the empty state",
		);
		expect(chart(view, "Tokens")).toBeUndefined();
	});

	test("a refused read says why and reads again on request", async () => {
		const view = await mount();
		const restore = view.fake
			.agent(edge)
			.models.refuse("stats", "busy", "The model host is starting.");
		await click(byRole("button", "7 d", block(view)));
		await until(
			view,
			() => text(block(view)).includes("didn't return its usage statistics"),
			"the failure",
		);
		expect(text(block(view))).toContain("“The model host is starting.”");
		restore();
		await click(byRole("button", "Try again", block(view)));
		await until(view, () => !!chart(view, "Tokens"), "the charts again");
		expect(
			chart(view, "Tokens")?.querySelectorAll("[data-column]"),
		).toHaveLength(28);
	});

	test("ninety days arrive as one bulk read, larger than a management reply", async () => {
		const view = await mount();
		await charted(view);
		await click(byRole("button", "90 d", block(view)));
		await until(
			view,
			() =>
				chart(view, "Tokens")?.querySelectorAll("[data-column]").length === 90,
			"daily columns",
		);
		const request = statsRequests(view).at(-1);
		expect(((request?.to as number) - (request?.from as number)) / HOUR).toBe(
			90 * 24,
		);
	});

	test("an answer too large for the connection says what to pick instead, with no retry that can't succeed", async () => {
		const view = await mount();
		await charted(view);
		view.fake
			.agent(edge)
			.models.refuse(
				"stats",
				"limit",
				"The answer exceeds one management message.",
			);
		await click(byRole("button", "7 d", block(view)));
		await untilSaid(view, "didn't return its usage statistics");
		expect(text(block(view))).toContain(
			"They are larger than edge-berlin-01 can send over this connection. Pick a shorter range or a single model.",
		);
		expect(queryByRole("button", "Try again", block(view))).toBeNull();
	});

	test("while another model's statistics load, a previous empty answer doesn't speak for it", async () => {
		const view = await mount();
		await charted(view);
		await pickModel(view, "Gemma 3 4B Q4_K_M");
		await untilSaid(
			view,
			"No requests to Gemma 3 4B Q4_K_M in the last 24 hours.",
		);
		const release = view.fake.agent(edge).hold("models");
		await pickModel(view, "Qwen3-8B Q4_K_M");
		await untilRead(view, "qwen3-8b");
		for (let round = 0; round < 5; round++) await view.settle();
		expect(text(block(view))).not.toContain("No requests to");
		expect(text(block(view))).toContain(
			"Reading usage statistics from edge-berlin-01…",
		);
		release();
		await charted(view);
	});

	test("the playground opens from the block's head", async () => {
		const view = await mount();
		await charted(view);
		const open = byRole("button", "Try a model…", block(view));
		expect(open.getAttribute("aria-disabled")).toBeNull();
	});

	test("a device without models shows no statistics and reads none", async () => {
		const view = await mount(emptyModels());
		await settle();
		expect(block(view)).toBeNull();
		expect(queryByRole("button", "Try a model…", view.container)).toBeNull();
		expect(statsRequests(view)).toEqual([]);
	});
});
