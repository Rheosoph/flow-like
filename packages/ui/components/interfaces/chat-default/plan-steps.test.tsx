import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IPlanStep } from "./chat-db";

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll. Radix picks its layout effect when first imported,
// so the real modules load under a document.
const globalDescriptors = [
	"document",
	"HTMLElement",
	"Node",
	"navigator",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"getComputedStyle",
	"window",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	nextThemes: { ...(await import("next-themes")) },
	reasoningViewer: { ...(await import("./reasoning-viewer")) },
	lib: { ...(await import("../../../lib")) },
	date: { ...(await import("../../../lib/date")) },
	collapsible: { ...(await import("../../ui/collapsible")) },
};

mock.module("next-themes", () => ({
	...actual.nextThemes,
	useTheme: () => ({ resolvedTheme: "dark" }),
}));
mock.module("./reasoning-viewer", () => ({
	...actual.reasoningViewer,
	ReasoningViewer: () => null,
}));
// Stub the two helpers the component actually uses so these tests stay independent of the
// `lib` barrel's formatting.
mock.module("../../../lib", () => ({
	...actual.lib,
	cn: (...classes: unknown[]) => classes.filter(Boolean).join(" "),
}));
mock.module("../../../lib/date", () => ({
	...actual.date,
	formatDuration: (ms: number) => `${Math.round(ms / 1000)}s`,
}));
// Radix's Collapsible reads computed styles that happy-dom cannot produce. Swap it for a shell that
// exposes the open state as an attribute, so these tests assert this component's decisions rather
// than an animation library's internals.
mock.module("../../ui/collapsible", () => ({
	...actual.collapsible,
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

afterAll(() => {
	mock.restore();
	mock.module("next-themes", () => actual.nextThemes);
	mock.module("./reasoning-viewer", () => actual.reasoningViewer);
	mock.module("../../../lib", () => actual.lib);
	mock.module("../../../lib/date", () => actual.date);
	mock.module("../../ui/collapsible", () => actual.collapsible);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

async function renderSteps(
	steps: IPlanStep[],
	interact?: (container: HTMLElement) => void,
) {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
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
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(<InlineStepGroup steps={steps} />);
	});
	if (interact) await act(async () => interact(container));
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

describe("tool activity context", () => {
	const webStep: IPlanStep = {
		id: "web",
		title: "Using http_request",
		toolName: "http_request",
		status: "done",
		detail: {
			kind: "tool_activity",
			category: "web",
			label: "Fetching page",
			method: "GET",
			outcome: "HTTP 200",
			sources: [
				{
					url: "https://github.com/flow-like/flow-like/blob/main/LICENSE",
					hostname: "github.com",
					path: "/flow-like/flow-like/blob/main/LICENSE",
				},
			],
		},
	};

	test("shows the website, resource and outcome with a source link outside buttons", async () => {
		const { text, html } = await renderSteps([webStep], (container) => {
			const link = container.querySelector("a");
			expect(link?.getAttribute("href")).toBe(
				"https://github.com/flow-like/flow-like/blob/main/LICENSE",
			);
			expect(link?.getAttribute("rel")).toBe("noopener noreferrer");
			expect(link?.closest("button")).toBeNull();
			expect(link?.querySelector("img")?.getAttribute("src")).toBe(
				"https://www.google.com/s2/favicons?domain=github.com&sz=32",
			);
			expect(link?.querySelector("img")?.getAttribute("referrerpolicy")).toBe(
				"no-referrer",
			);
		});
		expect(text).toContain("Fetching page");
		expect(text).toContain("github.com");
		expect(text).toContain("/flow-like/flow-like/blob/main/LICENSE");
		expect(text).toContain("GET");
		expect(text).toContain("HTTP 200");
		expect(text).not.toContain("Using http_request");
		expect(html).toContain('aria-label="Completed"');
	});

	test("falls back to a globe when a favicon fails", async () => {
		const { html } = await renderSteps([webStep], (container) => {
			expect(container.querySelector("a img")).not.toBeNull();
			for (const icon of container.querySelectorAll("img")) {
				icon.dispatchEvent(new window.Event("error"));
			}
		});
		expect(html).not.toContain("<img");
		expect(html).toContain("lucide-globe");
	});

	test("keeps private hostnames out of the favicon service", async () => {
		const { html, text } = await renderSteps([
			{
				...webStep,
				detail: {
					kind: "tool_activity",
					category: "web",
					label: "Fetching page",
					sources: [
						{
							url: "http://dashboard.internal/report",
							hostname: "dashboard.internal",
							path: "/report",
						},
					],
				},
			},
		]);
		expect(text).toContain("dashboard.internal");
		expect(html).not.toContain("google.com");
	});

	test("keeps authored step titles and avoids links to redacted destinations", async () => {
		const { text, html } = await renderSteps([
			{
				...webStep,
				title: "Check the archived license",
				detail: {
					kind: "tool_activity",
					category: "web",
					label: "Fetching URL",
					sources: [
						{
							url: "https://en.wikipedia.org/w/index.php",
							hostname: "en.wikipedia.org",
							path: "/w/index.php",
							linkable: false,
						},
					],
				},
			},
		]);
		expect(text).toContain("Check the archived license");
		expect(text).toContain("en.wikipedia.org");
		expect(text).not.toContain("Fetching URL");
		expect(html).not.toContain("href=");
	});

	test("shows search queries and file targets as context", async () => {
		const { text } = await renderSteps([
			{
				id: "search",
				title: "Using web_search",
				status: "progress",
				detail: {
					kind: "tool_activity",
					category: "search",
					label: "Searching the web",
					query: "Flow-Like license terms",
				},
			},
			{
				id: "file",
				title: "Using read_file",
				status: "done",
				detail: {
					kind: "tool_activity",
					category: "files",
					label: "Reading file",
					target: "reports/revenue.csv",
				},
			},
		]);
		expect(text).toContain("Flow-Like license terms");
		expect(text).toContain("reports/revenue.csv");
	});

	test("the outcome replaces the progress note so the meta stays one line", async () => {
		const { text } = await renderSteps([
			{
				id: "apps",
				title: "Using list_apps",
				toolName: "list_apps",
				description: "Looking through installed apps",
				status: "done",
				detail: {
					kind: "tool_activity",
					category: "apps",
					label: "Checking available apps",
					query: "weather",
					outcome: "0 apps found",
				},
			},
			{
				id: "apps-running",
				title: "Using list_apps",
				toolName: "list_apps",
				description: "Looking through installed apps",
				status: "progress",
				detail: {
					kind: "tool_activity",
					category: "apps",
					label: "Checking available apps",
				},
			},
		]);
		expect(text).toContain("weather");
		expect(text).toContain("0 apps found");
		expect(text.match(/Looking through installed apps/g)).toHaveLength(1);
	});
});
