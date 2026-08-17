"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ClipboardListIcon,
	ClockIcon,
	CodeIcon,
	CogIcon,
	FileTextIcon,
	FormInputIcon,
	GitBranchIcon,
	GlobeIcon,
	HashIcon,
	LayersIcon,
	LayoutIcon,
	LinkIcon,
	MailIcon,
	MessageSquareIcon,
	PlugIcon,
	SendIcon,
	ServerIcon,
	UnplugIcon,
	ZapIcon,
} from "lucide-react";
import type { ComponentType } from "react";
import { describeEventEntry } from "../../../lib/event-entry";
import { getEventTypeGlyph } from "../../../lib/event-sections";
import type { IEvent } from "../../../lib/schema/flow/event";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import type { SurfaceRunHealth } from "../../settings/dashboard/use-project-runs";
import type { IFlowRow } from "./flows-overview-model";

/** Mirrors the private map in `settings/events/events-overview.tsx`. */
const TYPE_ICONS: Record<string, ComponentType<{ className?: string }>> = {
	clock: ClockIcon,
	globe: GlobeIcon,
	server: ServerIcon,
	plug: PlugIcon,
	"message-square": MessageSquareIcon,
	hash: HashIcon,
	send: SendIcon,
	mail: MailIcon,
	link: LinkIcon,
	zap: ZapIcon,
	"clipboard-list": ClipboardListIcon,
	layout: LayoutIcon,
	cog: CogIcon,
	layers: LayersIcon,
	"form-input": FormInputIcon,
	code: CodeIcon,
	"git-branch": GitBranchIcon,
	"file-text": FileTextIcon,
};

const PILL_BASE =
	"shrink-0 rounded-full border px-1.5 py-px text-[9px] font-medium uppercase tracking-wide";

function BindingPill({
	tone,
	children,
}: Readonly<{
	tone: "paused" | "pinned" | "latest" | "canary" | "public";
	children: string;
}>) {
	const { t } = useTranslation("flow");
	const tones: Record<typeof tone, string> = {
		paused:
			`border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400`,
		pinned: `border-border bg-muted text-muted-foreground`,
		latest: `border-border/70 bg-muted/60 text-muted-foreground`,
		canary: `border-primary/40 bg-primary/10 text-primary`,
		public:
			`border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400`,
	};
	return <span className={cn(PILL_BASE, tones[tone])}>{children}</span>;
}

function BindingActivity({ health }: Readonly<{ health?: SurfaceRunHealth }>) {
	const { t } = useTranslation("flow");
	if (!health || health.total === 0) {
		return (
			<span className="text-[10px] italic text-muted-foreground/60">
				{t('noRunsIn24H', 'no runs in 24 h')}
			</span>
		);
	}
	const max = Math.max(...health.trend, 1);
	// Same fixed two-hour slots as the rail histogram, keyed by their age.
	const buckets = health.trend.map((value, index) => ({
		hoursAgo: (health.trend.length - index) * 2,
		value,
	}));
	return (
		<>
			<span className="block font-mono text-[10px] tabular-nums text-muted-foreground">
				{health.total.toLocaleString()} runs
				{health.failed > 0 ? (
					<span className="ml-1 text-red-500">{t('failedFailed', '{{failed}} failed', { failed: health.failed })}</span>
				) : null}
			</span>
			<span className="mt-1 flex h-3 items-end justify-end gap-px">
				{buckets.map(({ hoursAgo, value }) => (
					<span
						key={`spark-${hoursAgo}h`}
						className="w-[3px] rounded-sm bg-primary/45"
						style={{ height: `${Math.max(1, (value / max) * 12)}px` }}
					/>
				))}
			</span>
		</>
	);
}

function BindingRow({
	event,
	row,
	health,
}: Readonly<{ event: IEvent; row: IFlowRow; health?: SurfaceRunHealth }>) {
	const { t } = useTranslation("flow");
	const glyph = getEventTypeGlyph(event);
	const Icon = TYPE_ICONS[glyph.icon] ?? CogIcon;
	const entry = describeEventEntry(
		event,
		parseUint8ArrayToJson(event.config) ?? {},
	);
	const target = event.node_id
		? (row.board.nodes?.[event.node_id]?.friendly_name ??
			t('aNodeThatNoLongerExists', 'a Node that no longer exists'))
		: (row.pages.find((page) => page.pageId === event.default_page_id)?.name ??
			t('aPage', 'a page'));

	return (
		<li
			className={cn(
				"flex items-start gap-2.5 rounded-md border border-border/50 bg-background/40 p-2",
				!event.active && "opacity-70",
			)}
		>
			<span className="grid size-7 shrink-0 place-items-center rounded-md border bg-muted/50 text-muted-foreground">
				<Icon className="size-3.5" />
			</span>
			<span className="min-w-0 flex-1">
				<span className="flex flex-wrap items-center gap-1.5">
					<span className="truncate text-xs font-medium">{event.name}</span>
					{entry ? (
						<span
							className="shrink-0 rounded bg-muted px-1 py-px font-mono text-[10px] text-muted-foreground"
							title={entry.title ?? entry.text}
						>
							{entry.text}
						</span>
					) : null}
					{!event.active ? (
						<BindingPill tone="paused">{t('paused', 'Paused')}</BindingPill>
					) : null}
					{event.board_version ? (
						<BindingPill tone="pinned">
							{t('pinnedVval', 'pinned v{{val}}', { val: event.board_version.join(".") })}
						</BindingPill>
					) : (
						<BindingPill tone="latest">{t('latest', 'Latest')}</BindingPill>
					)}
					{event.canary ? (
						<BindingPill tone="canary">{t('canary', 'Canary')}</BindingPill>
					) : null}
					{event.exposure === "PUBLIC" ? (
						<BindingPill tone="public">{t('public', 'Public')}</BindingPill>
					) : null}
				</span>
				<span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
					{t('startsAt', 'starts at')} <span className="font-mono">{target}</span>
				</span>
			</span>
			<span className="ml-auto shrink-0 text-right">
				<BindingActivity health={health} />
			</span>
		</li>
	);
}

export function FlowCardBindings({
	row,
	healthByEvent,
	eventsHref,
}: Readonly<{
	row: IFlowRow;
	healthByEvent: Map<string, SurfaceRunHealth>;
	eventsHref?: string;
}>) {
	const { t } = useTranslation("flow");
	if (row.entryPoints.length === 0) {
		return (
			<p className="rounded-md border border-dashed border-amber-500/40 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-600 dark:text-amber-400"><Trans i18nKey="spanClassnamefontmediumnoEntryPointspanARunAlwaysBeginsAtOneNodeMarkedAsAStartAndThisFlowHasNoneSoNothingCanBeBoundToIt"><span className="font-medium">No entry point.</span> A run always begins
				at one node marked as a start, and this flow has none — so nothing can
				be bound to it.</Trans></p>
		);
	}

	if (row.bindings.length === 0) {
		return (
			<p className="rounded-md border border-dashed border-amber-500/40 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-600 dark:text-amber-400">
				<span className="font-medium">{t('nothingIsBound', 'Nothing is bound.')}</span>{" "}
				{t('countEntryPointsExist', { defaultValue_one: 'One entry point exists', defaultValue_other: '{{count}} entry points exist', count: row.entryPoints.length })}{" "}
				{t('butNoEventTargetsThisFlowSoOnlyTheEditorCanReachIt', 'but no event targets this flow, so only the editor can reach it.')}
				{eventsHref ? (
					<>
						{" "}<Trans i18nKey="aHrefeventshrefClassnameunderlineUnderlineoffset2BindOneOnEventsA"><a href={eventsHref} className="underline underline-offset-2">
							Bind one on Events
						</a>
						.</Trans></>
				) : null}
			</p>
		);
	}

	return (
		<ul className="flex flex-col gap-1.5">
			{row.bindings.map((event) => (
				<BindingRow
					key={event.id}
					event={event}
					row={row}
					health={healthByEvent.get(event.id)}
				/>
			))}
		</ul>
	);
}

export function FlowCardEntryPoints({ row }: Readonly<{ row: IFlowRow }>) {
	if (row.entryPoints.length === 0) return null;
	return (
		<ul className="flex flex-wrap gap-1">
			{row.entryPoints.map((node) => (
				<li
					key={node.id}
					className="inline-flex items-center gap-1 rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] text-muted-foreground"
				>
					<UnplugIcon className="size-3 opacity-70" />
					<span className="max-w-40 truncate">{node.friendly_name}</span>
				</li>
			))}
		</ul>
	);
}
