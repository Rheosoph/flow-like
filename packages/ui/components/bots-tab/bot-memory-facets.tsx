"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDownIcon, HourglassIcon, LocateFixedIcon } from "lucide-react";
import { type ReactNode, useId, useMemo } from "react";
import { cn } from "../../lib/utils";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import type { MemoryKindStyle } from "./bot-memory-kinds";
import {
	type MemoryFormat,
	type MemoryPlace,
	type MemoryTtl,
	type MemoryValidity,
	memoryPlace,
	memoryTtl,
	memoryValidity,
	parseInstant,
	placeGlyph,
	wholeDayZone,
} from "./bot-memory-model";

const glyphFrame =
	"relative flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-[10px] border border-border/70 bg-muted/45";

function svgId(id: string) {
	return id.replace(/[^\w-]/g, "");
}

function FacetRow({
	glyph,
	title,
	detail,
	tone,
	action,
}: Readonly<{
	glyph: ReactNode;
	title: string;
	detail?: string;
	tone?: "warn" | "muted";
	action?: {
		label: string;
		onClick: () => void;
		expanded?: boolean;
	};
}>) {
	const body = (
		<>
			{glyph}
			<span className="flex min-w-0 flex-1 flex-col gap-0.5">
				<span
					className={cn(
						"truncate text-xs font-medium",
						tone === "warn"
							? "text-amber-700 dark:text-amber-300"
							: tone === "muted"
								? "text-muted-foreground"
								: "text-foreground",
					)}
					title={title}
				>
					{title}
				</span>
				{detail && (
					<span
						className="truncate text-[11px] text-muted-foreground"
						title={detail}
					>
						{detail}
					</span>
				)}
			</span>
		</>
	);
	const layout = "flex min-h-13 w-full min-w-0 items-center gap-3 px-2 py-1.5";
	if (!action) return <div className={layout}>{body}</div>;
	return (
		<button
			type="button"
			onClick={action.onClick}
			aria-expanded={action.expanded}
			aria-label={`${title} · ${action.label}`}
			title={action.label}
			className={cn(
				layout,
				"text-left transition-colors hover:bg-muted/45 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
			)}
		>
			{body}
			{action.expanded === undefined ? (
				<LocateFixedIcon
					className="size-3.5 shrink-0 text-muted-foreground"
					aria-hidden="true"
				/>
			) : (
				<ChevronDownIcon
					className={cn(
						"size-3.5 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none",
						action.expanded && "rotate-180",
					)}
					aria-hidden="true"
				/>
			)}
		</button>
	);
}

function CalendarGlyph({
	time,
	format,
	style,
}: Readonly<{ time: number; format: MemoryFormat; style: MemoryKindStyle }>) {
	return (
		<span
			aria-hidden="true"
			className="flex size-10 shrink-0 flex-col overflow-hidden rounded-[10px] border border-border/70 bg-card shadow-xs"
		>
			<span
				className={cn(
					"text-center text-[8.5px] font-semibold uppercase leading-3.5 tracking-wider",
					style.color,
				)}
			>
				{format.monthShort(time).replace(/\.$/, "")}
			</span>
			<span className="flex flex-1 items-center justify-center text-[15px] font-semibold leading-none tabular-nums">
				{format.dayOfMonth(time)}
			</span>
		</span>
	);
}

export function PlaceGlyph({
	place,
	style,
	className,
}: Readonly<{
	place: MemoryPlace;
	style: MemoryKindStyle;
	className?: string;
}>) {
	const grid = `${svgId(useId())}-grid`;
	const glyph = useMemo(() => placeGlyph(place), [place]);
	const single = glyph.points.length === 1 && !glyph.areas && !glyph.lines;
	return (
		<span aria-hidden className={cn(glyphFrame, style.accent, className)}>
			<svg
				aria-hidden="true"
				focusable="false"
				viewBox="0 0 40 40"
				className="size-full"
			>
				<defs>
					<pattern id={grid} width="8" height="8" patternUnits="userSpaceOnUse">
						<path
							d="M8 0H0V8"
							fill="none"
							stroke="currentColor"
							strokeOpacity="0.14"
							strokeWidth="0.6"
						/>
					</pattern>
				</defs>
				<rect width="40" height="40" fill={`url(#${grid})`} />
				{glyph.areas && (
					<path
						d={glyph.areas}
						fill="currentColor"
						fillOpacity="0.2"
						fillRule="evenodd"
						stroke="currentColor"
						strokeWidth="1.25"
						strokeLinejoin="round"
					/>
				)}
				{glyph.lines && (
					<path
						d={glyph.lines}
						fill="none"
						stroke="currentColor"
						strokeWidth="1.75"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
				)}
				{glyph.points.map(([x, y]) =>
					single ? (
						<g key={`${x}-${y}`}>
							<circle cx={x} cy={y} r="9" fill="currentColor" opacity="0.12" />
							<circle cx={x} cy={y} r="5.5" fill="currentColor" opacity="0.2" />
							<circle
								cx={x}
								cy={y}
								r="3"
								fill="currentColor"
								className="stroke-card"
								strokeWidth="1.5"
							/>
						</g>
					) : (
						<circle
							key={`${x}-${y}`}
							cx={x}
							cy={y}
							r="2.25"
							fill="currentColor"
							className="stroke-card"
							strokeWidth="1"
						/>
					),
				)}
			</svg>
		</span>
	);
}

function TtlGlyph({ ttl }: Readonly<{ ttl: MemoryTtl }>) {
	const radius = 14;
	const circumference = 2 * Math.PI * radius;
	return (
		<span
			aria-hidden="true"
			className={cn(
				glyphFrame,
				ttl.urgent
					? "text-amber-600 dark:text-amber-400"
					: "text-foreground/55",
			)}
		>
			<svg
				aria-hidden="true"
				viewBox="0 0 40 40"
				className="absolute inset-0 size-full -rotate-90"
			>
				<circle
					cx="20"
					cy="20"
					r={radius}
					fill="none"
					stroke="currentColor"
					strokeOpacity="0.15"
					strokeWidth="2.5"
				/>
				<circle
					cx="20"
					cy="20"
					r={radius}
					fill="none"
					stroke="currentColor"
					strokeWidth="2.5"
					strokeLinecap="round"
					strokeDasharray={circumference}
					strokeDashoffset={circumference * (1 - ttl.fraction)}
				/>
			</svg>
			<HourglassIcon className="size-3.5" strokeWidth={1.75} />
		</span>
	);
}

function ValidityGlyph({
	validity,
	now,
	style,
}: Readonly<{
	validity: MemoryValidity;
	now: number;
	style: MemoryKindStyle;
}>) {
	const fade = `${svgId(useId())}-fade`;
	const known = [validity.from, validity.to, now].filter(
		(time): time is number => time !== null,
	);
	const low = Math.min(...known);
	const span = Math.max(...known) - low || 1;
	const x = (time: number) => 10 + ((time - low) / span) * 20;
	const start = validity.from === null ? 4 : x(validity.from);
	const end = validity.to === null ? 36 : x(validity.to);
	return (
		<span aria-hidden className={cn(glyphFrame, style.accent)}>
			<svg
				aria-hidden="true"
				focusable="false"
				viewBox="0 0 40 40"
				className="size-full"
			>
				<defs>
					<linearGradient
						id={fade}
						x1="4"
						x2="36"
						y1="0"
						y2="0"
						gradientUnits="userSpaceOnUse"
					>
						<stop
							offset="0"
							stopColor="currentColor"
							stopOpacity={validity.from === null ? 0 : 1}
						/>
						<stop offset="0.3" stopColor="currentColor" />
						<stop offset="0.7" stopColor="currentColor" />
						<stop
							offset="1"
							stopColor="currentColor"
							stopOpacity={validity.to === null ? 0 : 1}
						/>
					</linearGradient>
				</defs>
				<line
					x1="4"
					x2="36"
					y1="20"
					y2="20"
					stroke="currentColor"
					strokeOpacity="0.14"
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<line
					x1={start}
					x2={end}
					y1="20"
					y2="20"
					stroke={`url(#${fade})`}
					strokeOpacity={validity.state === "ended" ? 0.45 : 1}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				{validity.from !== null && (
					<circle cx={start} cy="20" r="2.5" fill="currentColor" />
				)}
				{validity.to !== null && (
					<circle cx={end} cy="20" r="2.5" fill="currentColor" />
				)}
				<line
					x1={x(now)}
					x2={x(now)}
					y1="13"
					y2="27"
					className="stroke-foreground"
					strokeOpacity="0.7"
					strokeWidth="1.25"
					strokeLinecap="round"
				/>
			</svg>
		</span>
	);
}

function useCoordinates(format: MemoryFormat) {
	const { t } = useTranslation("bots");
	return ([lon, lat]: [number, number], digits: number) =>
		`${format.coordinate(Math.abs(lat), digits)}° ${
			lat >= 0 ? t("memory.place.north", "N") : t("memory.place.south", "S")
		}, ${format.coordinate(Math.abs(lon), digits)}° ${
			lon >= 0 ? t("memory.place.east", "E") : t("memory.place.west", "W")
		}`;
}

export interface MemoryFacetModel {
	time: number | null;
	place: MemoryPlace | null;
	ttl: MemoryTtl | null;
	validity: MemoryValidity | null;
}

export function memoryFacets(fact: BotFact, now: number): MemoryFacetModel {
	return {
		time: parseInstant(fact.time),
		place: memoryPlace(fact.geojson),
		ttl: memoryTtl(fact, now),
		validity: memoryValidity(fact, now),
	};
}

/** When, where, for how long and how long it holds: each as a row with a small picture of it. */
export function MemoryFacets({
	facets,
	now,
	format,
	style,
	placeAction,
	children,
}: Readonly<{
	facets: MemoryFacetModel;
	now: number;
	format: MemoryFormat;
	style: MemoryKindStyle;
	/** `expanded` set: the row toggles a map below it; unset: the row focuses a map elsewhere. */
	placeAction: { onClick: () => void; expanded?: boolean };
	/** Rendered directly below the place row, e.g. its map. */
	children?: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const coordinates = useCoordinates(format);
	const { time, place, ttl, validity } = facets;
	if (time === null && !place && !ttl && !validity) return null;
	const placeTitle = (place: MemoryPlace) => {
		switch (place.shape) {
			case "point":
				return coordinates(place.center, 4);
			case "area":
				return t("memory.place.area", "Area");
			case "line":
				return t("memory.place.route", "Route");
			case "points":
				return t("memory.place.points", "{{count}} places", {
					count: place.count,
					defaultValue_one: "{{count}} place",
				});
			default:
				return t("memory.place.mixed", "Places");
		}
	};
	return (
		<div className="mt-3 divide-y divide-border/50 overflow-hidden rounded-xl border border-border/60 bg-background/70">
			{time !== null && (
				<FacetRow
					glyph={<CalendarGlyph time={time} format={format} style={style} />}
					title={format.day(time, now)}
					detail={
						wholeDayZone(time)
							? format.relative(time, now)
							: `${format.timeOfDay(time)} · ${format.relative(time, now)}`
					}
				/>
			)}
			{place && (
				<div>
					<FacetRow
						glyph={<PlaceGlyph place={place} style={style} />}
						title={placeTitle(place)}
						detail={
							place.shape === "point"
								? t("memory.place.pinned", "Pinned location")
								: t("memory.place.around", "Around {{coordinates}}", {
										coordinates: coordinates(place.center, 2),
									})
						}
						action={{
							...placeAction,
							label:
								placeAction.expanded === true
									? t("memory.place.hide", "Hide map")
									: t("memory.place.show", "Show on map"),
						}}
					/>
					{children}
				</div>
			)}
			{ttl && (
				<FacetRow
					glyph={<TtlGlyph ttl={ttl} />}
					tone={ttl.urgent ? "warn" : undefined}
					title={t("memory.ttl.title", "Forgets {{when}}", {
						when: format.relative(ttl.expiresAt, now),
					})}
					detail={format.full(ttl.expiresAt)}
				/>
			)}
			{validity && (
				<FacetRow
					glyph={<ValidityGlyph validity={validity} now={now} style={style} />}
					tone={validity.state === "ended" ? "muted" : undefined}
					title={
						validity.from !== null && validity.to !== null
							? format.range(validity.from, validity.to)
							: validity.from !== null
								? t("memory.validity.since", "Since {{date}}", {
										date: format.date(validity.from),
									})
								: t("memory.validity.until", "Until {{date}}", {
										date: format.date(validity.to ?? now),
									})
					}
					detail={
						validity.state === "ended"
							? t("memory.validity.ended", "Ended {{when}}", {
									when: format.relative(validity.to ?? now, now),
								})
							: validity.state === "pending"
								? t("memory.validity.pending", "Starts {{when}}", {
										when: format.relative(validity.from ?? now, now),
									})
								: t("memory.validity.current", "Still true")
					}
				/>
			)}
		</div>
	);
}
