import { afterAll, afterEach, describe, expect, spyOn, test } from "bun:test";
import { type ReactNode, createElement } from "react";
import { installStageDom } from "../../interfaces/form-workbench/stage/dom-setup";
import {
	click,
	settle,
	typeInto,
} from "../../settings/devices/testing/dom-harness";

const dom = installStageDom();
const [
	{ default: A2UICodeBlock },
	{ StreamingTextEditor },
	{ ChatFollowUpProvider },
	{ appGlobalState, pageLocalState },
	uiState,
	{ AppRouterContext },
	{ PathnameContext },
] = await Promise.all([
	import("./a2ui-code-block"),
	import("../../ui/streaming-text-editor"),
	import("../../interfaces/chat-default/chat-follow-up"),
	import("../../../lib/idb-storage"),
	import("../../../db/ui-state-db"),
	import("next/dist/shared/lib/app-router-context.shared-runtime"),
	import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
]);

const spies = [
	spyOn(appGlobalState, "getAll").mockResolvedValue({}),
	spyOn(pageLocalState, "getAll").mockResolvedValue({}),
	spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
	spyOn(uiState, "pruneElementValues").mockResolvedValue(),
];

afterEach(dom.cleanup);
afterAll(() => {
	for (const spy of spies) spy.mockRestore();
	dom.restore();
});

const lines = (...objects: unknown[]) =>
	objects.map((object) => JSON.stringify(object)).join("\n");

function inApp(node: ReactNode, send?: (text: string) => void) {
	const content = send
		? createElement(ChatFollowUpProvider, { send }, node)
		: node;
	return createElement(
		AppRouterContext.Provider,
		{ value: {} as never },
		createElement(PathnameContext.Provider, { value: "/use" }, content),
	);
}

const block = (content: string, isStreaming = false) =>
	createElement(A2UICodeBlock, { content, isStreaming });

function buttonByText(text: string): HTMLButtonElement {
	const button = [...document.querySelectorAll("button")].find(
		(candidate) => candidate.textContent?.trim() === text,
	);
	if (!button) throw new Error(`Missing button ${text}`);
	return button as HTMLButtonElement;
}

const RECIPE_LINES = [
	{ id: "root", type: "column", gap: "12px", children: ["title", "cards"] },
	{ id: "title", type: "text", content: "Recipe Remix", variant: "heading" },
	{ id: "cards", type: "row", children: ["servings", "go"] },
	{ id: "servings", type: "textField", value: "2", label: "Servings" },
	{
		id: "go",
		type: "button",
		label: "Remix this",
		action: {
			name: "send_message",
			context: { message: "Remix it for {{servings}} people" },
		},
	},
];

describe("a2ui chat fence", () => {
	test("renders progressively while the reply streams", async () => {
		const body = lines(...RECIPE_LINES);
		const markdown = (end: number) => `\`\`\`a2ui\n${body.slice(0, end)}`;
		const view = await dom.render(
			inApp(
				createElement(StreamingTextEditor, {
					content: markdown(0),
					presentation: "chat",
				}),
				() => {},
			),
		);
		await settle();
		const section = () => document.querySelector("[data-fl-a2ui-fence]");
		expect(section()?.getAttribute("aria-busy")).toBe("true");

		const firstLine = body.indexOf("\n");
		await view.rerender(
			inApp(
				createElement(StreamingTextEditor, {
					content: markdown(firstLine),
					presentation: "chat",
				}),
				() => {},
			),
		);
		await settle();
		expect(document.querySelectorAll("[data-slot=skeleton]").length).toBe(2);

		let sawTitleBeforeButton = false;
		for (let end = firstLine; end <= body.length; end += 7) {
			await view.rerender(
				inApp(
					createElement(StreamingTextEditor, {
						content: markdown(end),
						presentation: "chat",
					}),
					() => {},
				),
			);
			const text = section()?.textContent ?? "";
			if (text.includes("Recipe Remix") && !text.includes("Remix this"))
				sawTitleBeforeButton = true;
		}
		await view.rerender(
			inApp(
				createElement(StreamingTextEditor, {
					content: `${markdown(body.length)}\n\`\`\``,
					presentation: "chat",
				}),
				() => {},
			),
		);
		await settle();
		expect(sawTitleBeforeButton).toBe(true);
		expect(section()?.textContent).toContain("Remix this");
		expect(document.querySelectorAll("[data-fl-a2ui-fence]")).toHaveLength(1);
	});

	test("send_message posts the interpolated text through the chat", async () => {
		const sent: string[] = [];
		await dom.render(
			inApp(block(lines(...RECIPE_LINES)), (text) => sent.push(text)),
		);
		await settle();
		await click(buttonByText("Remix this"));
		expect(sent).toEqual(["Remix it for 2 people"]);

		const input = document.querySelector("input");
		if (!input) throw new Error("Missing servings input");
		await typeInto(input, "4");
		await settle();
		await new Promise((resolve) => setTimeout(resolve, 900));
		await click(buttonByText("Remix this"));
		expect(sent).toEqual(["Remix it for 2 people", "Remix it for 4 people"]);
	});

	test("send_message buttons are disabled outside a chat", async () => {
		await dom.render(inApp(block(lines(...RECIPE_LINES))));
		await settle();
		expect(buttonByText("Remix this").disabled).toBe(true);
	});

	test("strips disallowed components and actions", async () => {
		const sent: string[] = [];
		await dom.render(
			inApp(
				block(
					lines(
						{ id: "root", type: "column", children: ["frame", "run"] },
						{ id: "frame", type: "iframe", src: "https://example.com" },
						{
							id: "run",
							type: "button",
							label: "Run board",
							action: {
								name: "workflow_event",
								context: { nodeId: "n", appId: "a", boardId: "b" },
							},
						},
					),
				),
				(text) => sent.push(text),
			),
		);
		await settle();
		expect(document.querySelector("iframe")).toBeNull();
		await click(buttonByText("Run board"));
		await settle();
		expect(sent).toEqual([]);
	});

	test("external_link opens through a target=_blank anchor", async () => {
		const opened: string[] = [];
		const record = (event: Event) => {
			const anchor = (event.target as Element | null)?.closest?.("a");
			if (anchor?.getAttribute("target") !== "_blank") return;
			opened.push(anchor.getAttribute("href") ?? "");
			event.preventDefault();
		};
		document.addEventListener("click", record, true);
		await dom.render(
			inApp(
				block(
					lines({
						id: "root",
						type: "button",
						label: "Open docs",
						action: {
							name: "external_link",
							context: { url: "https://example.com/docs" },
						},
					}),
				),
				() => {},
			),
		);
		await settle();
		await click(buttonByText("Open docs"));
		document.removeEventListener("click", record, true);
		expect(opened).toEqual(["https://example.com/docs"]);
		expect(document.querySelectorAll("a[target=_blank]")).toHaveLength(0);
	});

	test("an a2ui fence inside the surface's markdown renders as code", async () => {
		const nested = '```a2ui\n{"id":"inner","type":"divider"}\n```';
		await dom.render(
			inApp(
				block(lines({ id: "root", type: "markdown", content: nested })),
				() => {},
			),
		);
		await settle();
		await settle();
		expect(document.querySelectorAll("[data-fl-a2ui-fence]")).toHaveLength(1);
		expect(
			document.querySelector("[data-fl-a2ui-surface] pre")?.textContent,
		).toContain('"id":"inner"');
	});

	test("shows the source when nothing renders", async () => {
		await dom.render(inApp(block("not a component")));
		await settle();
		expect(
			document.querySelector("[data-slot=a2ui-fence-error]"),
		).not.toBeNull();
		expect(
			document.querySelector("[data-slot=a2ui-fence-source]")?.textContent,
		).toBe("not a component");
	});
});
