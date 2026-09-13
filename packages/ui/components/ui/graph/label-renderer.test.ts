import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Settings } from "sigma/settings";
import {
	GRAPH_LABEL_LEFT_INSET,
	GRAPH_LABEL_RIGHT_INSET,
} from "./graph-layout";
import {
	drawNodeHover,
	drawNodeLabel,
	resetNodeLabelLayout,
} from "./label-renderer";
import { invalidateGraphTheme } from "./theme-colors";

const settings = {
	labelSize: 12,
	labelFont: "sans-serif",
	labelWeight: "500",
	labelRenderedSizeThreshold: 0,
} as Settings;

const baseNode = {
	x: 40,
	y: 80,
	size: 6,
	label: "Short",
	color: "#22aaff",
};

let window: Window;
let previousDocument: PropertyDescriptor | undefined;
let previousGetComputedStyle: PropertyDescriptor | undefined;

beforeEach(() => {
	previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
	previousGetComputedStyle = Object.getOwnPropertyDescriptor(
		globalThis,
		"getComputedStyle",
	);
	window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	window.document.documentElement.style.setProperty(
		"--background",
		"rgb(0,0,0)",
	);
	window.document.documentElement.style.setProperty(
		"--foreground",
		"rgb(255,255,255)",
	);
	Object.defineProperties(globalThis, {
		document: { configurable: true, value: window.document },
		getComputedStyle: {
			configurable: true,
			value: window.getComputedStyle.bind(window),
		},
	});
	invalidateGraphTheme();
});

afterEach(async () => {
	for (const [key, descriptor] of [
		["document", previousDocument],
		["getComputedStyle", previousGetComputedStyle],
	] as const) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	invalidateGraphTheme();
	await window.happyDOM.close();
});

function recordingCanvas(width = 600, height = 200) {
	const text: { label: string; x: number; y: number; width: number }[] = [];
	const pills: {
		left: number;
		right: number;
		top: number;
		bottom: number;
		fill: string;
	}[] = [];
	let points: { x: number; y: number }[] = [];
	const context = {
		canvas: { clientWidth: width, clientHeight: height, width, height },
		font: "",
		fillStyle: "",
		strokeStyle: "",
		textAlign: "left",
		measureText: (label: string) => ({ width: label.length * 7 }),
		fillText(label: string, x: number, y: number) {
			const textWidth = label.length * 7;
			text.push({
				label,
				x: context.textAlign === "right" ? x - textWidth : x,
				y,
				width: textWidth,
			});
		},
		strokeText() {},
		beginPath() {
			points = [];
		},
		moveTo(x: number, y: number) {
			points.push({ x, y });
		},
		lineTo(x: number, y: number) {
			points.push({ x, y });
		},
		arcTo(x: number, y: number, nextX: number, nextY: number) {
			points.push({ x, y }, { x: nextX, y: nextY });
		},
		arc() {},
		closePath() {},
		stroke() {},
		fill() {
			if (!points.length) return;
			pills.push({
				left: Math.min(...points.map(({ x }) => x)),
				right: Math.max(...points.map(({ x }) => x)),
				top: Math.min(...points.map(({ y }) => y)),
				bottom: Math.max(...points.map(({ y }) => y)),
				fill: String(context.fillStyle),
			});
		},
	} as unknown as CanvasRenderingContext2D;
	return { context, text, pills };
}

describe("graph caption collisions", () => {
	test("resolves foreground node blockers after reset and keeps the caption clear of its own node", () => {
		const { context, text } = recordingCanvas();
		const neighbor = { x: 300, y: baseNode.y, size: 6 };
		let reads = 0;
		resetNodeLabelLayout(context, () => {
			reads += 1;
			return [baseNode, neighbor];
		});
		expect(reads).toBe(0);

		// The camera can move a node between beforeRender and label painting.
		neighbor.x = 90;
		drawNodeLabel(context, baseNode, settings);
		expect(text).toHaveLength(0);
		expect(reads).toBe(1);
		drawNodeLabel(context, { ...baseNode, x: 190, label: "Clear" }, settings);
		expect(text.map(({ label }) => label)).toEqual(["Clear"]);
		expect(reads).toBe(1);

		resetNodeLabelLayout(context, () => [baseNode]);
		drawNodeLabel(context, baseNode, settings);
		expect(text.map(({ label }) => label)).toEqual(["Clear", "Short"]);
	});

	test("culls overlapping forced captions including the population badge", () => {
		const { context, text } = recordingCanvas();
		drawNodeLabel(context, { ...baseNode, badge: "12345" }, settings);
		drawNodeLabel(
			context,
			{ ...baseNode, x: 120, label: "Overlapping", forceLabel: true },
			settings,
		);
		drawNodeLabel(context, { ...baseNode, x: 190, label: "Clear" }, settings);

		expect(text.map(({ label }) => label)).toEqual(["Short", "12345", "Clear"]);
	});

	test("uses the actual bounds when a caption flips left at the viewport edge", () => {
		const { context, text } = recordingCanvas(500);
		drawNodeLabel(
			context,
			{ ...baseNode, x: 420, label: "Left caption", badge: "99" },
			settings,
		);
		drawNodeLabel(context, { ...baseNode, x: 300, label: "Overlap" }, settings);
		expect(text.map(({ label }) => label)).toEqual(["Left caption", "99"]);
	});

	test("resets culling for each render without sharing state across canvases", () => {
		const first = recordingCanvas();
		const second = recordingCanvas();
		drawNodeLabel(first.context, baseNode, settings);
		drawNodeLabel(first.context, baseNode, settings);
		drawNodeLabel(second.context, baseNode, settings);
		expect(first.text).toHaveLength(1);
		expect(second.text).toHaveLength(1);

		resetNodeLabelLayout(first.context);
		drawNodeLabel(first.context, baseNode, settings);
		expect(first.text).toHaveLength(2);
	});

	test("does not leave a duplicate caption behind the highlighted card", () => {
		const { context, text, pills } = recordingCanvas();
		drawNodeLabel(context, { ...baseNode, highlighted: true }, settings);
		drawNodeLabel(context, { ...baseNode, hidden: true }, settings);
		drawNodeHover(context, { ...baseNode, hidden: true }, settings);
		expect(text).toHaveLength(0);
		expect(pills).toHaveLength(0);
	});
});

describe("graph hover cards", () => {
	test.each([
		{ x: 30, y: 1 },
		{ x: 450, y: 199 },
	])(
		"keeps the caption and badge inside an opaque viewport-safe card at %o",
		(position) => {
			const { context, text, pills } = recordingCanvas(500);
			drawNodeHover(
				context,
				{
					...baseNode,
					...position,
					label:
						"A very long caption that needs to fit inside the graph viewport",
					badge: "1,234",
				},
				settings,
			);
			const card = pills[0];
			expect(text).toHaveLength(2);
			expect(card.fill).toBe("rgb(0,0,0)");
			expect(card.left).toBeGreaterThanOrEqual(GRAPH_LABEL_LEFT_INSET);
			expect(card.right).toBeLessThanOrEqual(500 - GRAPH_LABEL_RIGHT_INSET);
			expect(card.top).toBeGreaterThanOrEqual(4);
			expect(card.bottom).toBeLessThanOrEqual(196);
			for (const item of text) {
				expect(item.x).toBeGreaterThan(card.left);
				expect(item.x + item.width).toBeLessThan(card.right);
				expect(item.y).toBeGreaterThan(card.top);
				expect(item.y).toBeLessThan(card.bottom);
			}
		},
	);
});
