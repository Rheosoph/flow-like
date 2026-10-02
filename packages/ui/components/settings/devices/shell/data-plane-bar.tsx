"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronUp, Layers } from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import type { DevicesScope } from "../../../../lib/device-management/model/types";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { useAreaTime } from "../primitives/area-context";
import { cx } from "../primitives/tone";
import { useOverlayStore, useWidthBucket } from "../workspace";
import { CHROME_POPOVER } from "./attention-button";
import {
	PlaneDot,
	type PlaneLine,
	PlanePopover,
	type PlaneSegmentId,
	type PlaneState,
	planeCadence,
	planeIcon,
	planeName,
	usePlaneLines,
	worstPlane,
} from "./plane-popover";
import type { ChromeNavigate } from "./rail-row";

export interface DataPlaneBarProps {
	scope: DevicesScope;
	onNavigate: ChromeNavigate;
	/** One worst-plane button. Defaults to the area's width bucket (under 900 px). */
	compact?: boolean;
	className?: string;
}

const SEGMENT =
	"relative h-full items-center gap-1.5 px-2.5 text-xs text-muted-foreground hover:bg-row-hover hover:text-ink-2 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring aria-expanded:bg-row-selected data-[state=open]:bg-row-selected";

const STATE_TEXT: Partial<Record<PlaneState, string>> = {
	warn: "text-warning",
	err: "text-critical",
	busy: "text-info",
};

/** Calm planes give their text up first when the bar runs out of room. */
const QUIET = new Set<string>(["ok", "off"] satisfies PlaneState[]);

/** The order in which calm planes go down to dot + name. */
const TERSE_ORDER: readonly PlaneSegmentId[] = [
	"live",
	"certificates",
	"device",
	"status",
	"local",
	"hub",
];

const FIT_FLAGS = [
	"data-fit-icons",
	"data-fit-tight",
	"data-fit-clock",
	"data-fit-worst",
] as const;
type FitFlag = (typeof FIT_FLAGS)[number];

const BAR_INSET_PX = 8;
const SPACER_PX = 12;
/** Room a fuller form needs to spare before the bar goes back to it; more than an age text grows between two reads. */
const FIT_SLACK_PX = 40;

function clearPlaneFit(bar: HTMLElement) {
	for (const flag of FIT_FLAGS) bar.removeAttribute(flag);
	for (const segment of Array.from(bar.querySelectorAll("[data-terse]"))) {
		segment.removeAttribute("data-terse");
	}
}

function planeBarUsed(bar: HTMLElement) {
	let used = BAR_INSET_PX;
	for (const child of Array.from(bar.children)) {
		used += child.hasAttribute("data-plane-grow")
			? SPACER_PX
			: (child as HTMLElement).offsetWidth;
	}
	return used;
}

/**
 * Segments keep their content width and never ellipsize. While the bar is too
 * narrow it gives up, in this order: the icons, some padding, the text of calm
 * planes (dot + name stay), the clock, and at last the segments themselves for
 * the one "Data sources" button (`data-fit-worst`). Returns the number of steps
 * taken; pass it back as `previous` so a ticking age can't flip the form.
 */
export function fitPlaneBar(bar: HTMLElement, previous = 0): number {
	const width = bar.clientWidth;
	if (!width) return previous;
	clearPlaneFit(bar);
	const flag = (name: FitFlag) => () => bar.setAttribute(name, "");
	const terse = (id: PlaneSegmentId) => () => {
		const segment = bar.querySelector(`[data-plane="${id}"]`);
		if (QUIET.has(segment?.getAttribute("data-plane-state") ?? "")) {
			segment?.setAttribute("data-terse", "");
		}
	};
	const steps = [
		flag("data-fit-icons"),
		flag("data-fit-tight"),
		...TERSE_ORDER.map(terse),
		flag("data-fit-clock"),
		flag("data-fit-worst"),
	];
	let level = 0;
	while (level < steps.length) {
		const slack = level < previous ? FIT_SLACK_PX : 0;
		if (planeBarUsed(bar) + slack <= width) break;
		steps[level]();
		level += 1;
	}
	return level;
}

/** Refits after every render, on a width change and once the fonts are in. */
function usePlaneFit(enabled: boolean): {
	bar: RefObject<HTMLElement | null>;
	collapsed: boolean;
} {
	const bar = useRef<HTMLElement>(null);
	const level = useRef(0);
	const [collapsed, setCollapsed] = useState(false);
	const fit = useCallback(() => {
		const element = bar.current;
		if (!element) return;
		level.current = fitPlaneBar(element, level.current);
		setCollapsed(element.hasAttribute("data-fit-worst"));
	}, []);
	useLayoutEffect(() => {
		if (enabled) {
			fit();
		} else {
			level.current = 0;
			if (bar.current) clearPlaneFit(bar.current);
		}
	});
	useEffect(() => {
		const element = bar.current;
		if (!enabled || !element || typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(fit);
		observer.observe(element);
		void element.ownerDocument.fonts?.ready.then(fit);
		return () => observer.disconnect();
	}, [enabled, fit]);
	return { bar, collapsed: enabled && collapsed };
}

interface SegmentButtonProps {
	segment: PlaneLine;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	children: ReactNode;
}

function SegmentButton({
	segment,
	open,
	onOpenChange,
	children,
}: Readonly<SegmentButtonProps>) {
	const { t } = useTranslation("devices");
	const Icon = planeIcon(segment.id);
	const name = planeName(t, segment.id);
	return (
		<Popover open={open} onOpenChange={onOpenChange}>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-haspopup="dialog"
					data-plane={segment.id}
					data-plane-state={segment.state}
					title={t(
						"chrome.planes.segmentTitle",
						"{{name}} · {{word}} · refreshed {{cadence}}. Select for details.",
						{
							name,
							word: segment.word,
							cadence: planeCadence(t, segment.id),
						},
					)}
					className={cx(
						SEGMENT,
						"group/plane inline-flex shrink-0 border-l border-hairline first:border-l-0 group-data-[fit-tight]/planes:gap-1.25 group-data-[fit-tight]/planes:px-2 group-data-[fit-worst]/planes:hidden",
					)}
				>
					<PlaneDot state={segment.state} />
					<Icon
						aria-hidden
						className="size-3.25 shrink-0 group-data-[fit-icons]/planes:hidden"
					/>
					<b className="shrink-0 font-medium text-foreground">{name}</b>
					<span
						className={cx(
							"shrink-0 group-data-[terse]/plane:sr-only",
							STATE_TEXT[segment.state],
						)}
					>
						{segment.short}
					</span>
				</button>
			</PopoverTrigger>
			<PopoverContent
				side="top"
				align="start"
				aria-label={t("chrome.planes.popover", "{{name}} data source", {
					name,
				})}
				className={cx(CHROME_POPOVER, "w-[min(560px,calc(100vw-16px))]")}
			>
				{children}
			</PopoverContent>
		</Popover>
	);
}

/** "14:00:12 CEST": the status bar states the zone every time on screen is shown in (R16). */
function BarClock() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const zone = new Intl.DateTimeFormat(time.locale, { timeZoneName: "short" })
		.formatToParts(new Date(time.now))
		.find((part) => part.type === "timeZoneName")?.value;
	const place = new Intl.DateTimeFormat(time.locale).resolvedOptions().timeZone;
	return (
		<span
			data-plane-clock=""
			title={t(
				"chrome.planes.clockTitle",
				"All times are shown in this computer's time zone ({{zone}}).",
				{ zone: place },
			)}
			className="inline-flex shrink-0 items-center gap-1 px-2.5 tabular-nums"
		>
			<time className="text-ink-2">{time.clock(time.nowS)}</time>
			{zone}
		</span>
	);
}

type ClosePlane = () => void;

export interface DataPlaneBarViewProps {
	segments: readonly PlaneLine[];
	/** One worst-plane button instead of six segments. */
	compact?: boolean;
	/** The popover of one plane. */
	renderPlane: (plane: PlaneSegmentId, close: ClosePlane) => ReactNode;
	/** Compact: opens the sheet that lists every plane. */
	onOpenAll?: (plane: PlaneSegmentId | undefined) => void;
	className?: string;
}

interface CompactBarProps
	extends Pick<DataPlaneBarViewProps, "segments" | "onOpenAll"> {
	/** Stands in for the segments once they no longer fit, and shows only then. */
	fallback?: boolean;
}

function CompactBar({
	segments,
	onOpenAll,
	fallback = false,
}: Readonly<CompactBarProps>) {
	const { t } = useTranslation("devices");
	const worst = worstPlane(segments);
	const state: PlaneState = worst?.state ?? "ok";
	const summary = worst
		? t("chrome.planes.worst", "{{name}}: {{text}}", {
				name: planeName(t, worst.id),
				text: worst.short,
			})
		: t("chrome.planes.allCurrent", "all current");
	return (
		<button
			type="button"
			aria-haspopup="dialog"
			data-plane-worst={worst?.id ?? ""}
			data-plane-state={state}
			onClick={() => onOpenAll?.(worst?.id)}
			className={cx(
				SEGMENT,
				"min-w-0 flex-1",
				fallback
					? "hidden group-data-[fit-worst]/planes:inline-flex"
					: "inline-flex",
			)}
		>
			<PlaneDot state={state} />
			<Layers aria-hidden className="size-3.25 shrink-0" />
			<b className="shrink-0 font-medium text-foreground">
				{t("chrome.planes.dataSources", "Data sources")}
			</b>
			<span className={cx("min-w-0 truncate", STATE_TEXT[state])}>
				· {summary}
			</span>
			<ChevronUp aria-hidden className="size-3 shrink-0" />
		</button>
	);
}

/** The status bar over plain segments. */
export function DataPlaneBarView({
	segments,
	compact = false,
	renderPlane,
	onOpenAll,
	className,
}: Readonly<DataPlaneBarViewProps>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState<PlaneSegmentId | null>(null);
	const { bar, collapsed } = usePlaneFit(!compact);
	const alert = worstPlane(segments) !== undefined;
	return (
		<footer
			ref={bar}
			aria-label={t("chrome.planes.label", "Data sources and freshness")}
			data-chrome="planes"
			data-compact={compact ? "" : undefined}
			className={cx(
				"group/planes flex h-7 shrink-0 items-stretch overflow-hidden border-t border-border bg-card px-1 text-xs whitespace-nowrap text-muted-foreground",
				className,
			)}
		>
			{compact ? (
				<CompactBar segments={segments} onOpenAll={onOpenAll} />
			) : (
				segments.map((segment) => {
					return (
						<SegmentButton
							key={segment.id}
							segment={segment}
							open={open === segment.id}
							onOpenChange={(next) => setOpen(next ? segment.id : null)}
						>
							{open === segment.id
								? renderPlane(segment.id, () => setOpen(null))
								: null}
						</SegmentButton>
					);
				})
			)}
			{collapsed ? (
				<CompactBar segments={segments} onOpenAll={onOpenAll} fallback />
			) : null}
			{compact ? null : (
				<span
					data-plane-grow=""
					className="min-w-2 flex-1 group-data-[fit-worst]/planes:hidden"
				/>
			)}
			<span
				className={cx(
					"inline-flex shrink-0 items-center group-data-[fit-clock]/planes:hidden",
					compact && alert && "@max-[560px]/devices:hidden",
				)}
			>
				<BarClock />
			</span>
		</footer>
	);
}

/** SPEC §3.5: where the page's data comes from and how fresh it is, plus the time zone. */
export function DataPlaneBar({
	scope,
	onNavigate,
	compact,
	className,
}: Readonly<DataPlaneBarProps>) {
	const lines = usePlaneLines();
	const bucket = useWidthBucket();
	return (
		<DataPlaneBarView
			segments={lines}
			compact={compact ?? (bucket === "phone" || bucket === "narrow")}
			renderPlane={(plane, close) => (
				<PlanePopover
					plane={plane}
					scope={scope}
					onNavigate={onNavigate}
					onClose={close}
				/>
			)}
			onOpenAll={(plane) =>
				useOverlayStore.getState().openPlane(plane ?? "hub")
			}
			className={className}
		/>
	);
}
