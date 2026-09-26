"use client";

import { useTranslation } from "@flow-like/locales";
import { ResponsiveBar } from "@nivo/bar";
import { ResponsiveLine } from "@nivo/line";
import { ResponsivePie } from "@nivo/pie";
import { useMemo } from "react";
import type {
	IDailyStat,
	IFlowPaymentDay,
	ISalesOverview,
} from "../../../state/backend-state/sales-state";
import { formatDay } from "./sales-format";

const axisTheme = {
	axis: {
		ticks: { text: { fill: "hsl(var(--muted-foreground))" } },
		legend: { text: { fill: "hsl(var(--muted-foreground))" } },
	},
	grid: { line: { stroke: "hsl(var(--border))" } },
	legends: { text: { fill: "hsl(var(--muted-foreground))" } },
};

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
	const chartData = useMemo(() => {
		const days = [
			...new Set([
				...(store ?? []).map((day) => day.date),
				...(flows ?? []).map((day) => day.date),
			]),
		].sort();
		if (days.length === 0) return [];
		const series = (id: string, rows: { date: string; revenue: number }[]) => {
			const revenue = new Map(rows.map((row) => [row.date, row.revenue]));
			return {
				id,
				data: days.map((day) => ({
					x: formatDay(day),
					y: (revenue.get(day) ?? 0) / 100,
				})),
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
									itemTextColor: "hsl(var(--muted-foreground))",
								},
							]
						: []
				}
				theme={{
					...axisTheme,
					crosshair: { line: { stroke: "hsl(var(--primary))" } },
				}}
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
						itemTextColor: "hsl(var(--muted-foreground))",
					},
				]}
				theme={axisTheme}
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
				arcLinkLabelsTextColor="hsl(var(--muted-foreground))"
				arcLinkLabelsThickness={2}
				arcLabelsSkipAngle={10}
				arcLabelsTextColor="white"
				valueFormat={(v) => `€${v.toFixed(0)}`}
				theme={{
					labels: { text: { fill: "hsl(var(--foreground))" } },
				}}
			/>
		</div>
	);
}
