import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { expect, test } from "vitest";
import { parseWorkflowScreenshotArgs } from "../cli";
import {
	buildWorkflowScreenshotPlan,
	resolveWorkflowScreenshotFocus,
} from "../runner";

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

test("focuses the adjusted node unless an explicit focus overrides it", () => {
	const board = {
		nodes: {
			adjusted: {
				id: "adjusted",
				name: "api_call",
				friendly_name: "API Call",
				pins: {},
			},
			other: {
				id: "other",
				name: "log",
				friendly_name: "Write Log",
				pins: {},
			},
		},
		layers: {},
		comments: {},
	} as unknown as IBoard;
	const adjustedTarget = {
		id: "adjusted",
		kind: "node" as const,
		label: "API Call",
		matchedBy: "name" as const,
	};

	expect(
		resolveWorkflowScreenshotFocus(board, undefined, adjustedTarget),
	).toEqual(adjustedTarget);
	expect(
		resolveWorkflowScreenshotFocus(board, "Write Log", adjustedTarget),
	).toMatchObject({ id: "other", kind: "node" });
});
