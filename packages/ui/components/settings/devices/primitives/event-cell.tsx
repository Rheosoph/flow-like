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
import { eventTypeLabel } from "../copy/eligibility-copy";
import { useAreaPrefs } from "./area-context";
import { CellSub } from "./dv-table";
import { StatusChip } from "./status-chip";
import { cx } from "./tone";

/* Event types on devices (APP §7.3). */
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

export interface EventCellProps {
	eventType: string;
	hasPage?: boolean;
	name: ReactNode;
	/** The raw event id, shown with technical keys on (R3). */
	eventId?: string;
	/** The pinned versions as text: `{ event: "1.5.0", flow: "2.2.0" }`. */
	pin?: { event: string; flow: string };
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
				) : null}
				{runs ? <CellSub>{runs}</CellSub> : null}
			</span>
		</span>
	);
}
