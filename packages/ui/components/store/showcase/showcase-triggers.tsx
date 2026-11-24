"use client";

import { useTranslation } from "@flow-like/locales";
import { Clock, KeyRound, Users, Zap } from "lucide-react";
import type { IShowcaseEvent } from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { endpointMethod, eventKind, isEndpointEvent } from "./model";
import { Chip, ShowcaseSection, interactiveCardClass } from "./showcase-ui";

export type ShowcaseAccess = "members" | "purchase";

export function AccessChip({ access }: Readonly<{ access: ShowcaseAccess }>) {
	const { t } = useTranslation("store");
	return access === "purchase" ? (
		<Chip icon={KeyRound}>{t("showcaseAccessPurchase", "After purchase")}</Chip>
	) : (
		<Chip icon={Users}>{t("showcaseAccessMembers", "Members")}</Chip>
	);
}

export function useEventKindLabel() {
	const { t } = useTranslation("store");
	return (type: string) => {
		const [key, fallback] = eventKind(type).label;
		return t(key as "showcaseEventOther", fallback);
	};
}

export function MethodBadge({ method }: Readonly<{ method: string }>) {
	return (
		<span
			className={cn(
				"rounded-sm px-1.5 py-0.5 font-mono text-[11px] font-semibold",
				method === "GET"
					? "bg-[var(--showcase-time)]/15 text-foreground"
					: "bg-[var(--showcase-number)]/15 text-foreground",
			)}
		>
			{method}
		</span>
	);
}

export function EventTechnical({ event }: Readonly<{ event: IShowcaseEvent }>) {
	const { t } = useTranslation("store");
	const tech = event.technical;
	if (!tech) return null;
	if (isEndpointEvent(event)) {
		return (
			<div className="flex flex-wrap items-center gap-2 text-[13px]">
				<MethodBadge method={endpointMethod(event)} />
				<span className="wrap-anywhere font-mono">{tech.path}</span>
			</div>
		);
	}
	if (tech.cron) {
		return (
			<div className="space-y-0.5">
				<b className="block font-mono text-sm font-semibold">{tech.cron}</b>
				{tech.timezone ? (
					<small className="flex items-center gap-1 text-xs text-muted-foreground">
						<Clock className="h-3 w-3" aria-hidden="true" />
						{tech.timezone}
					</small>
				) : null}
			</div>
		);
	}
	if (tech.path) {
		return (
			<span className="wrap-anywhere font-mono text-[13px]">{tech.path}</span>
		);
	}
	return (
		<span className="sr-only">
			{t("showcaseTechnical", "Technical details")}
		</span>
	);
}

function EventCard({
	event,
	access,
	onOpen,
}: Readonly<{
	event: IShowcaseEvent;
	access: ShowcaseAccess;
	onOpen: (id: string) => void;
}>) {
	const kind = eventKind(event.event_type);
	const label = useEventKindLabel()(event.event_type);
	return (
		<button
			type="button"
			className={`${interactiveCardClass} flex flex-col gap-2.5`}
			onClick={() => onOpen(event.id)}
		>
			<span className="flex items-center justify-between gap-2">
				<span className="inline-flex items-center gap-1.5 text-xs font-semibold text-primary">
					<kind.icon className="h-4 w-4" aria-hidden="true" />
					{label}
				</span>
				<AccessChip access={access} />
			</span>
			<span className="wrap-anywhere text-[15px] font-semibold">
				{event.name}
			</span>
			{event.description ? (
				<span className="line-clamp-3 text-[13px] text-muted-foreground">
					{event.description}
				</span>
			) : null}
			<EventTechnical event={event} />
		</button>
	);
}

export function ShowcaseTriggers({
	events,
	access,
	onOpen,
}: Readonly<{
	events: IShowcaseEvent[];
	access: ShowcaseAccess;
	onOpen: (id: string) => void;
}>) {
	const { t } = useTranslation("store");
	return (
		<ShowcaseSection
			id="triggers"
			icon={Zap}
			eyebrow={t("showcaseTriggersEyebrow", "Ways to use it")}
			title={t("showcaseTriggersTitle", "How to start it")}
			subtitle={t(
				"showcaseTriggersSubtitle",
				"Each of these starts the app. Open one to see how it works.",
			)}
		>
			<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,300px),1fr))] gap-3">
				{events.map((e) => (
					<EventCard key={e.id} event={e} access={access} onOpen={onOpen} />
				))}
			</div>
		</ShowcaseSection>
	);
}
