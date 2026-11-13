// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, expect, test, vi } from "vitest";
import type { IChatUsageStat } from "../../../lib/llm/usage";
import { UsageStats } from "./usage-stats";

vi.mock("../../../hooks/use-model-names", () => ({
	useModelNames: () => new Map(),
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
	return document.body;
}

test("opens cache, context and compaction details from the compact usage badge", async () => {
	const usage = {
		prompt_tokens: 1_000,
		completion_tokens: 100,
		total_tokens: 1_100,
		prompt_tokens_details: { cached_tokens: 600, cache_write_tokens: 100 },
	};
	const body = await open([
		{
			step_name: "Assistant",
			stats: {
				usage,
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
						usage: {
							prompt_tokens: 200,
							completion_tokens: 10,
							total_tokens: 210,
						},
					},
				],
			},
		},
	]);
	expect(body.textContent).toContain("60% cached");
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
	const body = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: { prompt_tokens: 50, completion_tokens: 5, total_tokens: 55 },
			},
		},
	]);
	expect(body.textContent).toContain("Not reported");
	expect(body.textContent).not.toContain("0% cached");
	expect(
		body.querySelector('[role="meter"][aria-label="Cached input"]'),
	).toBeNull();
	expect(body.textContent).toContain(
		"It may still have used the provider's cache",
	);
});

test("labels vision and background memory calls while counting only memory as maintenance", async () => {
	const usage = {
		prompt_tokens: 100,
		completion_tokens: 10,
		total_tokens: 110,
	};
	const body = await open([
		{
			step_name: "Assistant",
			stats: {
				usage: { prompt_tokens: 200, completion_tokens: 20, total_tokens: 220 },
				calls: [
					{ model: "vision-model", purpose: "vision", usage },
					{ model: "memory-model", purpose: "memory_fact_update", usage },
				],
			},
		},
	]);
	expect(body.textContent).toContain("Image analysis");
	expect(body.textContent).toContain("Memory work");
	expect(body.textContent).toContain("110 tokens, included in usage");
	expect(body.textContent).toContain(
		"Background memory work can be reported with the following reply",
	);
});

test("malformed optional metadata in a saved reply cannot break the usage sheet", async () => {
	const usage = {
		prompt_tokens: 100,
		completion_tokens: 10,
		total_tokens: 110,
	};
	const body = await open([
		{
			step_name: "Assistant",
			stats: {
				usage,
				calls: [null, { model: "model", usage, purpose: 123 }],
				context: { context_window: "bad" },
				compactions: { status: "bad" },
			},
		} as unknown as IChatUsageStat,
	]);
	expect(body.textContent).toContain("Prompt cache");
	expect(body.textContent).toContain("110");
	expect(
		body.querySelector('[role="meter"][aria-label="Context window used"]'),
	).toBeNull();
});
