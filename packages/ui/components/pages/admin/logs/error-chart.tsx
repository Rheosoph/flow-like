"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";
import {
	type ChartConfig,
	ChartContainer,
	ChartTooltip,
	ChartTooltipContent,
} from "../../../ui/chart";
import type { IErrorTimeseriesPoint } from "./types";

interface ErrorChartProps {
	points: IErrorTimeseriesPoint[];
	bucket: string;
}

const chartConfig = {
	server: {
		label: "Server (5xx)",
		color: "var(--destructive)",
	},
	client: {
		label: "Client (4xx)",
		color: "var(--chart-3)",
	},
} satisfies ChartConfig;

function formatTick(value: string, bucket: string) {
	const d = new Date(value);
	if (Number.isNaN(d.getTime())) return value;
	if (bucket === "minute") {
		return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}
	if (bucket === "hour") {
		return d.toLocaleTimeString([], { hour: "2-digit" });
	}
	return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function ErrorChart({ points, bucket }: Readonly<ErrorChartProps>) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			points.map((p) => ({
				bucket: p.bucket,
				server: p.server,
				client: p.client,
			})),
		[points],
	);

	if (data.length === 0) {
		return (
			<div className="flex h-64 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
				{t("noErrorsInTheSelectedWindow", "No errors in the selected window.")}
			</div>
		);
	}

	return (
		<ChartContainer config={chartConfig} className="h-64 w-full">
			<AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
				<defs>
					<linearGradient id="errChartServer" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-server)"
							stopOpacity={0.5}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-server)"
							stopOpacity={0.05}
						/>
					</linearGradient>
					<linearGradient id="errChartClient" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-client)"
							stopOpacity={0.4}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-client)"
							stopOpacity={0.05}
						/>
					</linearGradient>
				</defs>
				<CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.4} />
				<XAxis
					dataKey="bucket"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					tickFormatter={(v) => formatTick(v, bucket)}
					minTickGap={32}
				/>
				<YAxis
					allowDecimals={false}
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					width={32}
				/>
				<ChartTooltip
					cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
					content={
						<ChartTooltipContent
							indicator="dot"
							labelFormatter={(value) => formatTick(value as string, bucket)}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="client"
					stackId="1"
					stroke="var(--color-client)"
					fill="url(#errChartClient)"
					strokeWidth={1.5}
				/>
				<Area
					type="monotone"
					dataKey="server"
					stackId="1"
					stroke="var(--color-server)"
					fill="url(#errChartServer)"
					strokeWidth={1.5}
				/>
			</AreaChart>
		</ChartContainer>
	);
}
