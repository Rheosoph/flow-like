"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronUp, Layers } from "lucide-react";
import { type ReactNode, useState } from "react";
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
	/** Phone chrome: one worst-plane button. Defaults to the area's width bucket. */
	compact?: boolean;
	className?: string;
}

const SEGMENT =
	"relative inline-flex h-full min-w-0 items-center gap-1.5 px-2.5 text-xs text-muted-foreground hover:bg-row-hover hover:text-ink-2 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring aria-expanded:bg-row-selected data-[state=open]:bg-row-selected";

const STATE_TEXT: Partial<Record<PlaneState, string>> = {
	warn: "text-warning",
	err: "text-critical",
	busy: "text-info",
};

/** Calm planes give their text up first when the bar runs out of room. */
const QUIET = new Set<PlaneState>(["ok", "off"]);

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
						"border-l border-hairline first:border-l-0",
						QUIET.has(segment.state) ? "shrink-3" : "shrink",
					)}
				>
					<PlaneDot state={segment.state} />
					<Icon
						aria-hidden
						className="size-3.25 shrink-0 @max-[1100px]/devices:hidden"
					/>
					<b className="shrink-0 font-medium text-foreground">{name}</b>
					<span
						className={cx(
							"min-w-0 truncate",
							STATE_TEXT[segment.state],
							QUIET.has(segment.state) && "@max-[960px]/devices:hidden",
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

function CompactBar({
	segments,
	onOpenAll,
}: Readonly<Pick<DataPlaneBarViewProps, "segments" | "onOpenAll">>) {
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
			className={cx(SEGMENT, "min-w-0 flex-1")}
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
	return (
		<footer
			aria-label={t("chrome.planes.label", "Data sources and freshness")}
			data-chrome="planes"
			data-compact={compact ? "" : undefined}
			className={cx(
				"flex h-7 shrink-0 items-stretch overflow-hidden border-t border-border bg-card px-1 text-xs whitespace-nowrap text-muted-foreground",
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
			{compact ? null : <span className="min-w-2 flex-1" />}
			<span
				className={cx(
					"inline-flex items-center",
					!compact && "@max-[880px]/devices:hidden",
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
	const phone = useWidthBucket() === "phone";
	return (
		<DataPlaneBarView
			segments={lines}
			compact={compact ?? phone}
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
