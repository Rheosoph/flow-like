"use client";

import { useTranslation } from "@flow-like/locales";
import { type PointerEvent, type ReactNode, useMemo, useState } from "react";
import { useAreaTime } from "./area-context";
import { cx } from "./tone";

const W = 240;
const H = 40;

interface SparkPoint {
	/** Position in `series`: a gap keeps its slot, so later points keep their place and time. */
	slot: number;
	value: number;
}

const sparkPoints = (series: readonly number[]) =>
	series.flatMap((value, slot): SparkPoint[] =>
		Number.isFinite(value) ? [{ slot, value }] : [],
	);

function sparkGeometry(
	points: readonly SparkPoint[],
	slots: number,
	min: number | undefined,
	max: number | undefined,
) {
	if (!points.length) return null;
	const values = points.map((point) => point.value);
	const low = Math.min(...values);
	const high = Math.max(...values);
	const lo = min ?? low;
	const span = (max ?? high) - lo || 1;
	const coords = points.map(
		(point) =>
			[
				slots <= 1 ? W : (point.slot / (slots - 1)) * W,
				H - 3 - ((point.value - lo) / span) * (H - 6),
			] as const,
	);
	const line = `M${coords.map(([px, py]) => `${px.toFixed(1)},${py.toFixed(1)}`).join("L")}`;
	const from = coords[0][0].toFixed(1);
	const to = coords[coords.length - 1][0].toFixed(1);
	return { coords, line, area: `${line}L${to},${H}L${from},${H}Z`, low, high };
}

/** Index of the point whose slot is closest to a (fractional) slot under the pointer. */
function nearestPoint(points: readonly SparkPoint[], slot: number) {
	let nearest = 0;
	for (let index = 1; index < points.length; index++) {
		const closer =
			Math.abs(points[index].slot - slot) <
			Math.abs(points[nearest].slot - slot);
		if (closer) nearest = index;
	}
	return nearest;
}

export interface SparklineProps {
	/** Oldest first; the last point is the value shown above the line. A non-finite value is a gap. */
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
	const slots = series.length;
	const points = useMemo(() => sparkPoints(series), [series]);
	const geometry = useMemo(
		() => sparkGeometry(points, slots, min, max),
		[points, slots, min, max],
	);

	if (!geometry) return null;

	const summary = t(
		"view.metric.sparkLabel",
		"{{label}} over {{span}}, {{min}} to {{max}}, now {{now}}",
		{
			label,
			span: span ?? t("view.metric.spanDefault", "the shown period"),
			min: format(geometry.low),
			max: format(geometry.high),
			now: format(points[points.length - 1].value),
		},
	);

	const onMove = (event: PointerEvent<HTMLDivElement>) => {
		const rect = event.currentTarget.getBoundingClientRect();
		if (rect.width <= 0) return;
		const share = (event.clientX - rect.left) / rect.width;
		setHover(nearestPoint(points, share * Math.max(1, slots - 1)));
	};

	const end = geometry.coords[geometry.coords.length - 1];
	const hoverPoint = hover === null ? undefined : points[hover];
	const hovered = hoverPoint ? geometry.coords[hover ?? 0] : undefined;
	const tip = !hoverPoint
		? null
		: startAt === undefined
			? format(hoverPoint.value)
			: t("view.metric.tip", "{{time}} · {{value}}", {
					time: time.clock(startAt + hoverPoint.slot * stepSec, false),
					value: format(hoverPoint.value),
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
				className="absolute -mt-1.25 -ml-1.25 size-2.5 rounded-full border-2 border-card bg-spark"
				style={{
					left: `${((end[0] / W) * 100).toFixed(2)}%`,
					top: `${((end[1] / H) * 100).toFixed(1)}%`,
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
				<span className="max-w-full flex-none truncate text-xs font-medium text-ink-2">
					{label}
				</span>
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
