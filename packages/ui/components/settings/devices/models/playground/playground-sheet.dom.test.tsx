import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../../testing/dom-harness";
import { fakeGateway, sse } from "./fake-gateway";

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
const { useLiveSession } = await import("../../workspace");
const { useModelsOverview } = await import("../use-models");
const { PlaygroundButton } = await import("./playground-sheet");

const { MODEL_HOST_FEATURES, gpuBoxModels } = samples;
const { edge } = SAMPLE_IDS;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

const CHAT = [
	sse({ choices: [{ delta: { content: "Hello" } }] }),
	sse({ choices: [{ delta: { content: " there" } }] }),
	sse({ choices: [], usage: { prompt_tokens: 5, completion_tokens: 2 } }),
	"data: [DONE]\n\n",
];

function Harness({ deviceId }: Readonly<{ deviceId: string }>) {
	useLiveSession(deviceId, { demand: true });
	const overview = useModelsOverview(deviceId).data;
	if (!overview) return <p data-waiting="" />;
	return (
		<PlaygroundButton
			deviceId={deviceId}
			device="edge-berlin-01"
			models={overview.models}
		/>
	);
}

type View = Awaited<ReturnType<typeof mountDevices>>;

async function until(view: View, done: () => boolean, what: string) {
	for (let round = 0; round < 80; round++) {
		if (done()) return;
		await view.settle();
	}
	throw new Error(`Timed out waiting for ${what}: ${text(document.body)}`);
}

/** The GPU box with Qwen3-8B not loaded: its first request loads it. */
function qwenStopped() {
	const sample = gpuBoxModels();
	sample.models = sample.models.map((model) => {
		if (model.id !== "qwen3-8b" || model.state !== "loaded") return model;
		const {
			ram_bytes: _ram,
			vram_bytes: _vram,
			slots: _slots,
			slots_busy: _busy,
			...rest
		} = model;
		return { ...rest, state: "stopped" };
	});
	return sample;
}

/** A promise the test settles; a fake gateway's chunk after it waits until then. */
function gate() {
	let release!: () => void;
	const held = new Promise<void>((resolve) => {
		release = resolve;
	});
	return { held, release };
}

/**
 * Mounts the button; with `gateway`, the live session offers the model
 * gateway as requested of the owner; with `null` it offers none, like a build
 * from before the gateway.
 */
async function mount(
	gateway: ReturnType<typeof fakeGateway> | null,
	sample = gpuBoxModels(),
) {
	const view = await mountDevices(<Harness deviceId={edge} />, {
		agentFeatures: { [edge]: MODEL_HOST_FEATURES },
		modelHosts: { [edge]: sample },
	});
	const holds: string[] = [];
	const live = view.fake.workspace.live as unknown as Record<string, unknown>;
	if (!gateway) live.openModelGateway = undefined;
	else {
		live.openModelGateway = gateway.open;
		const keys = view.fake.workspace.keys;
		const begin = keys.beginModelUse.bind(keys);
		keys.beginModelUse = (deviceId: string) => {
			holds.push(`hold ${deviceId}`);
			const end = begin(deviceId);
			return () => {
				holds.push("release");
				end();
			};
		};
	}
	await until(
		view,
		() => !view.container.querySelector("[data-waiting]"),
		"the overview",
	);
	await click(byRole("button", "Try a model…", view.container));
	await until(view, () => queryByRole("dialog") !== null, "the sheet");
	return { view, holds };
}

const sheet = () => inPortal("dialog");

describe("the playground", () => {
	test("streams a chat answer through the gateway and shows how fast it came", async () => {
		const gateway = fakeGateway(() => ({
			type: "text/event-stream",
			chunks: CHAT,
		}));
		const { view, holds } = await mount(gateway);
		expect(text(sheet())).toContain("Try a model on edge-berlin-01");
		await typeInto(byRole("textbox", "Message", sheet()), "Hi");
		await click(byRole("button", "Send", sheet()));
		await until(
			view,
			() => !!sheet().querySelector("[data-measure]"),
			"the measurements",
		);
		const transcript = sheet().querySelector("[data-transcript]") as Element;
		expect(text(transcript)).toContain("YouHi");
		expect(text(transcript)).toContain("Qwen3-8B Q4_K_MHello there");
		const measure = text(sheet().querySelector("[data-measure]") as Element);
		expect(measure).toMatch(
			/^First token \d+ ms · .* · 5 tokens in, 2 out · \d+ ms in all$/,
		);
		expect(gateway.requests).toHaveLength(1);
		expect(gateway.requests[0]).toMatchObject({
			method: "POST",
			path: "/v1/chat/completions",
		});
		expect(JSON.parse(gateway.requests[0]?.body ?? "{}")).toMatchObject({
			model: "qwen3-8b",
			messages: [{ role: "user", content: "Hi" }],
			stream: true,
		});
		await until(view, () => holds.length === 2, "the hold to end");
		expect(holds).toEqual([`hold ${edge}`, "release"]);
	});

	test("the next message carries the conversation so far", async () => {
		const gateway = fakeGateway(() => ({
			type: "text/event-stream",
			chunks: CHAT,
		}));
		const { view } = await mount(gateway);
		for (const message of ["Hi", "Again"]) {
			await typeInto(byRole("textbox", "Message", sheet()), message);
			await click(byRole("button", "Send", sheet()));
			await until(
				view,
				() => queryByRole("button", "Send", sheet()) !== null,
				"the answer",
			);
		}
		expect(JSON.parse(gateway.requests[1]?.body ?? "{}").messages).toEqual([
			{ role: "user", content: "Hi" },
			{ role: "assistant", content: "Hello there" },
			{ role: "user", content: "Again" },
		]);
	});

	test("an embedding model embeds text and shows the vector's size", async () => {
		const vector = Array.from({ length: 768 }, (_, index) => index / 1000);
		const gateway = fakeGateway(() => ({
			chunks: [
				JSON.stringify({
					data: [{ embedding: vector }],
					usage: { prompt_tokens: 9 },
				}),
			],
		}));
		const { view } = await mount(gateway);
		const models = byRole("combobox", "Model", sheet());
		expect(models.textContent).toBe("Qwen3-8B Q4_K_M · Loaded");
		await click(models);
		const options = [...document.querySelectorAll("[role=option]")].map(
			(option) => option.textContent,
		);
		expect(options).toEqual([
			"Nomic Embed v1.5 · Loaded",
			"Qwen3-8B Q4_K_M · Loaded",
		]);
		await click(byRole("option", "Nomic Embed v1.5 · Loaded"));
		await typeInto(byRole("textbox", "Text", sheet()), "invoice total");
		await click(byRole("button", "Embed", sheet()));
		await until(
			view,
			() => !!sheet().querySelector("[data-embedding]"),
			"the vector",
		);
		const result = text(sheet().querySelector("[data-embedding]") as Element);
		expect(result).toMatch(/^768 dimensions · 9 tokens · \d+ ms/);
		expect(result).toContain("[0.0000, 0.0010, 0.0020");
		expect(JSON.parse(gateway.requests[0]?.body ?? "{}")).toEqual({
			model: "nomic-embed-v1.5",
			input: "invoice total",
		});
	});

	test("a refusal shows the gateway's reason in the device's words", async () => {
		const gateway = fakeGateway(() => ({
			status: 503,
			chunks: [
				JSON.stringify({
					error: { message: "Qwen3-8B is loading", type: "model_unavailable" },
				}),
			],
		}));
		const { view } = await mount(gateway);
		await typeInto(byRole("textbox", "Message", sheet()), "Hi");
		await click(byRole("button", "Send", sheet()));
		await until(
			view,
			() => !!sheet().querySelector("[data-result=critical]"),
			"the failure",
		);
		expect(text(sheet())).toContain(
			"The model can't answer right now. “Qwen3-8B is loading”",
		);
	});

	test("a message that got no answer leaves the transcript and returns to the box, so roles keep alternating", async () => {
		let calls = 0;
		const gateway = fakeGateway(() =>
			++calls === 1
				? {
						status: 503,
						chunks: [
							JSON.stringify({
								error: { message: "Qwen3-8B is loading" },
							}),
						],
					}
				: { type: "text/event-stream", chunks: CHAT },
		);
		const { view } = await mount(gateway);
		const box = () =>
			byRole("textbox", "Message", sheet()) as HTMLTextAreaElement;
		await typeInto(box(), "Hi");
		await click(byRole("button", "Send", sheet()));
		await until(
			view,
			() => !!sheet().querySelector("[data-result=critical]"),
			"the failure",
		);
		expect(sheet().querySelector("[data-transcript]")).toBeNull();
		expect(box().value).toBe("Hi");
		await click(byRole("button", "Send", sheet()));
		await until(
			view,
			() => !!sheet().querySelector("[data-measure]"),
			"the answer",
		);
		expect(JSON.parse(gateway.requests[1]?.body ?? "{}").messages).toEqual([
			{ role: "user", content: "Hi" },
		]);
	});

	test("the transcript is announced once the answer is complete, not token by token", async () => {
		const middle = gate();
		const gateway = fakeGateway(() => ({
			type: "text/event-stream",
			chunks: [CHAT[0] as string, middle.held, ...CHAT.slice(1)],
		}));
		const { view } = await mount(gateway);
		await typeInto(byRole("textbox", "Message", sheet()), "Hi");
		await click(byRole("button", "Send", sheet()));
		const transcript = () =>
			sheet().querySelector("[data-transcript]") as Element;
		await until(
			view,
			() => text(transcript()).includes("Hello"),
			"the first tokens",
		);
		expect(transcript().getAttribute("aria-live")).toBe("polite");
		expect(transcript().getAttribute("aria-busy")).toBe("true");
		middle.release();
		await until(
			view,
			() => !!sheet().querySelector("[data-measure]"),
			"the answer",
		);
		expect(transcript().hasAttribute("aria-busy")).toBe(false);
		expect(text(transcript())).toContain("Hello there");
	});

	test("a model that isn't loaded says the device loads it first while the answer waits", async () => {
		const first = gate();
		const gateway = fakeGateway(() => ({
			type: "text/event-stream",
			chunks: [first.held, ...CHAT],
		}));
		const { view } = await mount(gateway, qwenStopped());
		await click(byRole("combobox", "Model", sheet()));
		await click(byRole("option", "Qwen3-8B Q4_K_M · Not loaded"));
		await typeInto(byRole("textbox", "Message", sheet()), "Hi");
		await click(byRole("button", "Send", sheet()));
		await until(
			view,
			() =>
				text(sheet()).includes(
					"Waiting for edge-berlin-01 to load Qwen3-8B Q4_K_M. The first answer can take a few minutes.",
				),
			"the loading note",
		);
		first.release();
		await until(
			view,
			() => !!sheet().querySelector("[data-measure]"),
			"the answer",
		);
	});

	test("without the gateway in this version, the sheet says so instead of failing", async () => {
		await mount(null);
		expect(text(sheet())).toContain(
			"This version of Flow-Like can't send requests to models on devices yet.",
		);
		expect(queryByRole("textbox", "Message", sheet())).toBeNull();
	});
});

test("device decision models use the native questions playground", async () => {
	const sample = gpuBoxModels();
	sample.models = sample.models
		.slice(0, 1)
		.map((model) => ({ ...model, kind: "systemone" as const }));
	const answers = {
		model: sample.models[0]?.id,
		answers: {
			sentiment: {
				type: "choice",
				choice: "negative",
				confidence: 0.95,
				probabilities: { negative: 0.95, neutral: 0.04, positive: 0.01 },
			},
		},
	};
	const gateway = fakeGateway(() => ({
		type: "application/json",
		chunks: [JSON.stringify(answers)],
	}));
	const { view } = await mount(gateway, sample);
	expect(sheet().querySelector('[data-playground="systemone"]')).not.toBeNull();
	expect(queryByRole("textbox", "Message", sheet())).toBeNull();
	await click(byRole("button", "Answer questions", sheet()));
	await until(
		view,
		() => !!sheet().querySelector('[aria-label="SystemOne answers"]'),
		"the typed decision",
	);
	expect(text(sheet())).toContain('"negative"');
	expect(gateway.requests[0]?.path).toBe("/v1/systemone");
	const body = JSON.parse(gateway.requests[0]?.body ?? "{}");
	expect(body.model).toBe(sample.models[0]?.id);
	expect(body.questions.sentiment.type).toBe("choice");
	expect(body.stream).toBeUndefined();
	expect(body.messages).toBeUndefined();
});
