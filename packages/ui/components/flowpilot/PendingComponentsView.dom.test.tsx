import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import { createElement } from "react";
import type { SurfaceComponent } from "../a2ui/types";
import { installStageDom } from "../interfaces/form-workbench/stage/dom-setup";

// The first render imports the A2UI component graph, several seconds on a cold cache.
setDefaultTimeout(60_000);

const dom = installStageDom();
const { PendingComponentsView } = await import("./PendingComponentsView");

afterEach(dom.cleanup);
afterAll(dom.restore);

const components = [
	{
		id: "root",
		component: { type: "column", children: { explicitList: ["title"] } },
	},
	{
		id: "title",
		component: { type: "text", content: { literalString: "Hello draft" } },
	},
] as unknown as SurfaceComponent[];

const hasApplyButton = () =>
	[...document.querySelectorAll("button")].some(
		(button) => button.textContent?.trim() === "Apply",
	);

describe("pending components view", () => {
	test("a live draft previews the tree with progress instead of apply controls", async () => {
		const props = { components, onApply: () => {}, onDismiss: () => {} };
		const view = await dom.render(
			createElement(PendingComponentsView, { ...props, drafting: true }),
		);
		expect(document.body.textContent).toContain("Drafting");
		expect(document.body.textContent).toContain("Hello draft");
		expect(document.body.textContent).not.toContain("Ready to Apply");
		expect(hasApplyButton()).toBe(false);

		await view.rerender(createElement(PendingComponentsView, props));
		expect(document.body.textContent).toContain("Ready to Apply");
		expect(hasApplyButton()).toBe(true);
	});
});
