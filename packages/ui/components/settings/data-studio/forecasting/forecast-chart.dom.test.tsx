// @vitest-environment happy-dom
import React, { act, type ComponentProps, type ReactNode } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ForecastChart } from "./forecast-chart";
import type { ForecastPlotPoint } from "./forecast-data";

type ChartEvent = { activeLabel?: number };
type MockChartProps = {
	children: ReactNode;
	data: { timestamp: number }[];
	onMouseDown?: (state: ChartEvent, event: { button: number }) => void;
	onMouseMove?: (state: ChartEvent) => void;
	onMouseUp?: () => void;
	onMouseLeave?: () => void;
	onDoubleClick?: () => void;
};

// Preserve the actual controls and dialog. Only chart layout is replaced, so
// DOM tests can inspect the visible dates without relying on SVG measurements.
vi.mock("recharts", () => ({
	ResponsiveContainer: ({ children }: { children: ReactNode }) => children,
	ComposedChart: ({ children, data, ...events }: MockChartProps) => (
		<svg
			data-testid="forecast-plot"
			data-times={JSON.stringify(data.map((point) => point.timestamp))}
			onMouseDown={(event) =>
				events.onMouseDown?.({ activeLabel: event.clientX }, event)
			}
			onMouseMove={(event) =>
				events.onMouseMove?.({ activeLabel: event.clientX })
			}
			onMouseUp={events.onMouseUp}
			onMouseLeave={events.onMouseLeave}
			onDoubleClick={events.onDoubleClick}
		>
			<title>Forecast plot</title>
			{children}
		</svg>
	),
	LineChart: ({ children, data }: MockChartProps) => (
		<svg
			data-testid="auxiliary-plot"
			data-times={JSON.stringify(data.map((point) => point.timestamp))}
		>
			<title>Auxiliary plot</title>
			{children}
		</svg>
	),
	XAxis: ({ domain }: { domain?: (number | string)[] }) => (
		<g data-testid="time-axis" data-domain={JSON.stringify(domain)} />
	),
	ReferenceArea: ({ x1, x2 }: { x1: number; x2: number }) => (
		<g data-testid="reference-area" data-range={JSON.stringify([x1, x2])} />
	),
	ReferenceLine: ({ x }: { x: number }) => (
		<g data-testid="reference-line" data-time={x} />
	),
	Area: () => null,
	CartesianGrid: () => null,
	Line: () => null,
	Tooltip: () => null,
	YAxis: () => null,
}));

const points: ForecastPlotPoint[] = Array.from({ length: 9 }, (_, index) => ({
	timestamp: (index + 1) * 1000,
	history: index <= 5 ? 20 + index : null,
	actual: null,
	forecast: index >= 5 ? 20 + index : null,
	interval: null,
}));
const defaults: ComponentProps<typeof ForecastChart> = {
	points,
	cutoff: 6000,
	metric: "Temperature",
	seriesLabel: "Machine 1",
	quantileLevels: [],
	loading: false,
	stale: false,
	backtest: false,
};
let root: Root;
let container: HTMLDivElement;

function required<T>(value: T | null | undefined): T {
	if (value == null) throw new Error("Expected test element");
	return value;
}
function button(label: string, scope: ParentNode = container) {
	return required(
		[...scope.querySelectorAll<HTMLButtonElement>("button")].find(
			(element) =>
				element.getAttribute("aria-label") === label ||
				element.textContent?.trim() === label,
		),
	);
}
function plot(scope: ParentNode = container) {
	return required(
		scope.querySelector<SVGElement>('[data-testid="forecast-plot"]'),
	);
}
function dates(scope: ParentNode = container): number[] {
	return JSON.parse(required(plot(scope).getAttribute("data-times")));
}
function bounds(scope: ParentNode = container) {
	const values = dates(scope);
	return [values[0], values.at(-1)];
}
async function render(
	props: Partial<ComponentProps<typeof ForecastChart>> = {},
) {
	await act(async () =>
		root.render(<ForecastChart {...defaults} {...props} />),
	);
}
async function click(label: string, scope: ParentNode = container) {
	await act(async () => button(label, scope).click());
}
async function key(element: Element, value: string) {
	await act(async () => {
		if (element instanceof HTMLElement) element.focus();
		element.dispatchEvent(
			new KeyboardEvent("keydown", {
				key: value,
				bubbles: true,
				cancelable: true,
			}),
		);
	});
}
async function drag(start: number, end: number) {
	for (const [type, time] of [
		["mousedown", start],
		["mousemove", end],
		["mouseup", end],
	] as const) {
		await act(async () => {
			plot().dispatchEvent(
				new MouseEvent(type, { clientX: time, bubbles: true, button: 0 }),
			);
		});
	}
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true, React });
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => {
		root.unmount();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	container.remove();
});

describe("forecast chart exploration", () => {
	it("bounds zoom to two points, expands within the full range and resets", async () => {
		await render();
		expect(bounds()).toEqual([1000, 9000]);
		expect(button("Zoom out").disabled).toBe(true);
		await click("Zoom in");
		expect(bounds()).toEqual([3000, 7000]);
		await click("Zoom in");
		await click("Zoom in");
		expect(dates()).toHaveLength(2);
		expect(button("Zoom in").disabled).toBe(true);
		for (let index = 0; index < 5; index++) await click("Zoom out");
		expect(bounds()).toEqual([1000, 9000]);
		expect(button("Zoom out").disabled).toBe(true);
		await click("Focus forecast");
		expect(bounds()).toEqual([5000, 9000]);
		await click("Reset zoom");
		expect(bounds()).toEqual([1000, 9000]);
	});

	it("moves both range handles with the keyboard without crossing them", async () => {
		await render();
		const start = required(
			container.querySelector<HTMLElement>(
				'[aria-label="Visible range start"]',
			),
		);
		const end = required(
			container.querySelector<HTMLElement>('[aria-label="Visible range end"]'),
		);
		await key(start, "ArrowRight");
		await key(end, "ArrowLeft");
		expect(bounds()).toEqual([2000, 8000]);
		expect(start.getAttribute("aria-valuetext")).toContain("UTC");
		for (let index = 0; index < 12; index++) await key(start, "ArrowRight");
		expect(bounds()).toEqual([7000, 8000]);
		expect(dates()).toHaveLength(2);
	});

	it("zooms a reverse drag, ignores a click and resets with a double click", async () => {
		await render();
		await drag(7000, 3000);
		expect(bounds()).toEqual([3000, 7000]);
		await drag(4000, 4000);
		expect(bounds()).toEqual([3000, 7000]);
		await act(async () =>
			plot().dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
		);
		expect(bounds()).toEqual([1000, 9000]);
	});

	it("resets for new results, series and forecast views while preserving ordinary renders", async () => {
		await render();
		await click("Zoom in");
		await render({ stale: true });
		expect(bounds()).toEqual([3000, 7000]);
		await render({ points: [...points] });
		expect(bounds()).toEqual([1000, 9000]);
		for (const changed of [
			{ seriesLabel: "Machine 2" },
			{ metric: "Pressure" },
			{ backtest: true },
			{ cutoff: 5000 },
		]) {
			await render();
			await click("Zoom in");
			await render(changed);
			expect(bounds()).toEqual([1000, 9000]);
		}
	});

	it("shares the selected dates with covariates and clips the cutoff annotation", async () => {
		await render({
			readOnly: true,
			covariates: [
				{
					name: "Promotion",
					role: "known",
					enabled: true,
					points: points.map((point) => ({
						timestamp: point.timestamp,
						value: 1,
					})),
				},
			],
		});
		await drag(1000, 4000);
		const auxiliary = container.querySelectorAll(
			'[data-testid="auxiliary-plot"]',
		);
		const covariate = required(auxiliary.item(auxiliary.length - 1));
		expect(JSON.parse(required(covariate.getAttribute("data-times")))).toEqual(
			dates(),
		);
		expect(plot().querySelector('[data-testid="reference-line"]')).toBeNull();
		expect(
			plot()
				.querySelector('[data-testid="reference-area"]')
				?.getAttribute("data-range"),
		).toBe("[1000,4000]");
	});

	it("keeps zoom in full screen and returns focus to the trigger on Escape", async () => {
		await render();
		await click("Zoom in");
		const trigger = button("Full screen");
		trigger.focus();
		await click("Full screen");
		const dialog = required(document.querySelector('[role="dialog"]'));
		expect(bounds(dialog)).toEqual([3000, 7000]);
		await click("Focus forecast", dialog);
		expect(bounds(dialog)).toEqual([5000, 9000]);
		await key(dialog, "Escape");
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
		expect(document.querySelector('[role="dialog"]')).toBeNull();
		expect(bounds()).toEqual([5000, 9000]);
		expect(document.activeElement).toBe(trigger);
	});

	it("keeps control-free compact embeds free of zoom and fullscreen interaction", async () => {
		await render({ compact: true, showControls: false });
		expect(container.querySelector('[aria-label="Chart controls"]')).toBeNull();
		expect(container.querySelector('[role="slider"]')).toBeNull();
		await drag(3000, 7000);
		expect(bounds()).toEqual([1000, 9000]);
		expect(document.querySelector('[role="dialog"]')).toBeNull();
	});

	it("closes fullscreen when controls are hidden without reopening it later", async () => {
		await render();
		await click("Full screen");
		expect(document.querySelector('[role="dialog"]')).not.toBeNull();
		await render({ showControls: false });
		expect(document.querySelector('[role="dialog"]')).toBeNull();
		await render({ showControls: true });
		expect(document.querySelector('[role="dialog"]')).toBeNull();
	});

	it("keeps an exit button available if the expanded result becomes empty", async () => {
		await render();
		await click("Full screen");
		await render({ points: [] });
		const dialog = required(document.querySelector('[role="dialog"]'));
		expect(dialog.textContent).toContain("See where your data is heading");
		await click("Exit full screen", dialog);
		expect(document.querySelector('[role="dialog"]')).toBeNull();
	});

	it("shows the empty state and disables zoom for a single observation", async () => {
		await render({ points: [] });
		expect(container.textContent).toContain("See where your data is heading");
		expect(container.querySelector('[aria-label="Chart controls"]')).toBeNull();
		await render({ points: [points[0]] });
		expect(dates()).toEqual([1000]);
		expect(button("Zoom in").disabled).toBe(true);
		expect(button("Zoom out").disabled).toBe(true);
		expect(button("Focus forecast").disabled).toBe(true);
		expect(container.querySelector('[role="slider"]')).toBeNull();
	});
});
