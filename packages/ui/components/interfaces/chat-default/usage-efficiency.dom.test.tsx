// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, expect, test, vi } from "vitest";
import type { IModelIdentity } from "../../../lib/bit/model-display-name";
import type { IChatUsageStat } from "../../../lib/llm/usage";
import { UsageStats } from "./usage-stats";

const identities = vi.hoisted(() => new Map<string, IModelIdentity>());

vi.mock("../../../hooks/use-model-names", () => ({
	useModelIdentities: () => identities,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, values: Record<string, unknown> = {}) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) =>
				String(values[key] ?? key),
			),
	}),
}));

let root: Root | undefined;
afterEach(async () => {
	await act(async () => root?.unmount());
	document.body.innerHTML = "";
	identities.clear();
});

async function open(stats: IChatUsageStat[]) {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	await act(async () => root?.render(<UsageStats stats={stats} />));
	const button = container.querySelector("button");
	expect(button).not.toBeNull();
	await act(async () => button?.click());
	expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
	return { body: document.body, chip: button as HTMLButtonElement };
}

async function showTab(name: string) {
	const tab = Array.from(
		document.body.querySelectorAll<HTMLElement>('[role="tab"]'),
	).find((element) => element.textContent?.startsWith(name));
	expect(tab).toBeDefined();
	await act(async () => {
		tab?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
	});
}

const usage = (prompt: number, completion: number, extra = {}) => ({
	prompt_tokens: prompt,
	completion_tokens: completion,
	total_tokens: prompt + completion,
	...extra,
});

test("opens cache, context and compaction details from the compact usage chip", async () => {
	const { body, chip } = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: usage(1_000, 100, {
					prompt_tokens_details: {
						cached_tokens: 600,
						cache_write_tokens: 100,
					},
				}),
				duration_ms: 2_400,
				context: {
					estimated_tokens: 900,
					measured_tokens: 1_000,
					context_window: 4_000,
					reserved_output_tokens: 500,
				},
				compactions: [
					{
						kind: "turn",
						trigger: "soft",
						status: "completed",
						before_tokens: 2_000,
						after_tokens: 500,
						duration_ms: 1_250,
						usage: usage(200, 10),
					},
				],
			},
		},
	]);
	expect(chip.textContent).toContain("1.1k tokens");
	expect(chip.textContent).toContain("2.4 s");
	expect(body.textContent).toContain(
		"60% of the input came from the prompt cache.",
	);
	await showTab("Context & cache");
	expect(
		body
			.querySelector('[role="meter"][aria-label="Cached input"]')
			?.getAttribute("aria-valuenow"),
	).toBe("60");
	expect(
		body
			.querySelector('[role="meter"][aria-label="Context window used"]')
			?.getAttribute("aria-valuenow"),
	).toBe("25");
	expect(body.textContent).toContain("Read from cache");
	expect(body.textContent).toContain("Written to cache");
	expect(body.textContent).toContain("75% smaller");
	expect(body.textContent).toContain("1.3 s");
	expect(body.textContent).toContain("210 tokens, included in usage");
});

test("old saved replies show unknown cache reporting without a misleading zero meter", async () => {
	const { body } = await open([
		{ step_name: "Assistant", stats: { usage: usage(50, 5) } },
	]);
	expect(body.textContent).toContain("No call reported cache use.");
	expect(body.textContent).not.toContain("0% of the");
	await showTab("Context & cache");
	expect(body.textContent).toContain("Not reported");
	expect(
		body.querySelector('[role="meter"][aria-label="Cached input"]'),
	).toBeNull();
	expect(body.textContent).toContain(
		"It may still have used the provider's cache",
	);
});

test("labels vision and background memory calls while counting only memory as maintenance", async () => {
	const { body } = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: usage(200, 20),
				calls: [
					{ model: "vision-model", purpose: "vision", usage: usage(100, 10) },
					{
						model: "memory-model",
						purpose: "memory_fact_update",
						usage: usage(100, 10),
					},
				],
			},
		},
	]);
	await showTab("Calls");
	expect(body.textContent).toContain("Image analysis");
	expect(body.textContent).toContain("Memory work");
	await showTab("Context & cache");
	expect(body.textContent).toContain("110 tokens, included in usage");
	expect(body.textContent).toContain(
		"Background memory work can be reported with the following reply",
	);
});

test("malformed optional metadata in a saved reply cannot break the usage popover", async () => {
	const { body } = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: usage(100, 10),
				calls: [null, { model: "model", usage: usage(100, 10), purpose: 123 }],
				context: { context_window: "bad" },
				compactions: { status: "bad" },
			},
		} as unknown as IChatUsageStat,
	]);
	expect(body.textContent).toContain("110");
	await showTab("Context & cache");
	expect(body.textContent).toContain("Prompt cache");
	expect(
		body.querySelector('[role="meter"][aria-label="Context window used"]'),
	).toBeNull();
});

test("names each model with its provider and pinned snapshot instead of the raw string", async () => {
	const { body } = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: usage(3_000, 300, { cost: 0.02 }),
				calls: [
					{
						model: "anthropic/claude-sonnet-4-5-20250929",
						usage: usage(2_000, 200, { cost: 0.02 }),
					},
					{ model: "tz4a98xxat96iws9zmbrgj3a", usage: usage(1_000, 100) },
				],
			},
		},
	]);
	const models = body.querySelector('[aria-label="Models"]');
	expect(models?.textContent).toContain("claude-sonnet-4-5");
	expect(models?.textContent).toContain("Anthropic · snapshot 2025-09-29");
	expect(models?.textContent).not.toContain("anthropic/claude");
	expect(models?.textContent).toContain("Unlisted model");
	expect(models?.textContent).toContain("tz4a98xxat96iws9zmbrgj3a");
});

test("shows catalog names, the exact model id and on-device runs for resolved models", async () => {
	identities.set("cm4x2ling30tiny8qabcdefgh", {
		raw: "cm4x2ling30tiny8qabcdefgh",
		name: "Ling 3.0 Tiny",
		provider: "On-device",
		onDevice: true,
		known: true,
		opaque: false,
		missing: false,
	});
	const { body } = await open([
		{
			step_name: "Memory",
			stats: {
				usage: usage(2_100, 180),
				calls: [
					{
						model: "cm4x2ling30tiny8qabcdefgh",
						purpose: "memory_extract",
						usage: usage(2_100, 180),
					},
				],
			},
		},
	]);
	const models = body.querySelector('[aria-label="Models"]');
	expect(models?.textContent).toContain("Ling 3.0 Tiny");
	expect(models?.textContent).toContain("On this device · 1 call");
	expect(models?.textContent).toContain("cm4x2ling30tiny8qabcdefgh");
	expect(models?.textContent).toContain("local");
	expect(body.textContent).toContain(
		"Ling 3.0 Tiny ran on this device, so no cost applies.",
	);
	await showTab("Calls");
	expect(body.textContent).toContain("Memory work · Ling 3.0 Tiny");
});
