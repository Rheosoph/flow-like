"use client";

import { useTranslation } from "@flow-like/locales";
import { type PointerEvent, type ReactNode, useMemo, useState } from "react";
import { useAreaTime } from "./area-context";
import { cx } from "./tone";

const W = 240;
const H = 40;

export interface SparklineProps {
	/** Oldest first; the last point is the value shown above the line. */
	series: readonly number[];
	/** The metric's name for the accessible summary ("CPU"). */
	label: string;
	/** Fixed y-domain (state it in the axis, e.g. "max 100 %"); the series range otherwise. */
	min?: number;
	max?: number;
	/** Value text with unit ("23.4 %"). */
	format?: (value: number) => string;
	/** Unix seconds of the first point, for the hover time. */
	startAt?: number;
	/** Seconds between points. */
	stepSec?: number;
	/** "the last 30 minutes". */
	span?: string;
	className?: string;
}

/** SPEC §4.15: inline SVG, no chart library. Hover shows a crosshair and "13:42 · 26.1 %". */
export function Sparkline({
	series,
	label,
	min,
	max,
	format = (value) => value.toFixed(1),
	startAt,
	stepSec = 60,
	span,
	className,
}: Readonly<SparklineProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [hover, setHover] = useState<number | null>(null);
	const points = useMemo(
		() => series.filter((value) => Number.isFinite(value)),
		[series],
	);
	const lo = min ?? Math.min(...points);
	const hi = max ?? Math.max(...points);
	const n = points.length;

	const geometry = useMemo(() => {
		if (n === 0) return null;
		const y = (value: number) =>
			H - 3 - ((value - lo) / (hi - lo || 1)) * (H - 6);
		const x = (index: number) => (n === 1 ? W : (index / (n - 1)) * W);
		const coords = points.map((value, index) => [x(index), y(value)] as const);
		const line = `M${coords.map(([px, py]) => `${px.toFixed(1)},${py.toFixed(1)}`).join("L")}`;
		return { coords, line, area: `${line}L${W},${H}L0,${H}Z` };
	}, [points, n, lo, hi]);

	if (!geometry) return null;

	const last = points[n - 1];
	const summary = t(
		"view.metric.sparkLabel",
		"{{label}} over {{span}}, {{min}} to {{max}}, now {{now}}",
		{
			label,
			span: span ?? t("view.metric.spanDefault", "the shown period"),
			min: format(Math.min(...points)),
			max: format(Math.max(...points)),
			now: format(last),
		},
	);

	const onMove = (event: PointerEvent<HTMLDivElement>) => {
		const rect = event.currentTarget.getBoundingClientRect();
		if (rect.width <= 0) return;
		const share = (event.clientX - rect.left) / rect.width;
		setHover(Math.max(0, Math.min(n - 1, Math.round(share * (n - 1)))));
	};

	const hovered = hover === null ? null : geometry.coords[hover];
	const tip =
		hover === null
			? null
			: startAt === undefined
				? format(points[hover])
				: t("view.metric.tip", "{{time}} · {{value}}", {
						time: time.clock(startAt + hover * stepSec, false),
						value: format(points[hover]),
					});

	return (
		<div
			role="img"
			aria-label={summary}
			data-sparkline=""
			onPointerMove={onMove}
			onPointerLeave={() => setHover(null)}
			className={cx("relative mt-1 h-10", className)}
		>
			<svg
				viewBox={`0 0 ${W} ${H}`}
				preserveAspectRatio="none"
				aria-hidden="true"
				className="block h-10 w-full overflow-visible"
			>
				<line
					className="stroke-hairline"
					x1="0"
					x2={W}
					y1={H - 0.5}
					y2={H - 0.5}
					strokeWidth="1"
					vectorEffect="non-scaling-stroke"
				/>
				<path className="fill-spark-fill stroke-none" d={geometry.area} />
				<path
					className="fill-none stroke-spark"
					d={geometry.line}
					strokeWidth="1.5"
					strokeLinejoin="round"
					strokeLinecap="round"
					vectorEffect="non-scaling-stroke"
				/>
			</svg>
			<i
				aria-hidden
				className="absolute -right-1.25 -mt-1.25 size-2.5 rounded-full border-2 border-card bg-spark"
				style={{
					top: `${((geometry.coords[n - 1][1] / H) * 100).toFixed(1)}%`,
				}}
			/>
			{hovered ? (
				<>
					<i
						aria-hidden
						className="pointer-events-none absolute inset-y-0 w-px bg-border-strong"
						style={{ left: `${((hovered[0] / W) * 100).toFixed(2)}%` }}
					/>
					<span
						data-spark-tip=""
						className="pointer-events-none absolute -top-6 z-10 -translate-x-1/2 rounded-sm border border-border-strong bg-popover px-1.5 py-0.5 font-mono text-xs whitespace-nowrap text-foreground tabular-nums"
						style={{ left: `${((hovered[0] / W) * 100).toFixed(2)}%` }}
					>
						{tip}
					</span>
				</>
			) : null}
		</div>
	);
}

export interface MetricProps {
	label: ReactNode;
	/** Formatted number ("23.4"). */
	value: ReactNode;
	unit?: ReactNode;
	/** "8 logical CPUs · 100 % = all of them". */
	note?: ReactNode;
	stamp?: ReactNode;
	spark?: SparklineProps;
	/** Axis labels under the line ("13:30", "max 100 %", "now"). */
	axis?: readonly ReactNode[];
	className?: string;
}

/** SPEC §4.15: one cell of a metric grid; the last spark point equals `value`. */
export function Metric({
	label,
	value,
	unit,
	note,
	stamp,
	spark,
	axis,
	className,
}: Readonly<MetricProps>) {
	return (
		<div
			data-metric=""
			className={cx(
				"flex min-w-0 flex-col gap-1 border-r border-b border-hairline px-4 py-3",
				className,
			)}
		>
			<div className="flex min-w-0 items-center justify-between gap-2">
				<span className="text-xs font-medium text-ink-2">{label}</span>
				{stamp}
			</div>
			<div className="flex flex-wrap items-baseline gap-1">
				<span className="font-mono text-lg leading-6 font-medium tabular-nums">
					{value}
				</span>
				{unit ? (
					<span className="text-xs text-muted-foreground">{unit}</span>
				) : null}
			</div>
			{note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
			{spark ? <Sparkline {...spark} /> : null}
			{axis?.length ? (
				<div className="flex justify-between gap-1.5 text-xs text-muted-foreground">
					{axis.map((part, index) => (
						// biome-ignore lint/suspicious/noArrayIndexKey: axis labels are positional
						<span key={index}>{part}</span>
					))}
				</div>
			) : null}
		</div>
	);
}

/** Cells divided by hairlines, no outer gaps; columns fill by container width (min 220 px). */
export function MetricGrid({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<div className="min-w-0 overflow-hidden">
			<div
				className={cx(
					"-mr-px -mb-px grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))]",
					className,
				)}
			>
				{children}
			</div>
		</div>
	);
}
