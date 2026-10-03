"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Braces,
	ClipboardList,
	Clock,
	Globe,
	Hash,
	Link2,
	type LucideIcon,
	Mail,
	MessageSquare,
	Monitor,
	Plug,
	Send,
	Timer,
	Zap,
} from "lucide-react";
import type { ReactNode } from "react";
import type { EventEligibility } from "../../../../lib/device-management/deployment";
import { appCopy } from "../copy/app-copy";
import { eventRunsCopy, eventTypeLabel } from "../copy/eligibility-copy";
import { type DevicesT, useAreaPrefs } from "./area-context";
import { CellSub } from "./dv-table";
import { StatusChip } from "./status-chip";
import { cx } from "./tone";

/* Event types on devices (APP §7.3): the one icon map of the area. */
const EVENT_ICON: Record<string, LucideIcon> = {
	simple_chat: MessageSquare,
	page: Monitor,
	http: Globe,
	rest: Braces,
	mcp: Plug,
	daemon: Timer,
	cron: Clock,
	email: Mail,
	inbound_email: Mail,
	teams: MessageSquare,
	discord: Hash,
	telegram: Send,
	generic_form: ClipboardList,
	quick_action: Zap,
	deeplink: Link2,
	geolocation: Globe,
	api: Globe,
};

/** An event with a page reads as Page, whatever its type. */
export function eventIcon(eventType: string, hasPage = false): LucideIcon {
	return EVENT_ICON[hasPage ? "page" : eventType] ?? Zap;
}

/** The 24 px type tile in front of an event name. */
export function EventTile({
	eventType,
	hasPage = false,
	className,
}: Readonly<{ eventType: string; hasPage?: boolean; className?: string }>) {
	const Icon = eventIcon(eventType, hasPage);
	return (
		<span
			aria-hidden
			data-event-tile=""
			className={cx(
				"inline-flex size-6 shrink-0 items-center justify-center rounded-md border border-border bg-card text-ink-2",
				className,
			)}
		>
			<Icon className="size-3.5" />
		</span>
	);
}

const PIN_VERSION = "font-mono text-[11.5px]";

type RunsRule = Pick<EventEligibility, "hosted" | "readiness"> &
	Partial<
		Pick<EventEligibility, "kind" | "schedule" | "once" | "bot" | "route">
	>;

/**
 * The line under an event that can run, in its kind's own words: an
 * Endpoint's method and path, when a schedule runs, or how a device runs it.
 */
export function eventRunsLine(t: DevicesT, rule: RunsRule): string {
	return rule.route
		? t("devices:view.event.route", "{{method}} {{path}}", {
				method: rule.route.method,
				path: rule.route.path,
			})
		: eventRunsCopy(t, rule);
}

/**
 * The tag of an event whose record has no flow pin. What a device runs is
 * always a concrete flow version, shown beside it: a device never "runs Latest".
 */
export function LatestTag({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	return (
		<span
			data-follows-latest=""
			title={copy.followsLatestTitle()}
			className={cx(
				"rounded border border-border px-1 text-[11px] font-normal whitespace-nowrap text-muted-foreground",
				className,
			)}
		>
			{copy.followsLatest()}
		</span>
	);
}

export interface EventCellProps {
	eventType: string;
	hasPage?: boolean;
	name: ReactNode;
	/** The raw event id, shown with technical keys on (R3). */
	eventId?: string;
	/** The pinned versions as text: `{ event: "1.5.0", flow: "2.2.0" }`. */
	pin?: { event: string; flow: string };
	/** The line that stands in for the pins when there are none ("event 0.9.0 · flow as it is now"). */
	pinNote?: ReactNode;
	/** The event follows Latest: the tag after its type. */
	followsLatest?: boolean;
	/** How the event runs on a device, or why it cannot: `howItRunsCopy`, `eligibilityCopy`. */
	runs?: ReactNode;
	/** The version that introduced the event ("v1.5.0"): shows the "New in …" chip. */
	newIn?: string;
	className?: string;
}

/**
 * APP §2.10 event cell: type tile, name, type label, the "New in …" chip, then
 * the pinned versions and how the event runs on a device.
 */
export function EventCell({
	eventType,
	hasPage = false,
	name,
	eventId,
	pin,
	pinNote,
	followsLatest = false,
	runs,
	newIn,
	className,
}: Readonly<EventCellProps>) {
	const { t } = useTranslation("devices");
	const { showTechnicalKeys } = useAreaPrefs();
	return (
		<span
			data-event-cell=""
			className={cx("flex min-w-0 items-start gap-2.5", className)}
		>
			<EventTile eventType={eventType} hasPage={hasPage} className="mt-0.5" />
			<span className="flex min-w-0 flex-col">
				<span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
					<span className="font-semibold text-foreground">{name}</span>
					<span className="text-muted-foreground">
						{eventTypeLabel(t, eventType, hasPage)}
					</span>
					{followsLatest ? <LatestTag /> : null}
					{newIn ? (
						<StatusChip tone="info">
							{t("view.event.newIn", "New in {{version}}", { version: newIn })}
						</StatusChip>
					) : null}
				</span>
				{showTechnicalKeys && eventId ? (
					<span
						data-tech=""
						className="font-mono text-xs text-muted-foreground"
					>
						{eventId}
					</span>
				) : null}
				{pin ? (
					<CellSub>
						<Trans
							t={t}
							i18nKey="view.event.pins"
							defaults="event <1/> · flow <2/>"
							components={{
								1: <span className={PIN_VERSION}>{pin.event}</span>,
								2: <span className={PIN_VERSION}>{pin.flow}</span>,
							}}
						/>
					</CellSub>
				) : pinNote ? (
					<CellSub>{pinNote}</CellSub>
				) : null}
				{runs ? <CellSub>{runs}</CellSub> : null}
			</span>
		</span>
	);
}
