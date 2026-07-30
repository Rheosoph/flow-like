import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IPlanStep } from "./chat-db";

// `mock.module` is process-global in bun, so a sibling test file's partial `next-themes` stub can
// break this file's import graph depending on run order. Declare the full surface we might touch.
mock.module("next-themes", () => ({
	useTheme: () => ({ resolvedTheme: "dark" }),
	ThemeProvider: ({ children }: { children?: React.ReactNode }) => children,
}));
mock.module("./reasoning-viewer", () => ({ ReasoningViewer: () => null }));
// The `lib` barrel imports back into components, so pulling it in from a direct unit import of
// plan-steps re-enters this module and its module-level consts hit the temporal dead zone. Stub the
// two helpers the component actually uses instead of dragging the barrel in.
mock.module("../../../lib", () => ({
	cn: (...classes: unknown[]) => classes.filter(Boolean).join(" "),
}));
mock.module("../../../lib/date", () => ({
	formatDuration: (ms: number) => `${Math.round(ms / 1000)}s`,
}));
// Radix's Collapsible reads computed styles that happy-dom cannot produce. Swap it for a shell that
// exposes the open state as an attribute, so these tests assert this component's decisions rather
// than an animation library's internals.
mock.module("../../ui/collapsible", () => ({
	Collapsible: ({
		open,
		children,
	}: {
		open?: boolean;
		children?: React.ReactNode;
	}) => <div data-open={String(Boolean(open))}>{children}</div>,
	CollapsibleTrigger: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
	CollapsibleContent: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
}));

afterAll(() => mock.restore());

async function renderSteps(steps: IPlanStep[]) {
	const window = new Window();
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		// Radix's Collapsible (used once a group is large enough to fall back to the timeline)
		// reads computed styles on ref attach.
		getComputedStyle: window.getComputedStyle.bind(window),
		window,
	});
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

	const { InlineStepGroup } = await import("./plan-steps");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(<InlineStepGroup steps={steps} />);
	});
	const text = container.textContent ?? "";
	const html = container.innerHTML;
	await act(async () => root.unmount());
	window.close();
	return { text, html };
}

describe("build lane steps", () => {
	test("renders every segment and its applied state instead of one truncated line", async () => {
		const { text } = await renderSteps([
			{
				id: "build-lane",
				title: "Workflow",
				status: "progress",
				detail: {
					kind: "build_lane",
					lane: "workflow",
					target: "Support Desk",
					segmentsApplied: 2,
					segmentsTotal: 4,
					earnedMinutes: 12,
					segments: [
						{ id: "s1", title: "Ingest mail", applied: true },
						{ id: "s2", title: "Classify ticket", applied: true },
						{ id: "s3", title: "Route to agent", applied: false },
						{ id: "s4", title: "Notify requester", applied: false },
					],
				},
			},
		]);

		expect(text).toContain("Workflow");
		expect(text).toContain("Support Desk");
		expect(text).toContain("2/4");
		expect(text).toContain("+12m");
		// Every segment is listed, not joined into one truncated description.
		for (const title of [
			"Ingest mail",
			"Classify ticket",
			"Route to agent",
			"Notify requester",
		]) {
			expect(text).toContain(title);
		}
	});

	test("surfaces the functions the build handed back for the user to implement", async () => {
		const { text } = await renderSteps([
			{
				id: "build-lane",
				title: "Workflow",
				status: "done",
				detail: {
					kind: "build_lane",
					lane: "workflow",
					gaps: [
						{
							function: "syncToJira",
							detail: "the catalog has no Jira node",
						},
					],
				},
			},
		]);

		expect(text).toContain("1 function needs your logic");
		expect(text).toContain("syncToJira");
		expect(text).toContain("the catalog has no Jira node");
	});

	test("concurrent lanes each render their own block", async () => {
		const { text } = await renderSteps([
			{
				id: "sub:a:build-lane",
				title: "Data",
				status: "done",
				detail: { kind: "build_lane", lane: "data", target: "tickets" },
			},
			{
				id: "sub:b:build-lane",
				title: "Page",
				status: "progress",
				detail: { kind: "build_lane", lane: "page", target: "/dashboard" },
			},
			{
				id: "sub:c:build-lane",
				title: "Workflow",
				status: "progress",
				detail: { kind: "build_lane", lane: "workflow" },
			},
		]);

		expect(text).toContain("Data");
		expect(text).toContain("tickets");
		expect(text).toContain("Page");
		expect(text).toContain("/dashboard");
		expect(text).toContain("Workflow");
	});

	test("lanes stay visible past the recent-steps window", async () => {
		// A real build emits far more rows than the window shows. The lanes are published when each
		// lane STARTS, so a plain "last N steps" cut would hide them exactly when they matter.
		const noise: IPlanStep[] = Array.from({ length: 12 }, (_, index) => ({
			id: `noise-${index}`,
			title: `Using some_tool_${index}`,
			status: "done" as const,
		}));
		const { text } = await renderSteps([
			{
				id: "sub:a:build-lane",
				title: "Workflow",
				status: "progress",
				detail: {
					kind: "build_lane",
					lane: "workflow",
					target: "Support Desk",
				},
			},
			...noise,
		]);

		expect(text).toContain("Workflow");
		expect(text).toContain("Support Desk");
	});

	test("a settled build with gaps does not fold itself away", async () => {
		const noise: IPlanStep[] = Array.from({ length: 12 }, (_, index) => ({
			id: `noise-${index}`,
			title: `Using some_tool_${index}`,
			status: "done" as const,
		}));
		const { text, html } = await renderSteps([
			{
				id: "sub:a:build-lane",
				title: "Workflow",
				status: "done",
				detail: {
					kind: "build_lane",
					lane: "workflow",
					gaps: [{ function: "syncToJira", detail: "no Jira node" }],
				},
			},
			...noise,
		]);

		// Everything settled cleanly, so this panel would normally auto-collapse.
		expect(html).toContain('data-open="true"');
		expect(text).toContain("1 function needs your logic");
		expect(text).toContain("syncToJira");
	});

	test("a settled build with no gaps still collapses", async () => {
		const noise: IPlanStep[] = Array.from({ length: 12 }, (_, index) => ({
			id: `noise-${index}`,
			title: `Using some_tool_${index}`,
			status: "done" as const,
		}));
		const { html } = await renderSteps([
			{
				id: "sub:a:build-lane",
				title: "Workflow",
				status: "done",
				detail: { kind: "build_lane", lane: "workflow" },
			},
			...noise,
		]);

		expect(html).toContain('data-open="false"');
	});

	test("failed tool attempts are subdued and do not hold a settled panel open", async () => {
		const steps: IPlanStep[] = [
			{
				id: "failed-tool",
				title: "Using ui_inspect",
				status: "failed",
			},
			...Array.from({ length: 5 }, (_, index) => ({
				id: `done-${index}`,
				title: `Using tool_${index}`,
				status: "done" as const,
			})),
		];
		const { text, html } = await renderSteps(steps);

		expect(html).toContain('data-open="false"');
		expect(html).not.toContain("text-red-500");
		expect(text).toContain("1 not completed");
		expect(text).not.toContain("1 failed");
	});

	test("a plain step still renders as a row", async () => {
		const { text } = await renderSteps([
			{
				id: "plain",
				title: "Using flowpilot_board",
				description: "Running...",
				status: "progress",
			},
		]);

		expect(text).toContain("Using flowpilot_board");
		expect(text).toContain("Running...");
	});
});
