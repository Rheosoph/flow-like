"use client";

import { useTranslation } from "@flow-like/locales";
import { ChartColumn, Table2 } from "lucide-react";
import {
	type FocusEvent,
	type KeyboardEvent,
	type PointerEvent,
	type ReactNode,
	type RefObject,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { type AreaTime, type DevicesT, useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { DvTable, Td, Th, Tr } from "./dv-table";
import { cx } from "./tone";

/*
 * SPEC §4.15 extended for model statistics (plan §3.7): one value axis over
 * time, inline SVG, no chart library. Colour does one job per series
 * (dataviz): `accent` is the series the chart is about, `context` the
 * de-emphasised one beside it, `critical` failures. Accent pairs with either;
 * context and critical never share a chart (they collapse under protanopia).
 * Hover and keyboard show every series of one slot; the table view carries
 * every value without either.
 */

export type ChartTone = "accent" | "context" | "critical";

/** One value per slot, oldest first; `null` = nothing reported (a gap). */
export type ChartValues = ReadonlyArray<number | null>;

export interface ChartSeries {
	id: string;
	/** Legend and tooltip name ("Out"). */
	label: string;
	tone: ChartTone;
	values: ChartValues;
	/** Legend text after the label: the series' total or typical value ("1.2M"). */
	note?: string;
}

export interface TimeSeriesChartProps {
	/** Figure title and the accessible name of the plot ("Tokens"). */
	title: string;
	/** The headline next to the title ("1.7M tokens"). */
	summary?: ReactNode;
	/** `columns` stack their series per slot; `lines` share the value axis. */
	kind: "columns" | "lines";
	series: readonly ChartSeries[];
	/** Unix seconds the first slot starts at. */
	start: number;
	/** Seconds per slot. */
	step: number;
	/** A value with its unit ("640 ms"); the axis uses it unless `formatTick` is given. */
	format: (value: number) => string;
	formatTick?: (value: number) => string;
	/** The last slot is still filling (the current hour): drawn lighter, read as "so far". */
	partialLast?: boolean;
	/** Shown instead of the plot while no slot has a value. */
	emptyText?: ReactNode;
	/** A newer read is under way: the last plot stays, dimmed. */
	busy?: boolean;
	className?: string;
}

const PLOT_H = 120;
const AXIS_H = 20;
const TOP = 6;
const BASE = TOP + PLOT_H;
const GUTTER = 48;
const RIGHT = 8;
const BAR_MAX = 24;
const RADIUS = 4;
const GAP = 2;
const FALLBACK_WIDTH = 560;
const TICK_SPACING = 80;
const EDGE = 24;
const TIP_FLIP = 120;
const DAY = 86_400;
const TICK_EVERY = [
	3_600,
	7_200,
	10_800,
	21_600,
	43_200,
	DAY,
	2 * DAY,
	7 * DAY,
	14 * DAY,
	30 * DAY,
] as const;
const NICE = [1, 2, 5, 10] as const;

const TONE: Record<
	ChartTone,
	{ fill: string; stroke: string; swatch: string; wash: string }
> = {
	accent: {
		fill: "fill-info-solid",
		stroke: "stroke-info-solid",
		swatch: "bg-info-solid",
		wash: "fill-info-solid/10",
	},
	context: {
		fill: "fill-spark",
		stroke: "stroke-spark",
		swatch: "bg-spark",
		wash: "fill-spark-fill",
	},
	critical: {
		fill: "fill-critical-solid",
		stroke: "stroke-critical-solid",
		swatch: "bg-critical-solid",
		wash: "fill-critical-solid/10",
	},
};

/** The fill of a tone for HTML marks beside a chart (inline bars, keys). */
export const CHART_FILL: Record<ChartTone, string> = {
	accent: TONE.accent.swatch,
	context: TONE.context.swatch,
	critical: TONE.critical.swatch,
};

/* Scales. */

const valueAt = (series: ChartSeries, slot: number) =>
	series.values[slot] ?? null;

const slotCount = (series: readonly ChartSeries[]) => {
	let count = 0;
	for (const entry of series) count = Math.max(count, entry.values.length);
	return count;
};

/** Stacked total of one slot; gaps count as zero. */
const stackedAt = (series: readonly ChartSeries[], slot: number) => {
	let total = 0;
	for (const entry of series) total += Math.max(0, valueAt(entry, slot) ?? 0);
	return total;
};

const highestAt = (series: readonly ChartSeries[], slot: number) => {
	let high = 0;
	for (const entry of series) high = Math.max(high, valueAt(entry, slot) ?? 0);
	return high;
};

/** The largest value the plot must hold: the stacked totals of columns, the highest point of lines. */
export function chartMax(
	kind: TimeSeriesChartProps["kind"],
	series: readonly ChartSeries[],
) {
	const of = kind === "columns" ? stackedAt : highestAt;
	let max = 0;
	for (let slot = 0; slot < slotCount(series); slot++)
		max = Math.max(max, of(series, slot));
	return max;
}

/** Whether any slot has something to draw: a value above zero for columns, any value for lines. */
export function chartHasData(
	kind: TimeSeriesChartProps["kind"],
	series: readonly ChartSeries[],
) {
	const floor = kind === "lines" ? Number.NEGATIVE_INFINITY : 0;
	for (const entry of series)
		for (const value of entry.values)
			if (value !== null && value > floor) return true;
	return false;
}

const clean = (value: number) => Number(value.toPrecision(12));

/** Gridline values from 0 to a 1-2-5 top at or above `max`, at most `intervals` steps. */
export function valueTicks(max: number, intervals = 3) {
	if (!(max > 0)) return [0, 1];
	const raw = max / intervals;
	const magnitude = 10 ** Math.floor(Math.log10(raw));
	let step = 10 * magnitude;
	for (const nice of NICE)
		if (nice * magnitude >= raw) {
			step = nice * magnitude;
			break;
		}
	const ticks = [0];
	while (ticks[ticks.length - 1] < max) ticks.push(clean(ticks.length * step));
	return ticks;
}

const localOffset = (atS: number) =>
	-new Date(atS * 1000).getTimezoneOffset() * 60;

/** Clean local times (whole hours, midnights) from `from` to `to`, at most `max` of them. */
export function timeTicks(
	from: number,
	to: number,
	max: number,
	offsetOf: (atS: number) => number = localOffset,
) {
	const span = to - from;
	let every: number = TICK_EVERY[TICK_EVERY.length - 1];
	for (const candidate of TICK_EVERY)
		if (span / candidate <= max) {
			every = candidate;
			break;
		}
	const offset = offsetOf(from);
	const ticks: number[] = [];
	for (
		let at = Math.ceil((from + offset) / every) * every - offset;
		at <= to;
		at += every
	)
		ticks.push(at);
	return { every, ticks };
}

/* Geometry. */

interface Frame {
	width: number;
	/** Pixels per slot. */
	slot: number;
	slots: number;
	/** The value at the top gridline. */
	max: number;
}

function frameOf(width: number, slots: number, max: number): Frame {
	const right = Math.max(GUTTER + 1, width - RIGHT);
	return { width, slot: (right - GUTTER) / Math.max(1, slots), slots, max };
}

const yOf = (frame: Frame, value: number) =>
	BASE - (Math.max(0, value) / frame.max) * PLOT_H;

const xOf = (frame: Frame, slot: number) => GUTTER + (slot + 0.5) * frame.slot;

const px = (value: number) => value.toFixed(1);

/** The slot under a horizontal position in the plot, or `null` outside it. */
function slotAt(frame: Frame, x: number) {
	const slot = Math.floor((x - GUTTER) / frame.slot);
	return slot < 0 || slot >= frame.slots ? null : slot;
}

function columnPath(x: number, w: number, top: number, bottom: number) {
	return `M${px(x)},${px(bottom)}V${px(top)}H${px(x + w)}V${px(bottom)}Z`;
}

/** A column segment with a 4 px rounded data-end, square at the baseline. */
function roundedPath(x: number, w: number, top: number, bottom: number) {
	const r = Math.min(RADIUS, w / 2, bottom - top);
	if (r <= 0) return columnPath(x, w, top, bottom);
	return `M${px(x)},${px(bottom)}V${px(top + r)}A${r},${r} 0 0 1 ${px(x + r)},${px(top)}H${px(x + w - r)}A${r},${r} 0 0 1 ${px(x + w)},${px(top + r)}V${px(bottom)}Z`;
}

interface Segment {
	tone: ChartTone;
	id: string;
	d: string;
}

const topSeries = (series: readonly ChartSeries[], slot: number) => {
	let top = -1;
	for (const [index, entry] of series.entries())
		if ((valueAt(entry, slot) ?? 0) > 0) top = index;
	return top;
};

/** One slot's stack, bottom first; a 2 px surface gap is carved from the top of every lower segment. */
function stackOf(frame: Frame, series: readonly ChartSeries[], slot: number) {
	const gap = frame.slot >= 6 ? GAP : 1;
	const width = Math.max(1, Math.min(BAR_MAX, frame.slot - gap));
	const x = xOf(frame, slot) - width / 2;
	const last = topSeries(series, slot);
	const segments: Segment[] = [];
	let total = 0;
	for (const [index, entry] of series.entries()) {
		const value = Math.max(0, valueAt(entry, slot) ?? 0);
		if (value <= 0) continue;
		const bottom = yOf(frame, total);
		total += value;
		const top = yOf(frame, total);
		segments.push({
			tone: entry.tone,
			id: entry.id,
			d:
				index === last
					? roundedPath(x, width, top, bottom)
					: columnPath(x, width, Math.min(bottom - 1, top + GAP), bottom),
		});
	}
	return segments;
}

type Point = readonly [number, number];

/** Runs of reported points between gaps. */
function runsOf(frame: Frame, values: ChartValues) {
	const runs: Point[][] = [];
	let run: Point[] = [];
	for (const [slot, value] of values.entries()) {
		if (value === null) {
			if (run.length) runs.push(run);
			run = [];
			continue;
		}
		run.push([xOf(frame, slot), yOf(frame, value)]);
	}
	if (run.length) runs.push(run);
	return runs;
}

const pointText = ([x, y]: Point) => `${px(x)},${px(y)}`;

/** A lone point is a zero-length run: the round cap draws it as a dot. */
const lineOf = (run: readonly Point[]) =>
	run.length === 1
		? `M${pointText(run[0])}l0,0`
		: `M${run.map(pointText).join("L")}`;

const washOf = (run: readonly Point[]) =>
	`M${px(run[0][0])},${BASE}L${run.map(pointText).join("L")}L${px(run[run.length - 1][0])},${BASE}Z`;

function lastPoint(values: ChartValues) {
	for (let slot = values.length - 1; slot >= 0; slot--) {
		const value = values[slot];
		if (value !== null && value !== undefined) return { slot, value };
	}
	return undefined;
}

/* Copy. */

const dayFormats = new Map<string, Intl.DateTimeFormat>();

function dayText(locale: string, atS: number) {
	let format = dayFormats.get(locale);
	if (!format) {
		format = new Intl.DateTimeFormat(locale, {
			day: "numeric",
			month: "short",
		});
		dayFormats.set(locale, format);
	}
	return format.format(atS * 1000);
}

const sameDay = (a: number, b: number) =>
	new Date(a * 1000).toDateString() === new Date(b * 1000).toDateString();

/** "3 Oct, 14:00–15:00", "2 Oct, 15:00 – 3 Oct, 15:00"; the slot still filling ends "(so far)". */
function spanText(
	t: DevicesT,
	time: AreaTime,
	slot: { from: number; to: number; partial: boolean },
) {
	const span = sameDay(slot.from, slot.to - 1)
		? t("devices:view.chart.spanSameDay", "{{from}}–{{to}}", {
				from: time.at(slot.from),
				to: time.clock(slot.to, false),
			})
		: t("devices:view.chart.span", "{{from}} – {{to}}", {
				from: time.at(slot.from),
				to: time.at(slot.to),
			});
	return slot.partial
		? t("devices:view.chart.soFar", "{{span}} (so far)", { span })
		: span;
}

interface Readout {
	at: number;
	span: string;
	rows: { id: string; tone: ChartTone; label: string; value: string }[];
}

type ReadoutProps = Pick<
	TimeSeriesChartProps,
	"series" | "start" | "step" | "format" | "partialLast"
>;

function readoutOf(
	t: DevicesT,
	time: AreaTime,
	props: ReadoutProps,
	slot: number,
): Readout {
	const from = props.start + slot * props.step;
	const partial =
		props.partialLast === true && slot === slotCount(props.series) - 1;
	const cell = (entry: ChartSeries) => {
		const value = valueAt(entry, slot);
		return {
			id: entry.id,
			tone: entry.tone,
			label: entry.label,
			value: value === null ? "–" : props.format(value),
		};
	};
	return {
		at: from,
		span: spanText(t, time, { from, to: from + props.step, partial }),
		rows: props.series.map(cell),
	};
}

const readoutText = (t: DevicesT, readout: Readout) =>
	t("devices:view.chart.readout", "{{span}}: {{values}}", {
		span: readout.span,
		values: readout.rows.map((row) => `${row.label} ${row.value}`).join(", "),
	});

/* Parts. */

function useWidth(ref: RefObject<HTMLElement | null>) {
	const [width, setWidth] = useState(FALLBACK_WIDTH);
	useEffect(() => {
		const element = ref.current;
		if (!element || typeof ResizeObserver === "undefined") return;
		const measure = () => {
			if (element.clientWidth > 0) setWidth(element.clientWidth);
		};
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(element);
		return () => observer.disconnect();
	}, [ref]);
	return width;
}

type Key = "box" | "line";

function Swatch({ tone, shape }: Readonly<{ tone: ChartTone; shape: Key }>) {
	return (
		<i
			aria-hidden
			className={cx(
				"inline-block shrink-0",
				shape === "box" ? "size-2.5 rounded-[2px]" : "h-0.5 w-3 rounded-full",
				TONE[tone].swatch,
			)}
		/>
	);
}

/** Two or more series: the legend mirrors the mark (box for columns, line for lines). */
function Legend({
	kind,
	series,
}: Readonly<{
	kind: TimeSeriesChartProps["kind"];
	series: readonly ChartSeries[];
}>) {
	if (series.length < 2) return null;
	return (
		<ul data-legend="" className="flex flex-wrap gap-x-3.5 gap-y-1 text-xs">
			{series.map((entry) => (
				<li key={entry.id} className="inline-flex items-center gap-1.5">
					<Swatch
						tone={entry.tone}
						shape={kind === "columns" ? "box" : "line"}
					/>
					<span className="text-ink-2">{entry.label}</span>
					{entry.note ? (
						<span className="font-medium tabular-nums">{entry.note}</span>
					) : null}
				</li>
			))}
		</ul>
	);
}

function ValueAxis({
	frame,
	ticks,
	label,
}: Readonly<{
	frame: Frame;
	ticks: readonly number[];
	label: (value: number) => string;
}>) {
	return (
		<g>
			{ticks.map((tick) => {
				const y = yOf(frame, tick) - 0.5;
				return (
					<g key={tick}>
						<line
							x1={GUTTER}
							x2={frame.width - RIGHT}
							y1={px(y)}
							y2={px(y)}
							strokeWidth="1"
							className={
								tick === 0 ? "stroke-border-strong" : "stroke-hairline"
							}
						/>
						<text
							x={GUTTER - 6}
							y={px(y + 4)}
							textAnchor="end"
							className="fill-muted-foreground text-[11px] tabular-nums"
						>
							{label(tick)}
						</text>
					</g>
				);
			})}
		</g>
	);
}

const anchorAt = (x: number, width: number) => {
	if (x < GUTTER + EDGE) return "start";
	return x > width - RIGHT - EDGE ? "end" : "middle";
};

function TimeAxis({
	frame,
	start,
	end,
}: Readonly<{ frame: Frame; start: number; end: number }>) {
	const time = useAreaTime();
	const usable = frame.width - GUTTER - RIGHT;
	const { every, ticks } = timeTicks(
		start,
		end,
		Math.max(2, Math.floor(usable / TICK_SPACING)),
	);
	const label = (at: number) =>
		every < DAY ? time.clock(at, false) : dayText(time.locale, at);
	return (
		<g data-time-axis="">
			{ticks.map((at) => {
				const x = GUTTER + ((at - start) / Math.max(1, end - start)) * usable;
				return (
					<text
						key={at}
						x={px(x)}
						y={BASE + AXIS_H - 5}
						textAnchor={anchorAt(x, frame.width)}
						className="fill-muted-foreground text-[11px] tabular-nums"
					>
						{label(at)}
					</text>
				);
			})}
		</g>
	);
}

function Columns({
	frame,
	series,
	partialLast,
}: Readonly<{
	frame: Frame;
	series: readonly ChartSeries[];
	partialLast: boolean;
}>) {
	const slots = useMemo(() => {
		const all: { slot: number; segments: Segment[]; partial: boolean }[] = [];
		for (let slot = 0; slot < frame.slots; slot++)
			all.push({
				slot,
				segments: stackOf(frame, series, slot),
				partial: partialLast && slot === frame.slots - 1,
			});
		return all;
	}, [frame, series, partialLast]);
	return (
		<g>
			{slots.map((column) => (
				<g
					key={column.slot}
					data-column={column.slot}
					opacity={column.partial ? 0.5 : undefined}
				>
					{column.segments.map((segment) => (
						<path
							key={segment.id}
							data-series={segment.id}
							d={segment.d}
							className={TONE[segment.tone].fill}
						/>
					))}
				</g>
			))}
		</g>
	);
}

function LineSeries({
	frame,
	entry,
	wash,
}: Readonly<{ frame: Frame; entry: ChartSeries; wash: boolean }>) {
	const runs = useMemo(() => runsOf(frame, entry.values), [frame, entry]);
	const end = lastPoint(entry.values);
	const tone = TONE[entry.tone];
	return (
		<g data-series={entry.id}>
			{wash
				? runs.map((run) => (
						<path
							key={run[0][0]}
							d={washOf(run)}
							className={cx(tone.wash, "stroke-none")}
						/>
					))
				: null}
			<path
				d={runs.map(lineOf).join("")}
				fill="none"
				strokeWidth="2"
				strokeLinejoin="round"
				strokeLinecap="round"
				className={tone.stroke}
			/>
			{end ? (
				<circle
					cx={px(xOf(frame, end.slot))}
					cy={px(yOf(frame, end.value))}
					r="4"
					strokeWidth="2"
					className={cx(tone.fill, "stroke-card")}
				/>
			) : null}
		</g>
	);
}

/** Columns: a band behind the hovered slot. */
function Band({ frame, slot }: Readonly<{ frame: Frame; slot: number }>) {
	return (
		<rect
			data-cursor={slot}
			x={px(GUTTER + slot * frame.slot)}
			y={TOP}
			width={px(frame.slot)}
			height={PLOT_H}
			className="fill-row-hover"
		/>
	);
}

/** Lines: a crosshair at the slot with every series' point on it. */
function Crosshair({
	frame,
	series,
	slot,
}: Readonly<{ frame: Frame; series: readonly ChartSeries[]; slot: number }>) {
	const x = px(xOf(frame, slot));
	const dot = (entry: ChartSeries) => {
		const value = valueAt(entry, slot);
		return value === null ? null : (
			<circle
				key={entry.id}
				cx={x}
				cy={px(yOf(frame, value))}
				r="4"
				strokeWidth="2"
				className={cx(TONE[entry.tone].fill, "stroke-card")}
			/>
		);
	};
	return (
		<g data-cursor={slot}>
			<line
				x1={x}
				x2={x}
				y1={TOP}
				y2={BASE}
				strokeWidth="1"
				className="stroke-border-strong"
			/>
			{series.map(dot)}
		</g>
	);
}

const tipSide = (x: number, width: number) => {
	if (x < TIP_FLIP) return "translate-x-0";
	return x > width - TIP_FLIP ? "-translate-x-full" : "-translate-x-1/2";
};

/** One readout for every series at the slot: values lead, names follow, line keys. */
function Tooltip({
	frame,
	slot,
	readout,
}: Readonly<{ frame: Frame; slot: number; readout: Readout }>) {
	const x = xOf(frame, slot);
	return (
		<div
			aria-hidden
			data-chart-tip=""
			className={cx(
				"pointer-events-none absolute top-0 z-10 flex min-w-36 flex-col gap-1 rounded-md border border-border-strong bg-popover px-2 py-1.5 text-xs whitespace-nowrap text-foreground",
				tipSide(x, frame.width),
			)}
			style={{ left: `${px((x / frame.width) * 100)}%` }}
		>
			<span className="text-muted-foreground">{readout.span}</span>
			{readout.rows.map((row) => (
				<span key={row.id} className="flex items-center gap-1.5">
					<Swatch tone={row.tone} shape="line" />
					<span className="font-medium tabular-nums">{row.value}</span>
					<span className="text-muted-foreground">{row.label}</span>
				</span>
			))}
		</div>
	);
}

/** The slot a key moves the cursor to; `null` clears it, `undefined` leaves the key alone. */
function keyStep(key: string, current: number | null, count: number) {
	const next = Math.min(count - 1, (current ?? -1) + 1);
	const previous = Math.max(0, (current ?? count) - 1);
	const moves: Record<string, number | null> = {
		ArrowRight: next,
		ArrowUp: next,
		ArrowLeft: previous,
		ArrowDown: previous,
		Home: 0,
		End: count - 1,
		Escape: null,
	};
	return Object.hasOwn(moves, key) ? moves[key] : undefined;
}

function usePlotGeometry(props: TimeSeriesChartProps, width: number) {
	const { kind, series, format, formatTick } = props;
	return useMemo(() => {
		const ticks = valueTicks(chartMax(kind, series));
		const frame = frameOf(width, slotCount(series), ticks[ticks.length - 1]);
		return { ticks, frame, label: formatTick ?? format };
	}, [kind, series, width, format, formatTick]);
}

/** The hovered or keyboard-chosen slot. */
function useCursor(frame: Frame, width: number) {
	const [slot, setSlot] = useState<number | null>(null);
	const handlers = {
		onPointerMove: (event: PointerEvent<HTMLDivElement>) => {
			const rect = event.currentTarget.getBoundingClientRect();
			if (rect.width > 0)
				setSlot(
					slotAt(frame, ((event.clientX - rect.left) / rect.width) * width),
				);
		},
		onPointerLeave: () => setSlot(null),
		onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
			const next = keyStep(event.key, slot, frame.slots);
			if (next === undefined) return;
			event.preventDefault();
			setSlot(next);
		},
		onFocus: (event: FocusEvent<HTMLDivElement>) => {
			if (event.target === event.currentTarget) setSlot(frame.slots - 1);
		},
		onBlur: () => setSlot(null),
	};
	return { slot, handlers };
}

function Plot({
	props,
	labelId,
}: Readonly<{ props: TimeSeriesChartProps; labelId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const ref = useRef<HTMLDivElement>(null);
	const width = useWidth(ref);
	const { ticks, frame, label } = usePlotGeometry(props, width);
	const { slot, handlers } = useCursor(frame, width);
	const shown = Math.max(0, slot ?? frame.slots - 1);
	const readout = readoutOf(t, time, props, shown);
	const columns = props.kind === "columns";
	return (
		<div
			ref={ref}
			role="slider"
			tabIndex={0}
			aria-labelledby={labelId}
			aria-valuemin={0}
			aria-valuemax={Math.max(0, frame.slots - 1)}
			aria-valuenow={shown}
			aria-valuetext={readoutText(t, readout)}
			data-plot=""
			{...handlers}
			className="relative touch-pan-y rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
		>
			<svg
				width="100%"
				height={BASE + AXIS_H}
				viewBox={`0 0 ${width} ${BASE + AXIS_H}`}
				aria-hidden="true"
				className="block overflow-visible"
			>
				{columns && slot !== null ? <Band frame={frame} slot={slot} /> : null}
				<ValueAxis frame={frame} ticks={ticks} label={label} />
				{columns ? (
					<Columns
						frame={frame}
						series={props.series}
						partialLast={props.partialLast === true}
					/>
				) : (
					props.series.map((entry) => (
						<LineSeries
							key={entry.id}
							frame={frame}
							entry={entry}
							wash={props.series.length === 1}
						/>
					))
				)}
				{!columns && slot !== null ? (
					<Crosshair frame={frame} series={props.series} slot={slot} />
				) : null}
				<TimeAxis
					frame={frame}
					start={props.start}
					end={props.start + frame.slots * props.step}
				/>
			</svg>
			{slot === null ? null : (
				<Tooltip frame={frame} slot={slot} readout={readout} />
			)}
		</div>
	);
}

/** Local calendar day of an epoch-milliseconds instant. */
const dayKey = (atMs: number) => new Date(atMs).toDateString();

/**
 * Every slot as a row. A span reads the area clock only for its date ("11:00"
 * today, "29 Sept, 11:00" before), so the rows are built again when the
 * locale or the day changes, not on every tick of the clock.
 */
function ChartTable({ props }: Readonly<{ props: TimeSeriesChartProps }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const day = dayKey(time.now);
	const timeLabel = t("devices:view.chart.time", "Time");
	const { series, start, step, format, partialLast } = props;
	// biome-ignore lint/correctness/useExhaustiveDependencies: `day` and `time.locale` are all the spans read of `time`
	const body = useMemo(() => {
		const shape: ReadoutProps = { series, start, step, format, partialLast };
		const rows: ReactNode[] = [];
		for (let slot = 0; slot < slotCount(series); slot++) {
			const row = readoutOf(t, time, shape, slot);
			rows.push(
				<Tr key={row.at}>
					<Td label={timeLabel}>{row.span}</Td>
					{row.rows.map((cell) => (
						<Td key={cell.id} label={cell.label} kind="num">
							{cell.value}
						</Td>
					))}
				</Tr>,
			);
		}
		return rows;
	}, [
		t,
		time.locale,
		day,
		timeLabel,
		series,
		start,
		step,
		format,
		partialLast,
	]);
	const columnPlan = useMemo(
		() => ["auto", ...series.map(() => "24%")],
		[series],
	);
	return (
		<div className="max-h-72 overflow-auto rounded-md border border-hairline">
			<DvTable
				label={props.title}
				cols={columnPlan}
				stackAt={false}
				head={
					<tr>
						<Th>{timeLabel}</Th>
						{series.map((entry) => (
							<Th key={entry.id} numeric>
								{entry.label}
							</Th>
						))}
					</tr>
				}
			>
				{body}
			</DvTable>
		</div>
	);
}

/** Names the view it switches to; no `aria-pressed`, whose label would have to stay fixed. */
function ViewToggle({
	table,
	onChange,
}: Readonly<{ table: boolean; onChange: (table: boolean) => void }>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			size="xs"
			variant="ghost"
			icon={table ? ChartColumn : Table2}
			onClick={() => onChange(!table)}
		>
			{table
				? t("devices:view.chart.showChart", "Show chart")
				: t("devices:view.chart.showTable", "Show table")}
		</DvButton>
	);
}

/**
 * A titled time-series figure: stacked columns or lines on one value axis, a
 * legend for two or more series, a crosshair readout on hover and on keyboard
 * focus (arrows move it) and a table view of every value.
 */
export function TimeSeriesChart(props: Readonly<TimeSeriesChartProps>) {
	const labelId = useId();
	const [table, setTable] = useState(false);
	const empty = !chartHasData(props.kind, props.series);
	return (
		<figure
			data-chart={props.kind}
			aria-labelledby={labelId}
			aria-busy={props.busy || undefined}
			className={cx(
				"m-0 flex min-w-0 flex-col gap-2 transition-opacity",
				props.busy && "opacity-60",
				props.className,
			)}
		>
			<figcaption className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
				<span id={labelId} className="text-xs font-medium text-ink-2">
					{props.title}
				</span>
				{props.summary ? (
					<span className="text-sm font-medium">{props.summary}</span>
				) : null}
				<span className="flex-1" />
				{empty ? null : <ViewToggle table={table} onChange={setTable} />}
			</figcaption>
			{empty ? (
				<p className="flex h-36 items-center justify-center rounded-md border border-dashed border-hairline px-4 text-center text-xs text-muted-foreground">
					{props.emptyText}
				</p>
			) : (
				<>
					<Legend kind={props.kind} series={props.series} />
					{table ? (
						<ChartTable props={props} />
					) : (
						<Plot props={props} labelId={labelId} />
					)}
				</>
			)}
		</figure>
	);
}
