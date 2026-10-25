"use client";

import { useTranslation } from "@flow-like/locales";
import { ResponsiveBar } from "@nivo/bar";
import {
	type Point,
	type PointTooltipProps,
	ResponsiveLine,
	type SliceTooltipProps,
} from "@nivo/line";
import { ResponsivePie } from "@nivo/pie";
import { useMemo } from "react";
import type {
	IDailyStat,
	IFlowPaymentDay,
	ISalesOverview,
} from "../../../state/backend-state/sales-state";
import { formatCurrency, formatDay } from "./sales-format";

/** Theme tokens are `oklch()`, so they are referenced bare — `hsl(var(--x))` is an invalid colour. */
const chartTheme = {
	axis: {
		ticks: { text: { fill: "var(--muted-foreground)" } },
		legend: { text: { fill: "var(--muted-foreground)" } },
	},
	grid: { line: { stroke: "var(--border)" } },
	legends: { text: { fill: "var(--muted-foreground)" } },
	labels: { text: { fill: "var(--foreground)" } },
	crosshair: { line: { stroke: "var(--primary)" } },
	tooltip: {
		container: {
			background: "var(--popover)",
			color: "var(--popover-foreground)",
			border: "1px solid var(--border)",
			borderRadius: "8px",
			fontSize: "12px",
		},
	},
};

type RevenueSeries = {
	id: string;
	data: { x: string; y: number; cents: number }[];
};

function RevenueTooltip({
	points,
}: {
	points: readonly Point<RevenueSeries>[];
}) {
	return (
		<div className="whitespace-nowrap rounded-lg border border-border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-xl">
			<div className="mb-1 font-medium">{points[0]?.data.x}</div>
			{points.map((point) => (
				<div key={point.id} className="flex items-center justify-between gap-5">
					<span className="flex items-center gap-2 text-muted-foreground">
						<span
							className="h-2.5 w-2.5 shrink-0 rounded-sm"
							style={{ backgroundColor: point.seriesColor }}
						/>
						{point.seriesId}
					</span>
					<span className="font-medium tabular-nums">
						{formatCurrency(point.data.cents)}
					</span>
				</div>
			))}
		</div>
	);
}

function RevenuePointTooltip({ point }: PointTooltipProps<RevenueSeries>) {
	return <RevenueTooltip points={[point]} />;
}

function RevenueSliceTooltip({ slice }: SliceTooltipProps<RevenueSeries>) {
	return <RevenueTooltip points={slice.points} />;
}

function ChartEmpty({ height, message }: { height: number; message: string }) {
	return (
		<div
			className="flex items-center justify-center text-muted-foreground"
			style={{ height }}
		>
			{message}
		</div>
	);
}

/**
 * Store sales and flow payments on one time axis. A source the app does not
 * use is left out rather than drawn as a flat zero line.
 */
export function RevenueChart({
	store,
	flows,
}: {
	store?: IDailyStat[];
	flows?: IFlowPaymentDay[];
}) {
	const { t } = useTranslation("settings");
	const chartData = useMemo<RevenueSeries[]>(() => {
		const days = [
			...new Set([
				...(store ?? []).map((day) => day.date),
				...(flows ?? []).map((day) => day.date),
			]),
		].sort();
		if (days.length === 0) return [];
		const series = (
			id: string,
			rows: { date: string; revenue: number }[],
		): RevenueSeries => {
			const revenue = new Map(rows.map((row) => [row.date, row.revenue]));
			return {
				id,
				data: days.map((day) => {
					const cents = revenue.get(day) ?? 0;
					return { x: formatDay(day), y: cents / 100, cents };
				}),
			};
		};
		return [
			...(store ? [series(t("storeSales", "Store sales"), store)] : []),
			...(flows ? [series(t("flowPayments", "Flow payments"), flows)] : []),
		];
	}, [store, flows, t]);

	if (chartData.length === 0) {
		return (
			<ChartEmpty
				height={300}
				message={t("noRevenueDataAvailable", "No revenue data available")}
			/>
		);
	}

	const stacked = chartData.length > 1;
	return (
		<div className="h-[300px]">
			<ResponsiveLine
				data={chartData}
				margin={{ top: stacked ? 40 : 20, right: 20, bottom: 50, left: 60 }}
				xScale={{ type: "point" }}
				yScale={{ type: "linear", min: 0, max: "auto", stacked }}
				axisBottom={{
					tickRotation: -45,
					legend: t("date", "Date"),
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: t("revenueInEuro", "Revenue (€)"),
					legendOffset: -50,
					legendPosition: "middle",
					format: (v) => `€${v}`,
				}}
				colors={{ scheme: "category10" }}
				pointSize={stacked ? 6 : 8}
				pointBorderWidth={2}
				pointBorderColor={{ from: "serieColor" }}
				enableArea={true}
				areaOpacity={stacked ? 0.25 : 0.1}
				useMesh={true}
				enableSlices={stacked ? "x" : false}
				tooltip={RevenuePointTooltip}
				sliceTooltip={RevenueSliceTooltip}
				enableGridX={false}
				legends={
					stacked
						? [
								{
									anchor: "top-left",
									direction: "row",
									translateY: -32,
									itemWidth: 120,
									itemHeight: 20,
									symbolSize: 10,
									symbolShape: "circle",
								},
							]
						: []
				}
				theme={chartTheme}
			/>
		</div>
	);
}

export function PurchasesChart({ data }: { data: IDailyStat[] }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: formatDay(d.date),
				purchases: d.purchases,
				refunds: d.refunds,
			})),
		[data],
	);

	if (data.length === 0) {
		return (
			<ChartEmpty
				height={300}
				message={t("noPurchaseDataAvailable", "No purchase data available")}
			/>
		);
	}

	return (
		<div className="h-[300px]">
			<ResponsiveBar
				data={chartData}
				keys={["purchases", "refunds"]}
				indexBy="date"
				margin={{ top: 20, right: 100, bottom: 50, left: 60 }}
				padding={0.3}
				groupMode="grouped"
				colors={{ scheme: "paired" }}
				axisBottom={{
					tickRotation: -45,
					legend: t("date", "Date"),
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: t("count", "Count"),
					legendOffset: -50,
					legendPosition: "middle",
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "bottom-right",
						direction: "column",
						translateX: 100,
						itemWidth: 80,
						itemHeight: 20,
					},
				]}
				theme={chartTheme}
			/>
		</div>
	);
}

export function RevenueBreakdownChart({ data }: { data: ISalesOverview }) {
	const { t } = useTranslation("settings");
	const chartData = useMemo(
		() =>
			[
				{
					id: t("netRevenue", "Net Revenue"),
					value: data.netRevenue / 100,
					color: "hsl(142, 76%, 36%)",
				},
				{
					id: t("refunds", "Refunds"),
					value: data.refundAmount / 100,
					color: "hsl(0, 84%, 60%)",
				},
				{
					id: t("discounts", "Discounts"),
					value: data.totalDiscounts / 100,
					color: "hsl(45, 93%, 47%)",
				},
			].filter((d) => d.value > 0),
		[data, t],
	);

	if (chartData.length === 0) {
		return (
			<ChartEmpty
				height={250}
				message={t(
					"noRevenueBreakdownAvailable",
					"No revenue breakdown available",
				)}
			/>
		);
	}

	return (
		<div className="h-[250px]">
			<ResponsivePie
				data={chartData}
				margin={{ top: 20, right: 80, bottom: 20, left: 80 }}
				innerRadius={0.5}
				padAngle={0.7}
				cornerRadius={3}
				activeOuterRadiusOffset={8}
				colors={{ datum: "data.color" }}
				arcLinkLabelsSkipAngle={10}
				arcLinkLabelsTextColor="var(--muted-foreground)"
				arcLinkLabelsThickness={2}
				arcLabelsSkipAngle={10}
				arcLabelsTextColor="white"
				valueFormat={(v) => `€${v.toFixed(0)}`}
				theme={chartTheme}
			/>
		</div>
	);
}
