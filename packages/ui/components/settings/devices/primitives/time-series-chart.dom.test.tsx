import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import {
	allByRole,
	byRole,
	click,
	fire,
	installDom,
	keyDown,
	queryByRole,
} from "../testing/dom-harness";
import type { ChartSeries, TimeSeriesChartProps } from "./time-series-chart";

const dom = installDom();
const area = await import("./area-context");
const { TimeSeriesChart } = await import("./time-series-chart");

afterEach(dom.cleanup);
afterAll(dom.restore);

const HOUR = 3_600;
const START = 1_790_812_800;
const NOW_S = START + 24 * HOUR - 600;

function At({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<area.AreaNowContext.Provider value={NOW_S * 1000}>
			{children}
		</area.AreaNowContext.Provider>
	);
}

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ").trim();

const hours = (count: number, value: (hour: number) => number | null) =>
	Array.from({ length: count }, (_, hour) => value(hour));

const tokens: ChartSeries[] = [
	{
		id: "in",
		label: "In",
		tone: "context",
		values: hours(24, (hour) => (hour % 2 ? 40 : 0)),
		note: "480",
	},
	{
		id: "out",
		label: "Out",
		tone: "accent",
		values: hours(24, (hour) => 10 + hour),
		note: "516",
	},
];

const format = (value: number) => `${value} tok`;

function chart(props: Partial<TimeSeriesChartProps> = {}) {
	return dom.render(
		<At>
			<TimeSeriesChart
				title="Tokens"
				summary="996 in all"
				kind="columns"
				series={tokens}
				start={START}
				step={HOUR}
				format={format}
				partialLast
				emptyText="Nothing reported in this range."
				{...props}
			/>
		</At>,
	);
}

const plot = (root: ParentNode) =>
	root.querySelector("[data-plot]") as HTMLElement;

const figureOf = (root: ParentNode) =>
	root.querySelector("figure[data-chart]") as HTMLElement;

const tip = (root: ParentNode) => root.querySelector("[data-chart-tip]");

describe("columns", () => {
	test("one stacked column per slot: the gap is carved from the lower segment, the top one is rounded", async () => {
		const { container } = await chart();
		expect(container.querySelectorAll("[data-column]")).toHaveLength(24);
		const second = container.querySelector('[data-column="1"]') as Element;
		const lower = second.querySelector('[data-series="in"]') as Element;
		const upper = second.querySelector('[data-series="out"]') as Element;
		expect(lower.getAttribute("d")).not.toContain("A");
		expect(upper.getAttribute("d")).toContain("A");
		expect(lower.getAttribute("class")).toContain("fill-spark");
		expect(upper.getAttribute("class")).toContain("fill-info-solid");
		const first = container.querySelector('[data-column="0"]') as Element;
		expect(first.querySelector('[data-series="in"]')).toBeNull();
		expect(
			container.querySelector('[data-column="23"]')?.getAttribute("opacity"),
		).toBe("0.5");
	});

	test("title, headline and a legend that carries each series' total; values never wear the series colour", async () => {
		const { container } = await chart();
		const figure = figureOf(container);
		const title = figure.getAttribute("aria-labelledby") ?? "";
		expect(document.getElementById(title)?.textContent).toBe("Tokens");
		expect(text(figure.querySelector("figcaption") as Element)).toContain(
			"Tokens996 in all",
		);
		const legend = container.querySelector("[data-legend]") as Element;
		expect(text(legend)).toBe("In480Out516");
		for (const span of legend.querySelectorAll("span"))
			expect(span.className).not.toMatch(/info|spark/);
	});

	test("value gridlines run from zero to a clean top with the unit", async () => {
		const { container } = await chart();
		const labels = [...container.querySelectorAll("svg text")].map(
			(node) => node.textContent,
		);
		expect(labels).toContain("0 tok");
		expect(labels).toContain("50 tok");
		expect(labels).toContain("100 tok");
	});
});

describe("lines", () => {
	const latency: ChartSeries[] = [
		{
			id: "p95",
			label: "95th percentile",
			tone: "accent",
			values: hours(24, (hour) =>
				hour === 5 ? 900 : hour >= 10 && hour < 20 ? 200 + hour : null,
			),
		},
	];

	test("gaps break the line, a lone hour is a dot, the last point carries an end marker", async () => {
		const { container } = await chart({
			kind: "lines",
			series: latency,
			partialLast: false,
		});
		const group = container.querySelector('[data-series="p95"]') as Element;
		const line = [...group.querySelectorAll("path")].find(
			(path) => path.getAttribute("fill") === "none",
		) as Element;
		const d = line.getAttribute("d") ?? "";
		expect(d.match(/M/g)).toHaveLength(2);
		expect(d).toContain("l0,0");
		expect(group.querySelectorAll("circle")).toHaveLength(1);
		expect(container.querySelector("[data-legend]")).toBeNull();
	});

	test("one series gets a wash under its line", async () => {
		const { container } = await chart({ kind: "lines", series: latency });
		expect(
			[...container.querySelectorAll("path")].some((path) =>
				(path.getAttribute("class") ?? "").includes("fill-info-solid/10"),
			),
		).toBe(true);
	});
});

describe("reading values", () => {
	test("keyboard focus shows the latest slot; arrows, Home and Escape move and clear it", async () => {
		const { container } = await chart();
		const slider = byRole("slider", "Tokens", container);
		expect(tip(container)).toBeNull();
		await fire(slider, new window.FocusEvent("focusin", { bubbles: true }));
		expect(slider.getAttribute("aria-valuenow")).toBe("23");
		expect(text(tip(container) as Element)).toContain("(so far)");
		expect(text(tip(container) as Element)).toContain("33 tok");
		await keyDown(slider, "ArrowLeft");
		expect(slider.getAttribute("aria-valuenow")).toBe("22");
		expect(slider.getAttribute("aria-valuetext")).toContain(
			"In 0 tok, Out 32 tok",
		);
		await keyDown(slider, "Home");
		expect(slider.getAttribute("aria-valuenow")).toBe("0");
		await keyDown(slider, "Escape");
		expect(tip(container)).toBeNull();
	});

	test("the pointer picks the slot under it", async () => {
		const { container } = await chart();
		const target = plot(container);
		target.getBoundingClientRect = () =>
			({ left: 0, top: 0, width: 560, height: 140 }) as DOMRect;
		await fire(
			target,
			new window.PointerEvent("pointermove", { bubbles: true, clientX: 60 }),
		);
		expect(
			container.querySelector("[data-cursor]")?.getAttribute("data-cursor"),
		).toBe("0");
		expect(text(tip(container) as Element)).toContain("10 tok");
		await fire(
			target,
			new window.PointerEvent("pointerout", {
				bubbles: true,
				relatedTarget: document.body,
			}),
		);
		expect(tip(container)).toBeNull();
	});

	test("the table view lists every slot with its span, gaps as a dash", async () => {
		const series: ChartSeries[] = [
			{
				id: "p50",
				label: "Median",
				tone: "context",
				values: hours(24, (hour) => (hour < 2 ? null : 100)),
			},
		];
		const { container } = await chart({ kind: "lines", series });
		const show = byRole("button", "Show table", container);
		expect(show.hasAttribute("aria-pressed")).toBe(false);
		await click(show);
		const table = byRole("table", "Tokens", container);
		const rows = table.querySelectorAll("tbody tr");
		expect(rows).toHaveLength(24);
		expect(text(rows[0] as Element)).toContain("–");
		expect(text(rows[23] as Element)).toContain("(so far)");
		expect(text(rows[23] as Element)).toContain("100 tok");
		expect(queryByRole("slider", undefined, container)).toBeNull();
		const back = byRole("button", "Show chart", container);
		expect(back.hasAttribute("aria-pressed")).toBe(false);
		await click(back);
		expect(allByRole("slider", undefined, container)).toHaveLength(1);
	});

	test("the table is formatted again when the day changes, not on every tick of the area clock", async () => {
		let formatted = 0;
		const counting = (value: number) => {
			formatted++;
			return format(value);
		};
		const at = (nowMs: number) => (
			<area.AreaNowContext.Provider value={nowMs}>
				<TimeSeriesChart
					title="Tokens"
					kind="columns"
					series={tokens}
					start={START}
					step={HOUR}
					format={counting}
					partialLast
				/>
			</area.AreaNowContext.Provider>
		);
		const view = await dom.render(at(NOW_S * 1000));
		const plotted = formatted;
		await click(byRole("button", "Show table", view.container));
		const table = formatted - plotted;
		expect(table).toBe(24 * tokens.length);
		for (const tick of [1, 2, 3])
			await view.rerender(at((NOW_S + tick) * 1000));
		expect(formatted).toBe(plotted + table);
		await view.rerender(at((NOW_S + 24 * HOUR) * 1000));
		expect(formatted).toBe(plotted + 2 * table);
	});
});

describe("states", () => {
	test("nothing to draw shows the empty sentence instead of axes", async () => {
		const { container } = await chart({
			series: [{ ...tokens[1], values: hours(24, () => 0) } as ChartSeries],
		});
		expect(text(container)).toContain("Nothing reported in this range.");
		expect(container.querySelector("svg")).toBeNull();
		expect(queryByRole("button", "Show table", container)).toBeNull();
	});

	test("a newer read under way dims the last plot", async () => {
		const { container } = await chart({ busy: true });
		const figure = figureOf(container);
		expect(figure.getAttribute("aria-busy")).toBe("true");
		expect(figure.className).toContain("opacity-60");
	});
});
