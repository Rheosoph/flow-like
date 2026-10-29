import {
	type ThinkingLabels,
	agentThinkingOptions,
	chatThinkingLevel,
	chatThinkingOptions,
	isValidReasoningEffort,
	thinkingLevelForEffort,
	thinkingStampLabel,
	turnThinking,
} from "@flow-like/flow-like-ui/components/thinking/thinking-options";
import { describe, expect, test } from "vitest";

const labels: ThinkingLabels = {
	off: "Off",
	low: "Low",
	mid: "Mid",
	high: "High",
	extraHigh: "Extra high",
	auto: "Auto",
	autoDescription: "Provider default",
	autoWithDefault: (level) => `Provider default (${level})`,
	describe: (level) => `level ${level}`,
};

describe("thinking picker options", () => {
	test("uses the active model's dynamically advertised effort labels", () => {
		const options = agentThinkingOptions(
			{
				defaultReasoningEffort: "high",
				supportedReasoningEfforts: [
					{ id: "low", name: "Low" },
					{ id: "high", name: "High", description: "Thinks longer" },
				],
			},
			labels,
		);

		expect(options.map((option) => option.id)).toEqual(["", "low", "high"]);
		expect(options[0]).toMatchObject({
			auto: true,
			description: "Provider default (High)",
			level: 3,
		});
		expect(options[1]?.description).toBe("level 1");
		expect(options[2]?.description).toBe("Thinks longer");
	});

	test("hides the picker for agent models without configurable effort", () => {
		expect(
			agentThinkingOptions({ supportedReasoningEfforts: [] }, labels),
		).toEqual([]);
		expect(agentThinkingOptions(undefined, labels)).toEqual([]);
	});

	test("offers the History thinking levels for chats and Bits models", () => {
		expect(chatThinkingOptions(labels).map((option) => option.id)).toEqual([
			"off",
			"low",
			"mid",
			"high",
		]);
		expect(chatThinkingLevel(undefined)).toBe("low");
		expect(chatThinkingLevel("xhigh")).toBe("low");
		expect(chatThinkingLevel("high")).toBe("high");
	});

	test("maps provider effort ids onto the glyph", () => {
		expect(thinkingLevelForEffort("minimal")).toBe(0);
		expect(thinkingLevelForEffort("medium")).toBe(2);
		expect(thinkingLevelForEffort("xhigh")).toBe(4);
		expect(thinkingLevelForEffort("something-new")).toBe(2);
	});

	test("stamps a turn with the level it ran at, never with the provider's Auto", () => {
		expect(turnThinking(false, "")).toBe("low");
		expect(turnThinking(false, "high")).toBe("high");
		expect(turnThinking(true, "")).toBeUndefined();
		expect(turnThinking(true, "xhigh")).toBe("xhigh");
	});

	test("names stamped levels for the transcript", () => {
		expect(thinkingStampLabel("mid", labels)).toBe("Mid");
		expect(thinkingStampLabel("xhigh", labels)).toBe("Extra high");
		expect(thinkingStampLabel("medium", labels)).toBe("Medium");
		expect(thinkingStampLabel("max_budget", labels)).toBe("Max budget");
	});

	test("keeps a stored effort only while the provider and model accept it", () => {
		const model = {
			supportedReasoningEfforts: [{ id: "xhigh", name: "Extra high" }],
		};
		expect(isValidReasoningEffort(false, "mid", undefined)).toBe(true);
		expect(isValidReasoningEffort(false, "xhigh", undefined)).toBe(false);
		expect(isValidReasoningEffort(true, "xhigh", model)).toBe(true);
		expect(isValidReasoningEffort(true, "low", model)).toBe(false);
		expect(isValidReasoningEffort(true, "", model)).toBe(true);
	});
});
