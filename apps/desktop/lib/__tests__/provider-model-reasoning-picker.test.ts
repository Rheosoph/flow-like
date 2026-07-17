import { reasoningEffortLabels } from "@flow-like/flow-like-ui/components/flowpilot/provider-model-reasoning-picker";
import { describe, expect, test } from "vitest";

describe("provider/model/reasoning picker", () => {
	test("uses the active model's dynamically advertised effort labels", () => {
		const model = {
			id: "sonnet",
			label: "Sonnet",
			defaultReasoningEffort: "high",
			supportedReasoningEfforts: [
				{ id: "low", name: "Low" },
				{ id: "high", name: "High" },
			],
		};

		expect(reasoningEffortLabels(model, "")).toMatchObject({
			automatic: "Auto (High default)",
			selected: "Auto (High default)",
		});
		expect(reasoningEffortLabels(model, "low").selected).toBe("Low");
	});
});
