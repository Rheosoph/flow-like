import { expect, test, vi } from "vitest";
import type { SurfaceComponent } from "../../components/a2ui/types";
import type { IBackendState } from "../../state/backend-state";
import type { IWidget } from "../../state/backend-state/widget-state";
import { compileAppSpec } from "./compiler";
import {
	appResourceInstruction,
	materializeAppBuildResource,
} from "./resource-materializer";
import { exampleAppSpec } from "./test-fixtures";

test("repair briefs retain the full contract and exact symbols alongside host diagnostics", () => {
	const plan = compileAppSpec(exampleAppSpec(), {
		app_id: "app",
		build_id: "build",
	});
	const resource = plan.resources.find((item) => item.kind === "board");
	if (!resource) throw new Error("The example spec has no board resource.");
	const instruction = appResourceInstruction(plan, resource, [
		"Missing field title in persisted insert payload.",
	]);
	expect(instruction).toContain("Validate the payload and persist the issue.");
	expect(instruction).toContain(resource.physical_id);
	expect(instruction).toContain("capture_issue");
	expect(instruction).toContain(
		"Missing field title in persisted insert payload.",
	);
	expect(instruction).toContain(
		"do not authorize extra actions or reduced scope",
	);
});

test("a generated widget declares every event its components fire", async () => {
	const plan = compileAppSpec(
		{
			schema_version: 1,
			name: "Fixture",
			requirements: [{ id: "ship", description: "Ship the fixture." }],
			resources: [
				{
					key: "card",
					kind: "widget",
					depends_on: [],
					requirement_ids: ["ship"],
					config: { name: "Card", instruction: "Build the widget." },
				},
			],
			scenarios: [],
		},
		{ app_id: "app_fixture", build_id: "build_fixture" },
	);
	const resource = plan.resources[0];
	const components: SurfaceComponent[] = [
		{
			id: "root",
			component: {
				id: "root",
				type: "button",
				label: { literalString: "Approve" },
				eventHandlers: {
					click: [{ name: "widget_event", context: { actionId: "approve" } }],
				},
			},
		},
	];
	const updateWidget = vi.fn(async (_appId: string, _widget: IWidget) => {});
	await materializeAppBuildResource(
		{ widgetState: { updateWidget } } as unknown as IBackendState,
		plan,
		resource,
		{
			assertActive: () => {},
			delegate: async () => ({}),
			generateWidget: async () => components,
		} as never,
	);
	expect(updateWidget).toHaveBeenCalledTimes(1);
	expect(updateWidget.mock.calls[0][1].actions).toEqual([
		{ id: "approve", label: "approve", contextSchema: [] },
	]);
});
