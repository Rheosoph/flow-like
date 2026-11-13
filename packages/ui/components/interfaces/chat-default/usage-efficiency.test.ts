import { describe, expect, test } from "bun:test";
import { type IChatUsageStat, readUsageStat } from "../../../lib/llm/usage";
import { usageEfficiency } from "./usage-efficiency";

const usage = (prompt: number, cached?: number, written?: number) => ({
	prompt_tokens: prompt,
	completion_tokens: 10,
	total_tokens: prompt + 10,
	...(cached !== undefined || written !== undefined
		? {
				prompt_tokens_details: {
					cached_tokens: cached,
					cache_write_tokens: written,
				},
			}
		: {}),
});

describe("cache and context accounting", () => {
	test("delegated model work does not replace the assistant context window", () => {
		const stats: IChatUsageStat[] = [
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_000),
					context: {
						estimated_tokens: 1_000,
						context_window: 10_000,
						reserved_output_tokens: 100,
					},
				},
			},
			{
				step_name: "Delegated model work",
				stats: {
					usage: usage(500),
					context: {
						estimated_tokens: 500,
						context_window: 2_000,
						reserved_output_tokens: 100,
					},
				},
			},
		];
		for (const records of [stats, stats.toReversed()]) {
			const result = usageEfficiency(records);
			expect(result.context?.step).toBe("Assistant");
			expect(result.context?.context_window).toBe(10_000);
		}
	});

	test("retained call histories report partial cache coverage against complete input totals", () => {
		const result = usageEfficiency([
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_000, 500),
					calls: [{ model: "a", usage: usage(100, 80) }],
					calls_omitted: 8,
					compactions_omitted: 2,
				},
			},
		]);
		expect(result.cacheShare).toBe(0.8);
		expect(result.cacheReadTokens).toBe(500);
		expect(result.inputTokens).toBe(1_000);
		expect(result.reportedInputTokens).toBe(100);
		expect(result.partialReads).toBe(true);
		expect(result.callsOmitted).toBe(8);
		expect(result.compactionsOmitted).toBe(2);
	});

	test("all-omitted calls preserve known counts without inventing reporting coverage", () => {
		const result = usageEfficiency([
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_000, 500),
					calls_omitted: 10,
					context: {
						estimated_tokens: 100,
						context_window: 0,
						reserved_output_tokens: 20,
					},
				},
			},
		]);
		expect(result.cacheReadTokens).toBe(500);
		expect(result.cacheShare).toBeNull();
		expect(result.context).toBeNull();
	});

	test("incomplete legacy call lists disclose partial coverage even without omission counts", () => {
		const result = usageEfficiency([
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_000, 500, 100),
					calls: [{ model: "a", usage: usage(100, 80, 10) }],
				},
			},
		]);
		expect(result.cacheShare).toBe(0.8);
		expect(result.reportedInputTokens).toBe(100);
		expect(result.partialReads).toBe(true);
		expect(result.partialWrites).toBe(true);
	});

	test("malformed optional saved metadata is ignored before any aggregation", () => {
		const broken = {
			step_name: "Assistant",
			stats: {
				usage: usage(100),
				calls: [
					null,
					{ model: 42, usage: usage(100, 25), purpose: 123 },
					{ usage: "bad" },
				],
				context: { estimated_tokens: 2, context_window: "bad" },
				compactions: "broken",
			},
		};
		const parsed = readUsageStat(broken);
		expect(parsed?.stats.calls).toEqual([{ model: "", usage: usage(100, 25) }]);
		expect(parsed?.stats.context).toBeUndefined();
		expect(parsed?.stats.compactions).toBeUndefined();
		expect(
			usageEfficiency([broken as unknown as IChatUsageStat]).cacheShare,
		).toBe(0.25);
		expect(
			readUsageStat({
				stats: {
					usage: usage(100),
					compactions: [
						null,
						{},
						{
							kind: "turn",
							trigger: "hard",
							status: "failed",
							before_tokens: 100,
							duration_ms: 20,
							usage: 3,
						},
					],
				},
			})?.stats.compactions,
		).toEqual([
			{
				kind: "turn",
				trigger: "hard",
				status: "failed",
				before_tokens: 100,
				duration_ms: 20,
			},
		]);
	});

	test("distinguishes an explicit cache miss from an unreported count", () => {
		const known = usageEfficiency([
			{ step_name: "Assistant", stats: { usage: usage(100, 0, 0) } },
		]);
		const unknown = usageEfficiency([
			{ step_name: "Assistant", stats: { usage: usage(100) } },
		]);
		expect(known.cacheShare).toBe(0);
		expect(known.cacheReadTokens).toBe(0);
		expect(known.cacheWriteTokens).toBe(0);
		expect(unknown.cacheShare).toBeNull();
		expect(unknown.cacheReadTokens).toBeNull();
	});

	test("weights reported calls by input and exposes partial coverage without double counting totals", () => {
		const result = usageEfficiency([
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_000, 250, 20),
					calls: [
						{ model: "a", usage: usage(100, 50, 20) },
						{ model: "b", usage: usage(300, 200) },
						{ model: "c", usage: usage(600) },
					],
				},
			},
		]);
		expect(result.cacheShare).toBe(250 / 400);
		expect(result.cacheReadTokens).toBe(250);
		expect(result.cacheWriteTokens).toBe(20);
		expect(result.reportedInputTokens).toBe(400);
		expect(result.inputTokens).toBe(1_000);
		expect(result.partialReads).toBe(true);
		expect(result.partialWrites).toBe(true);
	});

	test("retains failed/reused compactions, prefers measured context, and counts maintenance once", () => {
		const compactUsage = usage(200, 100);
		const result = usageEfficiency([
			{
				step_name: "Assistant",
				stats: {
					usage: usage(1_200),
					context: {
						estimated_tokens: 700,
						measured_tokens: 800,
						context_window: 2_000,
						reserved_output_tokens: 200,
					},
					calls: [
						{ model: "a", usage: compactUsage, purpose: "context_compaction" },
					],
					compactions: [
						{
							kind: "turn",
							trigger: "soft",
							status: "completed",
							before_tokens: 1_000,
							after_tokens: 300,
							duration_ms: 50,
							usage: compactUsage,
						},
						{
							kind: "conversation",
							trigger: "hard",
							status: "failed",
							before_tokens: 1_000,
							duration_ms: 10,
						},
						{
							kind: "conversation",
							trigger: "carry",
							status: "reused",
							before_tokens: 1_000,
							after_tokens: 300,
							duration_ms: 0,
						},
					],
				},
			},
		]);
		expect(result.context?.tokens).toBe(800);
		expect(result.context?.measured).toBe(true);
		expect(result.maintenanceTokens).toBe(210);
		expect(result.compactions.map((event) => event.status)).toEqual([
			"completed",
			"failed",
			"reused",
		]);
	});

	test("rejects malformed usage and preserves every additive field through stream parsing", () => {
		const payload: IChatUsageStat = {
			step_name: "Assistant",
			source_id: "native:child:repair",
			stats: {
				usage: usage(100, 0, 0),
				context: {
					estimated_tokens: 100,
					context_window: 2_000,
					reserved_output_tokens: 100,
				},
				compactions: [
					{
						kind: "turn",
						trigger: "soft",
						status: "failed",
						before_tokens: 1_000,
						duration_ms: 10,
					},
				],
			},
		};
		expect(readUsageStat(JSON.parse(JSON.stringify(payload)))).toEqual(payload);
		for (const bad of [
			null,
			{},
			{ stats: { usage: null } },
			{ stats: { usage: usage(Number.NaN) } },
			{ stats: { usage: usage(-1) } },
		]) {
			expect(readUsageStat(bad)).toBeNull();
		}
	});
});
