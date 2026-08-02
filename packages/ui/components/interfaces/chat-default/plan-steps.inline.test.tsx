import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { IPlanStep } from "./chat-db";

function step(overrides: Partial<IPlanStep> & { id: string }): IPlanStep {
	return {
		title: `Step ${overrides.id}`,
		status: "done",
		...overrides,
	} as IPlanStep;
}

async function mount(node: React.ReactNode) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		// Radix's Collapsible presence logic reaches for both.
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(node);
	});
	return { container, root };
}

describe("InlineStepGroup", () => {
	test("renders in place, so the step/text/step anchor pattern survives", async () => {
		const { InlineStepGroup } = await import("./plan-steps");
		const { container } = await mount(
			<div>
				<p>before</p>
				<InlineStepGroup steps={[step({ id: "a" })]} />
				<p>between</p>
				<InlineStepGroup steps={[step({ id: "b" })]} />
				<p>after</p>
			</div>,
		);

		const children = Array.from(container.firstElementChild?.children ?? []);
		const shape = children.map((child) =>
			child.hasAttribute("data-fl-plan-group") ? "steps" : child.tagName,
		);

		// Two groups, each sitting between the paragraphs that anchored them.
		expect(shape).toEqual(["P", "steps", "P", "steps", "P"]);
	});

	test("a settled group collapses to its own summary line", async () => {
		const { InlineStepGroup } = await import("./plan-steps");
		const { container } = await mount(
			<InlineStepGroup
				steps={[step({ id: "a" }), step({ id: "b", status: "failed" })]}
			/>,
		);

		const trigger = container.querySelector("button");
		expect(trigger?.getAttribute("data-state")).toBe("closed");
		expect(trigger?.textContent).toContain("2 steps");
		expect(trigger?.textContent).toContain("1 not completed");
	});

	test("a live group stays open so work in progress stays visible", async () => {
		const { InlineStepGroup } = await import("./plan-steps");
		const { container } = await mount(
			<InlineStepGroup
				steps={[step({ id: "a" }), step({ id: "b", status: "progress" })]}
				currentStepId="b"
				loading
			/>,
		);

		expect(container.querySelector("button")?.getAttribute("data-state")).toBe(
			"open",
		);
	});
});

describe("InlineStepGroup during a live turn", () => {
	test("a settled group stays open while the turn is still generating", async () => {
		const { InlineStepGroup } = await import("./plan-steps");
		// This group's own steps are all done and it does not own the active step,
		// so `loading` is false for it — but the turn is still running.
		const { container } = await mount(
			<InlineStepGroup
				steps={[step({ id: "a" }), step({ id: "b" })]}
				loading={false}
				turnActive
			/>,
		);

		expect(container.querySelector("button")?.getAttribute("data-state")).toBe(
			"open",
		);
	});

	test("it folds to its summary once the turn ends", async () => {
		const { InlineStepGroup } = await import("./plan-steps");
		const { container, root } = await mount(
			<InlineStepGroup
				steps={[step({ id: "a" })]}
				loading={false}
				turnActive
			/>,
		);
		expect(container.querySelector("button")?.getAttribute("data-state")).toBe(
			"open",
		);

		await act(async () => {
			root.render(
				<InlineStepGroup
					steps={[step({ id: "a" })]}
					loading={false}
					turnActive={false}
				/>,
			);
		});
		expect(container.querySelector("button")?.getAttribute("data-state")).toBe(
			"closed",
		);
	});
});
