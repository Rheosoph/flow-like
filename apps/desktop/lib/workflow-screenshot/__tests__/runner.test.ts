import { expect, test } from "vitest";
import { parseWorkflowScreenshotArgs } from "../cli";
import { buildWorkflowScreenshotPlan } from "../runner";

test("keeps the function id in the route while waiting for its rendered boundary", () => {
	const options = parseWorkflowScreenshotArgs([
		"example.flow",
		"--output=example.webp",
	]);
	if (!options.output) throw new Error("The output fixture was not parsed.");
	const plan = buildWorkflowScreenshotPlan(
		options,
		options.output,
		"function-layer",
		"function-layer-input",
	);
	const [scenario] = plan.scenarios;
	if (!scenario) throw new Error("The screenshot scenario was not built.");

	expect(scenario.query?.node).toBe("function-layer");
	expect(scenario.steps).toContainEqual(
		expect.objectContaining({
			type: "waitFor",
			selector: '.react-flow__node[data-id="function-layer-input"]',
		}),
	);
});
