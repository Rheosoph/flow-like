"use client";

import { useTranslation } from "@flow-like/locales";
import { Activity, History } from "lucide-react";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import type {
	IShowcaseActivity,
	IShowcaseRunDay,
} from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { compact, formatNumber, niceMax } from "./model";
import { ShowcaseSection, cardClass } from "./showcase-ui";

const H = 180;
const PL = 40;
const PR = 8;
const PT = 12;
const PB = 26;

const parseDay = (day: string) => new Date(`${day}T00:00:00Z`);
const shortDay = (day: string) =>
	parseDay(day).toLocaleDateString(undefined, {
		day: "numeric",
		month: "short",
		timeZone: "UTC",
	});
const longDay = (day: string) =>
	parseDay(day).toLocaleDateString(undefined, {
		weekday: "short",
		day: "numeric",
		month: "short",
		timeZone: "UTC",
	});

function useWidth(fallback: number) {
	const ref = useRef<HTMLDivElement>(null);
	const [width, setWidth] = useState(fallback);
	useLayoutEffect(() => {
		const el = ref.current;
		if (!el) return;
		const read = () => {
			if (el.clientWidth > 0) setWidth(el.clientWidth);
		};
		read();
		if (typeof ResizeObserver === "undefined") return;
		const ro = new ResizeObserver(read);
		ro.observe(el);
		return () => ro.disconnect();
	}, []);
	return { ref, width };
}

function RunsChart({ days }: Readonly<{ days: IShowcaseRunDay[] }>) {
	const { t } = useTranslation("store");
	const [hover, setHover] = useState<number | null>(null);
	const { ref, width: W } = useWidth(640);
	const max = niceMax(Math.max(...days.map((d) => d.runs), 0));
	const iw = W - PL - PR;
	const ih = H - PT - PB;
	const step = iw / days.length;
	const bw = Math.max(1, Math.min(step * 0.72, 18));
	const y = (v: number) => PT + ih - (v / max) * ih;

	const bars = useMemo(
		() =>
			days.map((d, i) => {
				const x0 = PL + i * step + (step - bw) / 2;
				const h = (ih * d.runs) / max;
				const r = Math.min(3, h);
				const y0 = PT + ih - h;
				return {
					i,
					x0,
					path: `M${x0},${PT + ih}V${y0 + r}Q${x0},${y0} ${x0 + r},${y0}H${x0 + bw - r}Q${x0 + bw},${y0} ${x0 + bw},${y0 + r}V${PT + ih}Z`,
				};
			}),
		[days, step, ih, max, bw],
	);

	const onMove = useCallback(
		(ev: React.PointerEvent<SVGSVGElement>) => {
			const r = ev.currentTarget.getBoundingClientRect();
			const x = ((ev.clientX - r.left) / r.width) * W;
			const i = Math.floor((x - PL) / step);
			setHover(i >= 0 && i < days.length ? i : null);
		},
		[days.length, step, W],
	);

	const onKey = useCallback(
		(ev: React.KeyboardEvent<SVGSVGElement>) => {
			if (ev.key === "ArrowRight" || ev.key === "ArrowLeft") {
				ev.preventDefault();
				const dir = ev.key === "ArrowRight" ? 1 : -1;
				setHover((cur) =>
					Math.min(
						days.length - 1,
						Math.max(0, (cur ?? (dir > 0 ? -1 : days.length)) + dir),
					),
				);
			} else if (ev.key === "Escape") setHover(null);
		},
		[days.length],
	);

	const tipX = hover === null ? 0 : ((PL + hover * step + step / 2) / W) * 100;
	const labelIdx = [0, Math.floor(days.length / 2), days.length - 1];

	return (
		<div ref={ref} className="relative">
			<svg
				viewBox={`0 0 ${W} ${H}`}
				width={W}
				height={H}
				className="block max-w-full rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				role="img"
				// biome-ignore lint/a11y/noNoninteractiveTabindex: arrow keys read single days
				tabIndex={0}
				aria-label={t(
					"showcaseRunsChartLabel",
					"Runs per day over the last 30 days. Use the arrow keys to read single days.",
				)}
				onPointerMove={onMove}
				onPointerLeave={() => setHover(null)}
				onBlur={() => setHover(null)}
				onKeyDown={onKey}
			>
				{[0, max / 2, max].map((v) => (
					<g key={v}>
						<line
							x1={PL}
							x2={W - PR}
							y1={y(v)}
							y2={y(v)}
							className="stroke-border"
							strokeWidth={1}
						/>
						<text
							x={PL - 8}
							y={y(v) + 4}
							textAnchor="end"
							className="fill-muted-foreground text-[11px]"
						>
							{compact(v)}
						</text>
					</g>
				))}
				{bars.map((b) => (
					<path
						key={b.i}
						data-bar={b.i}
						d={b.path}
						className={cn(
							"fill-primary/85 motion-safe:transition-opacity",
							hover !== null && hover !== b.i && "opacity-35",
						)}
					/>
				))}
				{labelIdx.map((i, k) => (
					<text
						key={i}
						x={PL + i * step + step / 2}
						y={H - 8}
						textAnchor={k === 0 ? "start" : k === 2 ? "end" : "middle"}
						className="fill-muted-foreground text-[11px]"
					>
						{shortDay(days[i].day)}
					</text>
				))}
			</svg>
			{hover !== null ? (
				<output
					className="pointer-events-none absolute top-0 -translate-x-1/2 whitespace-nowrap rounded-md bg-foreground px-2.5 py-1 text-xs text-background"
					style={{ left: `${Math.min(Math.max(tipX, 12), 88)}%` }}
				>
					{t("showcaseRunsTooltip", "{{day}} · {{runs}} runs", {
						day: longDay(days[hover].day),
						runs: formatNumber(days[hover].runs),
					})}
				</output>
			) : null}
		</div>
	);
}

function Stat({
	label,
	value,
	extra,
}: Readonly<{ label: string; value: string; extra?: string }>) {
	return (
		<div className="flex flex-col">
			<small className="text-xs text-muted-foreground">{label}</small>
			<span className="flex items-baseline gap-1.5">
				<b className="text-2xl font-semibold tabular-nums tracking-tight">
					{value}
				</b>
				{extra ? (
					<span className="text-xs text-muted-foreground">{extra}</span>
				) : null}
			</span>
		</div>
	);
}

function RunsBlock({ days }: Readonly<{ days: IShowcaseRunDay[] }>) {
	const { t } = useTranslation("store");
	const stats = useMemo(() => {
		const total = days.reduce((a, d) => a + d.runs, 0);
		const peak = days.reduce((a, d) => (d.runs > a.runs ? d : a), days[0]);
		return { total, avg: Math.round(total / days.length), peak };
	}, [days]);
	return (
		<div className={`${cardClass} space-y-5 p-5`}>
			<div className="flex flex-wrap gap-x-10 gap-y-3">
				<Stat
					label={t("showcaseStatRuns", "Runs, last 30 days")}
					value={formatNumber(stats.total)}
				/>
				<Stat
					label={t("showcaseStatAverage", "Daily average")}
					value={formatNumber(stats.avg)}
				/>
				<Stat
					label={t("showcaseStatBusiest", "Busiest day")}
					value={formatNumber(stats.peak.runs)}
					extra={shortDay(stats.peak.day)}
				/>
			</div>
			<RunsChart days={days} />
		</div>
	);
}

export function ShowcaseActivity({
	activity,
}: Readonly<{ activity: IShowcaseActivity }>) {
	const { t } = useTranslation("store");
	const days = activity.runs ?? [];
	const versions = activity.versions ?? [];
	return (
		<ShowcaseSection
			id="activity"
			icon={Activity}
			eyebrow={t("showcaseActivityEyebrow", "Activity")}
			title={t("showcaseActivityTitle", "How much it gets used")}
		>
			{days.length > 0 ? <RunsBlock days={days} /> : null}
			{versions.length > 0 ? (
				<div>
					<h3 className="flex items-center gap-2 text-sm font-semibold">
						<History
							className="h-4 w-4 text-muted-foreground"
							aria-hidden="true"
						/>
						{t("showcaseVersionHistory", "Latest release")}
					</h3>
					<ol className="mt-3 flex flex-col">
						{versions.map((v, i) => (
							<li
								key={v.version}
								className="grid grid-cols-[72px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2.5 sm:grid-cols-[72px_110px_minmax(0,1fr)]"
							>
								<span
									className={cn(
										"font-mono font-medium",
										i === 0 && "text-primary",
									)}
								>
									{v.version}
								</span>
								<span className="hidden text-[12.5px] text-muted-foreground sm:block">
									{v.date ? new Date(v.date).toLocaleDateString() : ""}
								</span>
								<span className="text-sm">{v.notes}</span>
							</li>
						))}
					</ol>
				</div>
			) : null}
		</ShowcaseSection>
	);
}
