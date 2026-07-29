import { describe, expect, test } from "bun:test";
import {
	normalizePlotlyTitle,
	parseChartData,
	toNivoData,
	toPlotlyData,
} from "./chart-data-parser";

describe("markdown chart configuration", () => {
	test("maps Nivo labels and a positioned legend without clipping margins", () => {
		const input = parseChartData(
			`type: bar
xLabel: Quarter
yLabel: Revenue
showLegend: true
legendPosition: right
stacked: true
animate: false
---
quarter,product,services
Q1,12,18
Q2,16,21`,
			"nivo",
		);

		const result = toNivoData(input);
		expect(result.chartType).toBe("bar");
		expect(result.props.axisBottom).toEqual({
			legend: "Quarter",
			legendOffset: 44,
			legendPosition: "middle",
		});
		expect(result.props.axisLeft).toEqual({
			legend: "Revenue",
			legendOffset: -52,
			legendPosition: "middle",
		});
		expect(result.props.margin).toEqual({
			top: 30,
			right: 136,
			bottom: 68,
			left: 78,
		});
		expect(result.props.groupMode).toBe("stacked");
		expect(result.props.animate).toBe(false);
		expect(result.props.legends).toEqual([
			expect.objectContaining({
				anchor: "right",
				direction: "column",
				translateX: 112,
			}),
		]);
	});

	test("maps an explicit false stacked option to grouped Nivo bars", () => {
		const input = parseChartData(
			`type: bar
stacked: false
---
quarter,revenue,cost
Q1,120,80`,
			"nivo",
		);

		expect(toNivoData(input).props.groupMode).toBe("grouped");
	});

	test("maps Plotly titles, stacking, horizontal bars, and legend placement", () => {
		const input = parseChartData(
			`type: bar
title: Quarterly mix
xLabel: Amount
yLabel: Quarter
showLegend: true
legendPosition: bottom
stacked: true
layout: horizontal
---
quarter,product,services
Q1,12,18
Q2,16,21`,
			"plotly",
		);

		const result = toPlotlyData(input);
		expect(result.layout.title).toEqual({ text: "Quarterly mix" });
		expect(result.layout.xaxis).toEqual(
			expect.objectContaining({ title: { text: "Amount" } }),
		);
		expect(result.layout.yaxis).toEqual({ title: { text: "Quarter" } });
		expect(result.layout.barmode).toBe("stack");
		expect(result.layout.legend).toEqual(
			expect.objectContaining({
				orientation: "h",
				xanchor: "center",
				yanchor: "top",
			}),
		);
		expect(result.layout.margin).toEqual({ t: 40, r: 20, b: 76, l: 50 });
		expect(result.data[0]).toEqual(
			expect.objectContaining({
				orientation: "h",
				x: [12, 16],
				y: ["Q1", "Q2"],
			}),
		);
	});

	test("normalizes native Plotly string titles without losing their text", () => {
		expect(normalizePlotlyTitle("Monthly sales")).toEqual({
			text: "Monthly sales",
		});
		expect(normalizePlotlyTitle({ text: "Monthly sales", x: 0.5 })).toEqual({
			text: "Monthly sales",
			x: 0.5,
		});
		expect(normalizePlotlyTitle(null)).toEqual({});
	});
});
