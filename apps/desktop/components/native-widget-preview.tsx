"use client";

import type {
	NativeCustomWidget,
	NativeWidgetChart,
	NativeWidgetPageNode,
} from "@flow-like/flow-like-ui/lib/native-widget";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { ArrowUpRightIcon, CircleAlertIcon, ImageIcon } from "lucide-react";
import { useTheme } from "next-themes";
import type { CSSProperties } from "react";

const colors = [
	"var(--widget-accent)",
	"#4c9dce",
	"#60ad99",
	"#9b80c3",
	"#d5aa49",
	"#d87693",
];
const accents = {
	orange: ["#d56123", "#ffa665"],
	blue: ["#2165cd", "#80b7ff"],
	teal: ["#087f72", "#58d6bd"],
	purple: ["#7952c7", "#c2a4ff"],
};
const clamp = (value: number) => Math.max(0, Math.min(1, value));

function ProgressMeter({ value }: { value: number }) {
	return (
		<div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-[var(--widget-inset)]">
			<progress
				className="sr-only"
				aria-label="Progress"
				value={clamp(value)}
				max={1}
			/>
			<div
				className="h-full rounded-full bg-[var(--widget-accent)]"
				style={{ width: `${clamp(value) * 100}%` }}
			/>
		</div>
	);
}

function ChartLegend({ labels }: { labels: string[] }) {
	return (
		<div className="flex shrink-0 items-center gap-2 overflow-hidden text-[9px]">
			{labels.slice(0, 4).map((label, index) => (
				<span key={label} className="flex min-w-0 items-center gap-1">
					<span
						className="size-1.5 shrink-0 rounded-full"
						style={{ background: colors[index % colors.length] }}
					/>
					<span className="truncate">{label}</span>
				</span>
			))}
			{labels.length > 4 && (
				<span className="shrink-0">+{labels.length - 4}</span>
			)}
		</div>
	);
}

function ChartPreview({
	chart,
	compact,
}: { chart: NativeWidgetChart; compact: boolean }) {
	const scalar = chart.value ?? chart.points.at(-1)?.value ?? 0;
	const formatted =
		chart.formattedValue ?? chart.points.at(-1)?.formattedValue ?? "0";
	const formatter = new Intl.NumberFormat(undefined, {
		style: chart.format.style === "number" ? "decimal" : chart.format.style,
		currency: chart.format.currency,
		maximumFractionDigits: chart.format.decimals,
	});
	const ratio = clamp(scalar / Math.max(chart.target ?? 1, 0.000001));
	if (chart.type === "gauge")
		return (
			<div className="flex h-full min-h-0 items-center justify-center">
				<svg
					viewBox="0 0 160 160"
					className="h-full max-h-28 max-w-full"
					role="img"
					aria-label={`${formatted}${chart.target ? ` of ${formatter.format(chart.target)}` : ""}`}
				>
					<circle
						cx="80"
						cy="80"
						r="51"
						fill="none"
						stroke="var(--widget-inset)"
						strokeWidth="9"
						strokeLinecap="round"
						pathLength="100"
						strokeDasharray="75 25"
						transform="rotate(135 80 80)"
					/>
					{ratio > 0 && (
						<circle
							cx="80"
							cy="80"
							r="51"
							fill="none"
							stroke="var(--widget-accent)"
							strokeWidth="9"
							strokeLinecap="round"
							pathLength="100"
							strokeDasharray={`${ratio * 75} ${100 - ratio * 75}`}
							transform="rotate(135 80 80)"
						/>
					)}
					<text
						x="80"
						y="86"
						textAnchor="middle"
						fill="currentColor"
						fontSize={formatted.length > 8 ? 15 : 24}
						fontWeight="600"
					>
						{formatted}
					</text>
					<text
						x="80"
						y="126"
						textAnchor="middle"
						fill="currentColor"
						fontSize="12"
					>
						Goal
					</text>
				</svg>
			</div>
		);
	if (chart.type === "stat" || chart.type === "progress")
		return (
			<div className="flex h-full min-h-0 flex-col justify-center gap-2">
				<div
					className={cn(
						"truncate font-semibold tracking-tight tabular-nums",
						compact ? "text-3xl" : "text-4xl",
						chart.type === "stat" && "text-[var(--widget-accent)]",
					)}
				>
					{formatted}
				</div>
				{chart.type === "stat" ? (
					<div className="line-clamp-2 text-xs text-[var(--widget-secondary)]">
						{chart.points.at(-1)?.label}
					</div>
				) : (
					<>
						<div className="flex">
							<ProgressMeter value={ratio} />
						</div>
						{chart.target !== undefined && (
							<span className="truncate text-xs text-[var(--widget-secondary)]">
								of {formatter.format(chart.target)}
							</span>
						)}
					</>
				)}
			</div>
		);
	const series = [...new Set(chart.points.map((point) => point.series))];
	const labels = [...new Set(chart.points.map((point) => point.label))];
	if (chart.type === "pie" || chart.type === "donut") {
		const total = chart.points.reduce(
			(sum, point) => sum + Math.max(0, point.value),
			0,
		);
		let offset = 0;
		return (
			<div className="flex h-full min-h-0 flex-col gap-1">
				<svg
					viewBox="0 0 160 160"
					className="min-h-0 w-full flex-1"
					role="img"
					aria-label="Chart preview"
				>
					{chart.points.map((point) => {
						const share =
							total > 0 ? (Math.max(0, point.value) / total) * 100 : 0;
						const start = offset;
						offset += share;
						const category = labels.indexOf(point.label);
						return (
							<circle
								key={point.id}
								cx="80"
								cy="80"
								r={chart.type === "donut" ? 55 : 34}
								fill="none"
								stroke={colors[category % colors.length]}
								strokeWidth={chart.type === "donut" ? 22 : 68}
								pathLength="100"
								strokeDasharray={`${Math.max(0, share - 0.5)} ${100 - Math.max(0, share - 0.5)}`}
								strokeDashoffset={-start}
								transform="rotate(-90 80 80)"
							>
								<title>{`${point.label}: ${point.formattedValue}`}</title>
							</circle>
						);
					})}
				</svg>
				{!compact && labels.length > 1 && <ChartLegend labels={labels} />}
			</div>
		);
	}
	const cumulative = new Map<string, { positive: number; negative: number }>();
	const bars = chart.points.map((point) => {
		if (chart.type !== "stacked") return { point, start: 0, end: point.value };
		const totals = cumulative.get(point.label) ?? { positive: 0, negative: 0 };
		const side = point.value >= 0 ? "positive" : "negative";
		const start = totals[side];
		totals[side] += point.value;
		cumulative.set(point.label, totals);
		return { point, start, end: totals[side] };
	});
	const minimum = Math.min(
		0,
		...bars.flatMap(({ start, end }) => [start, end]),
	);
	const maximum = Math.max(
		0,
		...bars.flatMap(({ start, end }) => [start, end]),
	);
	const span = maximum - minimum || 1;
	const left = compact ? 4 : 48;
	const right = 294;
	const top = 8;
	const bottom = compact ? 150 : 132;
	const width = right - left;
	const height = bottom - top;
	const y = (value: number) => bottom - ((value - minimum) / span) * height;
	const horizontalX = (value: number) =>
		left + ((value - minimum) / span) * width;
	const x = (index: number) =>
		left + ((index + 0.5) * width) / Math.max(1, labels.length);
	const numericTicks = [minimum, minimum + span / 2, minimum + span];
	const labelTicks = [
		...new Set([0, Math.floor((labels.length - 1) / 2), labels.length - 1]),
	].filter((index) => index >= 0);
	return (
		<div className="flex h-full min-h-0 flex-col gap-1">
			<svg
				viewBox="0 0 300 156"
				preserveAspectRatio="none"
				className="min-h-0 w-full flex-1"
				role="img"
				aria-label="Chart preview"
			>
				{!compact &&
					(chart.type === "horizontal" ? (
						<>
							{numericTicks.map((value) => (
								<text
									key={value}
									x={horizontalX(value)}
									y="149"
									textAnchor="middle"
									fill="var(--widget-secondary)"
									fontSize="8"
								>
									{formatter.format(value)}
								</text>
							))}
							{labelTicks.map((index) => (
								<text
									key={index}
									x={left - 4}
									y={
										top + ((index + 0.5) * height) / Math.max(1, labels.length)
									}
									textAnchor="end"
									dominantBaseline="middle"
									fill="var(--widget-secondary)"
									fontSize="8"
								>
									{labels[index].slice(0, 10)}
								</text>
							))}
						</>
					) : (
						<>
							{numericTicks.map((value) => (
								<g key={value}>
									<path
										d={`M${left} ${y(value)} H${right}`}
										stroke="var(--widget-border)"
									/>
									<text
										x={left - 5}
										y={y(value)}
										textAnchor="end"
										dominantBaseline="middle"
										fill="var(--widget-secondary)"
										fontSize="8"
									>
										{formatter.format(value)}
									</text>
								</g>
							))}
							{labelTicks.map((index) => (
								<text
									key={index}
									x={x(index)}
									y="149"
									textAnchor="middle"
									fill="var(--widget-secondary)"
									fontSize="8"
								>
									{labels[index].slice(0, 12)}
								</text>
							))}
						</>
					))}
				{chart.type === "horizontal" && (
					<path
						d={`M${horizontalX(0)} ${top} V${bottom}`}
						stroke="var(--widget-border)"
					/>
				)}
				{chart.type === "line" || chart.type === "area"
					? series.map((name, index) => {
							const points = chart.points
								.filter((point) => point.series === name)
								.sort(
									(a, b) => labels.indexOf(a.label) - labels.indexOf(b.label),
								);
							const path = points
								.map(
									(point, index) =>
										`${index ? "L" : "M"}${x(labels.indexOf(point.label))} ${y(point.value)}`,
								)
								.join(" ");
							return (
								<g key={name}>
									<title>{name}</title>
									{chart.type === "area" && points.length > 0 ? (
										<path
											d={`${path} L${x(labels.indexOf(points[points.length - 1].label))} ${y(0)} L${x(labels.indexOf(points[0].label))} ${y(0)} Z`}
											fill={colors[index % colors.length]}
											opacity=".35"
										/>
									) : (
										<>
											<path
												d={path}
												fill="none"
												stroke={colors[index % colors.length]}
												strokeWidth="2.5"
											/>
											{points.map((point) => (
												<circle
													key={point.id}
													cx={x(labels.indexOf(point.label))}
													cy={y(point.value)}
													r="2"
													fill={colors[index % colors.length]}
												>
													<title>{point.formattedValue}</title>
												</circle>
											))}
										</>
									)}
								</g>
							);
						})
					: bars.map(({ point, start, end }) => {
							const group = labels.indexOf(point.label);
							const seriesIndex = series.indexOf(point.series);
							const groupSize =
								(chart.type === "horizontal" ? height : width) /
								Math.max(1, labels.length);
							const barSize =
								(groupSize * 0.8) /
								(chart.type === "stacked" ? 1 : Math.max(1, series.length));
							const offset =
								chart.type === "stacked" ? 0 : seriesIndex * barSize;
							const position = group * groupSize + groupSize * 0.1 + offset;
							return (
								<rect
									key={point.id}
									x={
										chart.type === "horizontal"
											? Math.min(horizontalX(start), horizontalX(end))
											: left + position
									}
									y={
										chart.type === "horizontal"
											? top + position
											: Math.min(y(start), y(end))
									}
									width={
										chart.type === "horizontal"
											? Math.abs(horizontalX(end) - horizontalX(start))
											: Math.max(
													0,
													barSize - (chart.type === "stacked" ? 0 : 1),
												)
									}
									height={
										chart.type === "horizontal"
											? Math.max(0, barSize - 1)
											: Math.abs(y(end) - y(start))
									}
									rx={chart.type === "stacked" ? 1 : 2}
									fill={colors[seriesIndex % colors.length]}
								>
									<title>{`${point.label}, ${point.series}: ${point.formattedValue}`}</title>
								</rect>
							);
						})}
			</svg>
			{!compact && series.length > 1 && <ChartLegend labels={series} />}
		</div>
	);
}

function PagePreview({
	node,
	compact,
}: { node: NativeWidgetPageNode; compact: boolean }) {
	const tone = {
		default: "",
		muted: "text-[var(--widget-secondary)]",
		accent: "text-[var(--widget-accent)]",
		success: "text-emerald-700 dark:text-emerald-300",
		warning: "text-amber-700 dark:text-amber-300",
		danger: "text-red-700 dark:text-red-300",
	}[node.tone ?? "default"];
	const children = node.children?.map((child) => (
		<PagePreview key={child.id} node={child} compact={compact} />
	));
	const alignment =
		node.alignment === "center"
			? "center"
			: node.alignment === "trailing"
				? "flex-end"
				: "flex-start";
	const textAlign =
		node.alignment === "trailing"
			? "right"
			: node.alignment === "center"
				? "center"
				: "left";
	if (["column", "row", "grid", "stack", "card"].includes(node.kind))
		return (
			<div
				className={cn(
					"min-w-0",
					node.kind === "row"
						? "flex items-start"
						: node.kind === "grid" || node.kind === "stack"
							? "grid"
							: "flex flex-col",
					node.kind === "stack" && "[&>*]:col-start-1 [&>*]:row-start-1",
					node.kind === "card" &&
						"w-full rounded-xl bg-[var(--widget-inset)] p-2.5",
					tone,
				)}
				style={{
					gap: node.spacing ?? 8,
					...(node.kind === "grid"
						? {
								gridTemplateColumns: `repeat(${Math.min(compact ? 2 : 6, node.columns ?? 2)}, minmax(0, 1fr))`,
								justifyItems:
									alignment === "flex-end"
										? "end"
										: alignment === "flex-start"
											? "start"
											: "center",
							}
						: node.kind === "stack"
							? { alignItems: "start", justifyItems: "start" }
							: node.kind === "row"
								? {}
								: { alignItems: alignment }),
					textAlign,
				}}
			>
				{node.kind === "card" && node.text && (
					<span className="line-clamp-2 text-sm font-semibold">
						{node.text}
					</span>
				)}
				{node.kind === "card" && node.value && (
					<span className="line-clamp-2 text-[10px] text-[var(--widget-secondary)]">
						{node.value}
					</span>
				)}
				{children}
			</div>
		);
	if (node.kind === "divider")
		return <hr className="w-full border-[var(--widget-border)]" />;
	if (node.kind === "spacer")
		return (
			<div
				className="shrink-0"
				style={{ height: Math.max(1, node.spacing ?? 8) }}
			/>
		);
	if (node.kind === "image" || node.kind === "icon") {
		const source = node.image?.png
			? `data:image/png;base64,${node.image.png}`
			: undefined;
		return (
			<div
				className={cn(
					"flex min-h-6 min-w-0 items-center",
					node.kind === "image" && "w-full",
					tone,
				)}
			>
				{node.image?.text ? (
					<span className={node.kind === "icon" ? "text-2xl" : "text-4xl"}>
						{node.image.text}
					</span>
				) : source ? (
					node.image?.template ? (
						<span
							role="img"
							aria-label={node.text ?? ""}
							className={
								node.kind === "icon"
									? "size-7 shrink-0 bg-current"
									: "block h-16 w-full bg-current"
							}
							style={{
								mask: `url("${source}") center / contain no-repeat`,
								WebkitMask: `url("${source}") center / contain no-repeat`,
							}}
						/>
					) : (
						<img
							src={source}
							className={
								node.kind === "icon"
									? "size-7 object-contain"
									: cn(
											"max-w-full rounded-lg object-contain",
											compact ? "max-h-16" : "max-h-28",
										)
							}
							alt={node.text ?? ""}
						/>
					)
				) : (
					<ImageIcon className="size-6 text-[var(--widget-secondary)]" />
				)}
			</div>
		);
	}
	if (node.kind === "progress")
		return (
			<div className="w-full space-y-1.5">
				{node.text && <div className="truncate text-xs">{node.text}</div>}
				<div className="flex items-center gap-2">
					<ProgressMeter value={node.progress ?? 0} />
					{node.value && (
						<span className="shrink-0 text-[10px]">{node.value}</span>
					)}
				</div>
			</div>
		);
	if (node.kind === "chart" && node.chart)
		return (
			<div className={cn("w-full", compact ? "h-[76px]" : "h-32")}>
				<ChartPreview chart={node.chart} compact={compact} />
			</div>
		);
	if (node.kind === "table" && node.table)
		return (
			<table className="w-full table-fixed text-left text-[10px]">
				<thead>
					<tr>
						{node.table.columns.map((column, index) => (
							<th
								key={`${index}:${column}`}
								className="truncate pb-1 font-medium text-[var(--widget-secondary)]"
							>
								{column}
							</th>
						))}
					</tr>
				</thead>
				<tbody>
					{node.table.rows.slice(0, compact ? 3 : 8).map((row, index) => (
						<tr key={`${index}:${row.join("|")}`}>
							{row.map((value, cell) => (
								<td key={`${cell}:${value}`} className="truncate py-0.5">
									{value}
								</td>
							))}
						</tr>
					))}
				</tbody>
			</table>
		);
	if (node.kind === "link")
		return (
			<div className="flex min-w-0 items-center gap-1 text-xs font-semibold text-[var(--widget-accent)]">
				<span className="truncate">{node.text ?? "Open app"}</span>
				{node.action && <ArrowUpRightIcon className="size-3 shrink-0" />}
			</div>
		);
	return (
		<div
			className={cn(
				"min-w-0 whitespace-pre-wrap break-words",
				compact ? "line-clamp-3" : "line-clamp-6",
				node.kind === "badge" &&
					"w-fit rounded-full bg-current/10 px-2 py-1 text-[10px] font-semibold",
				node.role === "title"
					? compact
						? "text-xl font-semibold"
						: "text-2xl font-semibold"
					: node.role === "headline"
						? "text-sm font-semibold"
						: node.role === "caption"
							? "text-[10px]"
							: "text-xs",
				tone,
			)}
			style={{ textAlign }}
		>
			{node.text}
		</div>
	);
}

export function NativeWidgetPreview({
	widget,
	family,
}: { widget: NativeCustomWidget; family: "small" | "medium" | "large" }) {
	const { resolvedTheme } = useTheme();
	const dark = resolvedTheme === "dark";
	const compact = family === "small";
	return (
		<div
			className="mx-auto flex max-w-full flex-col gap-2 overflow-hidden rounded-[28px] border border-[var(--widget-border)] bg-[var(--widget-surface)] p-4 text-[var(--widget-ink)] shadow-sm"
			style={
				{
					width: compact ? 180 : 360,
					height: family === "large" ? 380 : 180,
					"--widget-accent": accents[widget.accent ?? "orange"][dark ? 1 : 0],
					"--widget-surface": dark ? "#17191d" : "#ffffff",
					"--widget-ink": dark ? "#f4f5f7" : "#181b20",
					"--widget-secondary": dark ? "#a1a8b2" : "#626a75",
					"--widget-inset": dark ? "#22262d" : "#f4f6f8",
					"--widget-border": dark ? "#30343c" : "#e8ebef",
				} as CSSProperties
			}
		>
			<div className="flex shrink-0 items-center justify-between gap-2">
				<span className="truncate text-sm font-semibold">{widget.title}</span>
				<img alt="" src="/flow-like.svg" className="size-5 shrink-0" />
			</div>
			<div className="min-h-0 flex-1 overflow-hidden">
				{widget.state === "ready" && widget.chart ? (
					<ChartPreview chart={widget.chart} compact={compact} />
				) : widget.state === "ready" && widget.page ? (
					<PagePreview node={widget.page} compact={compact} />
				) : (
					<div className="flex h-full items-center text-xs text-[var(--widget-secondary)]">
						{widget.message ??
							(widget.state === "empty"
								? "No data yet."
								: "Open the app to update this widget.")}
					</div>
				)}
			</div>
			<div className="flex shrink-0 items-center gap-1 text-[9px] text-[var(--widget-secondary)]">
				<span className="truncate">
					{Date.parse(widget.staleAt) <= Date.now()
						? "Last saved "
						: "Updated "}
					{new Date(widget.updatedAt).toLocaleTimeString([], {
						hour: "2-digit",
						minute: "2-digit",
					})}
				</span>
				<span className="flex-1" />
				{Boolean(widget.warnings?.length) && (
					<CircleAlertIcon
						className="size-3 shrink-0"
						aria-label="Some content is unavailable"
					/>
				)}
				<ArrowUpRightIcon className="size-3 shrink-0" />
			</div>
		</div>
	);
}
