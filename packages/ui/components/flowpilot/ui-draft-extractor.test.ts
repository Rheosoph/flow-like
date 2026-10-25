import { describe, expect, test } from "bun:test";
import {
	createCopilotStreamParser,
	readUiDraftFrame,
} from "./copilot-stream-parser";
import {
	MAX_UI_DRAFT_COMPONENTS,
	type UiDraftScan,
	type UiDraftSnapshot,
	createThrottledUiDraft,
	createUiDraftCollector,
	createUiDraftTextExtractor,
} from "./ui-draft-extractor";

const components = [
	{
		id: "root",
		style: { className: "flex {gap-2} [&>*]:p-1" },
		component: {
			type: "column",
			children: { explicitList: ["title", "body"] },
		},
	},
	{
		id: "title",
		type: "text",
		content: 'Quote " brace } bracket ] backslash \\ end\\',
		variant: "h3",
	},
	{
		id: "body",
		type: "markdown",
		content: 'Line\n{"nested": [1, {"deep": true}]} ```json </ui_draft> 🚀',
		rows: [[1, 2], { k: [3, { x: "}" }] }],
	},
];
const canvasSettings = {
	backgroundColor: "bg-background",
	customCss: ".a { color: red; } /* ] */",
};

const surfaceJson = [
	"{",
	`  "canvasSettings" : ${JSON.stringify(canvasSettings)},`,
	'  "components": [',
	`    ${components.map((component) => JSON.stringify(component)).join(" ,\n    ")}`,
	"  ],",
	'  "rootComponentId": "root"',
	"}",
].join("\n");

const fencedReply = [
	'I\'ll lay it out with {braces} and an inline {"type": "text"} example.',
	"",
	"```ts",
	'const example = { "components": [{ "id": "not-a-draft" }] };',
	"```",
	"",
	"Here is the UI:",
	"```json",
	surfaceJson,
	"```",
	'Afterwards {"components": [{"id": "ignored"}]} is not part of the draft.',
].join("\n");

function extractInChunks(text: string, splitPoints: number[]) {
	const extractor = createUiDraftTextExtractor();
	const merged: UiDraftScan = { components: [] };
	let previous = 0;
	for (const point of [...splitPoints, text.length]) {
		const scan = extractor.push(text.slice(previous, point));
		previous = point;
		merged.components.push(...scan.components);
		if (scan.rootComponentId !== undefined) {
			expect(merged.rootComponentId).toBeUndefined();
			merged.rootComponentId = scan.rootComponentId;
		}
		if (scan.canvasSettings) {
			expect(merged.canvasSettings).toBeUndefined();
			merged.canvasSettings = scan.canvasSettings;
		}
	}
	return merged;
}

function expectCompleteDraft(scan: UiDraftScan) {
	expect(scan.components).toEqual(components);
	expect(scan.rootComponentId).toBe("root");
	expect(scan.canvasSettings).toEqual(canvasSettings);
}

describe("ui draft text extractor", () => {
	test("extracts every component exactly once for any chunk boundary", () => {
		for (let point = 1; point < fencedReply.length; point += 1) {
			expectCompleteDraft(extractInChunks(fencedReply, [point]));
		}
		for (let size = 1; size <= 13; size += 1) {
			const points: number[] = [];
			for (let point = size; point < fencedReply.length; point += size) {
				points.push(point);
			}
			expectCompleteDraft(extractInChunks(fencedReply, points));
		}
	});

	test("emits components progressively as each object closes", () => {
		const extractor = createUiDraftTextExtractor();
		const firstEnd =
			fencedReply.indexOf(JSON.stringify(components[0])) +
			JSON.stringify(components[0]).length;
		const first = extractor.push(fencedReply.slice(0, firstEnd - 1));
		expect(first.components).toEqual([]);
		expect(first.canvasSettings).toEqual(canvasSettings);
		const second = extractor.push(fencedReply.slice(firstEnd - 1, firstEnd));
		expect(second.components).toEqual([components[0]]);
	});

	test("reads a bare JSON object without a fence", () => {
		const scan = extractInChunks(`Sure!\n${surfaceJson}\nThat's it.`, [9, 40]);
		expectCompleteDraft(scan);
	});

	test("reads a fenced bare component array", () => {
		const reply = `\`\`\`json\n${JSON.stringify(components, null, 2)}\n\`\`\``;
		const scan = extractInChunks(reply, [5, 12, 200]);
		expect(scan.components).toEqual(components);
		expect(scan.rootComponentId).toBeUndefined();
	});

	test("keeps looking past JSON blocks that hold no authored components", () => {
		const reply = [
			"```json",
			'[{"command_type": "AddNode", "node": {"name": "x"}}, {"id": "loose"}]',
			"```",
			"```json",
			surfaceJson,
			"```",
		].join("\n");
		expectCompleteDraft(extractInChunks(reply, [40, 90]));
	});

	test("ignores text that never contains a component tree", () => {
		const scan = extractInChunks(
			'Use `{ "a": 1 }` or {"b": [1, 2]} and ```\nplain block\n```.',
			[3, 17, 30],
		);
		expect(scan.components).toEqual([]);
	});

	test("stops after the component cap", () => {
		const many = Array.from(
			{ length: MAX_UI_DRAFT_COMPONENTS + 7 },
			(_, index) => ({ id: `c${index}`, type: "text", content: `${index}` }),
		);
		const scan = extractInChunks(
			`\`\`\`json\n${JSON.stringify({ components: many })}\n\`\`\``,
			[100, 2000],
		);
		expect(scan.components).toHaveLength(MAX_UI_DRAFT_COMPONENTS);
		expect(scan.components.at(-1)).toEqual(many[MAX_UI_DRAFT_COMPONENTS - 1]);
	});
});

describe("ui draft frames", () => {
	test("parse as typed ui_draft stream events", () => {
		const parser = createCopilotStreamParser();
		const frame =
			'<ui_draft>{"tool_call_id":"toolu_1","sequence":2,"components":[{"id":"a"}],"root_component_id":"a","canvas_settings":{"padding":"1rem"}}</ui_draft>';
		const events = [
			...parser.push(`before ${frame.slice(0, 20)}`),
			...parser.push(`${frame.slice(20)} after`),
		];
		expect(events.map((event) => event.type)).toEqual([
			"text",
			"ui_draft",
			"text",
		]);
		expect(readUiDraftFrame(events[1].data)).toEqual({
			toolCallId: "toolu_1",
			sequence: 2,
			components: [{ id: "a" }],
			rootComponentId: "a",
			canvasSettings: { padding: "1rem" },
		});
	});

	test("reject malformed payloads", () => {
		expect(readUiDraftFrame(null)).toBeNull();
		expect(readUiDraftFrame([])).toBeNull();
		expect(readUiDraftFrame({ components: [] })).toBeNull();
		expect(readUiDraftFrame({ tool_call_id: "t", components: {} })).toBeNull();
		expect(
			readUiDraftFrame({
				tool_call_id: "t",
				components: [],
				canvas_settings: [],
			}),
		).toEqual({ toolCallId: "t", sequence: 0, components: [] });
	});
});

describe("ui draft collector", () => {
	const frame = (toolCallId: string, ids: string[], sequence = 1) => ({
		toolCallId,
		sequence,
		components: ids.map((id) => ({ id, type: "text", content: id })),
	});

	test("accumulates per tool call and shows the most recent source", () => {
		const collector = createUiDraftCollector();
		expect(collector.snapshot()).toBeNull();
		expect(collector.pushFrame(frame("first", ["a", "b"]))).toBe(true);
		expect(collector.pushFrame(frame("retry", ["x"]))).toBe(true);
		expect(collector.snapshot()?.components.map(idOf)).toEqual(["x"]);
		expect(collector.pushFrame(frame("first", ["c"], 2))).toBe(true);
		expect(collector.snapshot()?.components.map(idOf)).toEqual(["a", "b", "c"]);
		expect(
			collector.pushFrame({
				toolCallId: "first",
				sequence: 3,
				components: [null, 7],
			}),
		).toBe(false);
	});

	test("merges root and canvas settings into the draft of their source", () => {
		const collector = createUiDraftCollector();
		collector.pushFrame({
			toolCallId: "t",
			sequence: 1,
			components: [],
			rootComponentId: "root",
		});
		collector.pushFrame({
			...frame("t", ["root"], 2),
			canvasSettings: { padding: "1rem" },
		});
		expect(collector.snapshot()).toEqual({
			components: [{ id: "root", type: "text", content: "root" }],
			rootComponentId: "root",
			canvasSettings: { padding: "1rem" },
		});
	});

	test("text drafts compete with frame drafts by recency", () => {
		const collector = createUiDraftCollector();
		collector.pushFrame(frame("t", ["a"]));
		expect(collector.pushText("Just prose so far.")).toBe(false);
		expect(collector.pushText(fencedReply)).toBe(true);
		expect(collector.snapshot()?.components).toEqual(components);
	});
});

describe("throttled ui draft", () => {
	test("publishes the latest snapshot once per interval and nothing after cancel", async () => {
		const published: UiDraftSnapshot[] = [];
		const draft = createThrottledUiDraft(
			(snapshot) => published.push(structuredClone(snapshot)),
			5,
		);
		draft.pushFrame({
			toolCallId: "t",
			sequence: 1,
			components: [{ id: "a" }],
		});
		draft.pushFrame({
			toolCallId: "t",
			sequence: 2,
			components: [{ id: "b" }],
		});
		await new Promise((resolve) => setTimeout(resolve, 25));
		expect(published).toHaveLength(1);
		expect(published[0].components).toEqual([{ id: "a" }, { id: "b" }]);

		draft.pushFrame({
			toolCallId: "t",
			sequence: 3,
			components: [{ id: "c" }],
		});
		draft.cancel();
		draft.pushText(fencedReply);
		await new Promise((resolve) => setTimeout(resolve, 25));
		expect(published).toHaveLength(1);
	});
});

function idOf(component: unknown) {
	return (component as { id: string }).id;
}
