"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ClockIcon,
	HourglassIcon,
	LockIcon,
	MailIcon,
	MessageCircleIcon,
	PinIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { formatRelativeTime } from "../../../lib/date";
import type { IAppType } from "../../../lib/schema/app/app";
import { initials, seedGradient } from "../../../lib/seed-gradient";
import { cn } from "../../../lib/utils";
import type {
	IOrgApp,
	IOrgAppStats,
	OrgAppType,
} from "../../../state/backend-state/bot-team-state";
import { AppTypeMark } from "../../ui/app-type-mark";
import {
	APP_FAMILIES,
	type AppFilter,
	type DeckEntry,
	type Schedule,
	activityOf,
	runBars,
	scheduleOf,
	triggerKinds,
} from "./org-chart-apps";
import { useOrgChart } from "./org-chart-context";

export const APP_CARD_WIDTH = 148;

export function OrgAppMark({
	app,
	size,
	badge = false,
}: Readonly<{ app: IOrgApp; size: number; badge?: boolean }>) {
	return (
		<AppTypeMark
			type={(app.app_type ?? null) as IAppType | null}
			size={size}
			src={app.icon}
			fallback={initials(app.name)}
			hideBadge={!badge}
			background={seedGradient(app.id)}
		/>
	);
}

const MAX_BOT_MARKS = 4;

/** Small type marks on a bot's row for the apps it uses; the selected app's mark gets a ring. */
export function BotAppMarks({
	appIds,
}: Readonly<{ appIds: readonly string[] | undefined }>) {
	const { t } = useTranslation("bots");
	const { org, showApps, highlight } = useOrgChart();
	if (!showApps) return null;
	const apps = (appIds ?? []).flatMap((id) => {
		const app = org.apps?.find((entry) => entry.id === id);
		return app ? [app] : [];
	});
	if (apps.length === 0) return null;
	const shown = apps.slice(0, MAX_BOT_MARKS);
	const label = t("teams.org.apps.usesTitle", "Uses {{apps}}", {
		apps: apps.map((app) => app.name).join(", "),
	});
	return (
		<span
			role="img"
			aria-label={label}
			title={label}
			className="flex shrink-0 items-center gap-0.75"
		>
			{shown.map((app) => (
				<span
					key={app.id}
					className={cn(
						"flex rounded-lg p-px",
						highlight.appIds.has(app.id) && "ring-[1.5px] ring-primary",
					)}
				>
					<OrgAppMark app={app} size={12} />
				</span>
			))}
			{apps.length > shown.length && (
				<span className="text-[10px] text-muted-foreground tabular-nums">
					+{apps.length - shown.length}
				</span>
			)}
		</span>
	);
}

function useFamilyLabel() {
	const { t } = useTranslation("bots");
	return (family: AppFilter) =>
		({
			all: t("teams.org.apps.filter.all", "All"),
			agents: t("teams.org.apps.filter.agents", "Agents"),
			apps: t("teams.org.apps.filter.apps", "Apps"),
			data: t("teams.org.apps.filter.data", "Data"),
			automations: t("teams.org.apps.filter.automations", "Automations"),
		})[family];
}

const FILTER_LOOK = {
	segmented: {
		group: "gap-0.5 rounded-lg bg-muted p-0.5",
		item: "h-6 rounded-md px-2.5",
		on: "bg-card font-medium shadow-xs",
		off: "text-muted-foreground hover:text-foreground",
	},
	pills: {
		group: "flex-wrap gap-1.5",
		item: "h-6.5 rounded-full border px-2.5",
		on: "border-foreground bg-foreground text-background",
		off: "hover:bg-muted",
	},
} as const;

/** All, Agents, Apps, Data, Automations: the toolbar's segmented control or the dialog's pills. */
export function AppFilterPicker({
	value,
	onChange,
	look,
}: Readonly<{
	value: AppFilter;
	onChange: (value: AppFilter) => void;
	look: keyof typeof FILTER_LOOK;
}>) {
	const { t } = useTranslation("bots");
	const familyLabel = useFamilyLabel();
	const style = FILTER_LOOK[look];
	return (
		<fieldset className={cn("flex items-center", style.group)}>
			<legend className="sr-only">
				{t("teams.org.apps.filterLabel", "Which apps to show")}
			</legend>
			{(["all", ...APP_FAMILIES] as const).map((family) => (
				<button
					key={family}
					type="button"
					aria-pressed={value === family}
					onClick={() => onChange(family)}
					className={cn(
						"text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring",
						style.item,
						value === family ? style.on : style.off,
					)}
				>
					{familyLabel(family)}
				</button>
			))}
		</fieldset>
	);
}

export function useAppTypeWord() {
	const { t } = useTranslation("bots");
	return (type: OrgAppType | null | undefined) => {
		switch (type) {
			case "Agent":
				return t("teams.org.apps.word.agent", "Agent");
			case "CustomInterface":
				return t("teams.org.apps.word.interface", "Interface");
			case "Form":
				return t("teams.org.apps.word.form", "Form");
			case "DataFocus":
				return t("teams.org.apps.word.data", "Data");
			case "Analytics":
				return t("teams.org.apps.word.analytics", "Analytics");
			case "DataPipeline":
				return t("teams.org.apps.word.automation", "Automation");
			default:
				return t("teams.org.apps.word.app", "App");
		}
	};
}

export function useScheduleText() {
	const { t } = useTranslation("bots");
	return (schedule: Schedule) => {
		switch (schedule.kind) {
			case "minutes":
				return t("teams.org.apps.everyMinutes", "Every {{count}} min", {
					count: schedule.every,
				});
			case "hourly":
				return t("teams.org.apps.hourly", "Every hour");
			case "hours":
				return t("teams.org.apps.everyHours", "Every {{count}} h", {
					count: schedule.every,
				});
			case "daily":
				return t("teams.org.apps.daily", "Daily at {{time}}", {
					time: schedule.time,
				});
			default:
				return schedule.expression;
		}
	};
}

function Line({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<span
			className={cn(
				"block truncate text-[10px] leading-3.25 text-muted-foreground",
				className,
			)}
		>
			{children}
		</span>
	);
}

function Strong({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span className="block truncate text-[11px] leading-3.5 font-semibold">
			{children}
		</span>
	);
}

export function RunStrip({
	stats,
	height = 18,
	barWidth = 6,
	gap = 3,
}: Readonly<{
	stats: IOrgAppStats;
	height?: number;
	barWidth?: number;
	gap?: number;
}>) {
	return (
		<span
			aria-hidden
			className="flex items-end"
			style={{ height, gap: `${gap}px` }}
		>
			{runBars(stats).map((bar, index) => (
				<span
					// biome-ignore lint/suspicious/noArrayIndexKey: one bar per day in a fixed window
					key={index}
					className={cn(
						"block rounded-[2px]",
						bar.failed > 0
							? "bg-critical"
							: bar.runs > 0
								? "bg-good-line"
								: "bg-border",
					)}
					style={{
						width: barWidth,
						height:
							bar.runs === 0
								? 3
								: Math.max(Math.round(height / 3), bar.height * height),
					}}
				/>
			))}
		</span>
	);
}

export function Sparkline({
	values,
	width,
	height,
}: Readonly<{ values: readonly number[]; width: number; height: number }>) {
	const peak = Math.max(1, ...values);
	const step = values.length > 1 ? (width - 2) / (values.length - 1) : 0;
	const points = values.map(
		(value, index) =>
			`${(1 + index * step).toFixed(1)},${(height - 2 - (value / peak) * (height - 6)).toFixed(1)}`,
	);
	if (points.length === 0) return null;
	return (
		<svg
			aria-hidden="true"
			width={width}
			height={height}
			viewBox={`0 0 ${width} ${height}`}
			className="block overflow-visible text-primary"
		>
			<polygon
				points={`1,${height - 1} ${points.join(" ")} ${width - 1},${height - 1}`}
				className="fill-current opacity-15"
			/>
			<polyline
				points={points.join(" ")}
				fill="none"
				stroke="currentColor"
				strokeWidth={1.6}
				strokeLinejoin="round"
				strokeLinecap="round"
			/>
		</svg>
	);
}

function PipelineBody({ stats }: Readonly<{ stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	const scheduleText = useScheduleText();
	const activity = activityOf(stats);
	return (
		<>
			<RunStrip stats={stats} />
			<Line className="flex items-center gap-1">
				<ClockIcon className="size-2.5 shrink-0" aria-hidden />
				<span className="truncate">
					{stats.schedule
						? scheduleText(scheduleOf(stats.schedule))
						: t("teams.org.apps.onDemand", "When something starts it")}
				</span>
			</Line>
			{activity.failedToday > 0 ? (
				<Line className="font-medium text-critical">
					{t("teams.org.apps.failedToday", "{{count}} failed today", {
						count: activity.failedToday,
					})}
				</Line>
			) : (
				<Line className="font-medium text-good">
					{stats.last_run_at
						? t("teams.org.apps.ranAgo", "Ran {{when}}", {
								when: formatRelativeTime(stats.last_run_at, "short", ""),
							})
						: t("teams.org.apps.runsWeek", "{{count}} runs this week", {
								count: activity.week,
							})}
				</Line>
			)}
		</>
	);
}

function AgentBody({ stats }: Readonly<{ stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	const kinds = triggerKinds(stats.triggers);
	const activeToday = (stats.runs.at(-1) ?? 0) > 0;
	return (
		<>
			<span className="flex items-center gap-1">
				{kinds.includes("chat") && (
					<span className="grid h-4 w-4.5 place-items-center rounded-full bg-info-bg text-info">
						<MessageCircleIcon className="size-2.5" aria-hidden />
					</span>
				)}
				{kinds.includes("mail") && (
					<span className="grid h-4 w-4.5 place-items-center rounded-full bg-info-bg text-info">
						<MailIcon className="size-2.5" aria-hidden />
					</span>
				)}
				{activeToday && (
					<span
						aria-hidden
						className="ml-auto size-1.5 rounded-full bg-good ring-3 ring-good-bg"
					/>
				)}
			</span>
			<Strong>
				{t("teams.org.apps.chatsWeek", "{{count}} chats this week", {
					count: activityOf(stats).week,
				})}
			</Strong>
			{stats.last_run_at && (
				<Line>
					{t("teams.org.apps.answeredAgo", "Answered {{when}}", {
						when: formatRelativeTime(stats.last_run_at, "short", ""),
					})}
				</Line>
			)}
		</>
	);
}

function InterfaceBody({
	app,
	stats,
}: Readonly<{ app: IOrgApp; stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<Thumbnail app={app} className="h-6.5" />
			<Strong>
				{t("teams.org.apps.pages", "{{count}} pages", { count: stats.pages })}
			</Strong>
			<Line>
				{t("teams.org.apps.usesWeek", "{{count}} uses this week", {
					count: activityOf(stats).week,
				})}
			</Line>
		</>
	);
}

export function Thumbnail({
	app,
	className,
}: Readonly<{ app: IOrgApp; className?: string }>) {
	if (app.thumbnail) {
		return (
			<img
				src={app.thumbnail}
				alt=""
				aria-hidden
				loading="lazy"
				className={cn(
					"block w-full rounded-[5px] border object-cover",
					className,
				)}
			/>
		);
	}
	return (
		<span
			aria-hidden
			className={cn(
				"relative block w-full overflow-hidden rounded-[5px] border bg-muted",
				className,
			)}
		>
			<span className="absolute inset-x-0 top-0 h-1.5 bg-primary/70" />
			<span className="absolute top-2.5 left-1.5 h-3 w-6 rounded-[2px] bg-card" />
			<span className="absolute top-2.5 right-1.5 left-9 h-1 rounded-[2px] bg-card" />
			<span className="absolute top-4.25 right-9 left-9 h-1 rounded-[2px] bg-card" />
		</span>
	);
}

function FormBody({ stats }: Readonly<{ stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<span aria-hidden className="flex flex-col gap-0.75">
				{[88, 64, 76].map((width, index) => (
					<span key={width} className="flex items-center gap-1">
						<span
							className="size-1.25 rounded-[1px] bg-primary"
							style={{ opacity: 1 - index * 0.3 }}
						/>
						<span className="h-0.75 rounded-full bg-muted" style={{ width }} />
					</span>
				))}
			</span>
			<Strong>
				{t("teams.org.apps.entriesWeek", "{{count}} entries this week", {
					count: activityOf(stats).week,
				})}
			</Strong>
			{stats.last_run_at && (
				<Line>
					{t("teams.org.apps.lastAgo", "Last {{when}}", {
						when: formatRelativeTime(stats.last_run_at, "short", ""),
					})}
				</Line>
			)}
		</>
	);
}

function DataBody({
	app,
	stats,
}: Readonly<{ app: IOrgApp; stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<span className="flex items-center gap-2">
				<span aria-hidden className="grid grid-cols-4 gap-0.5">
					{Array.from({ length: 12 }, (_, index) => (
						<span
							// biome-ignore lint/suspicious/noArrayIndexKey: decorative cells
							key={index}
							className="block h-1.25 w-1.75 rounded-[1px] bg-primary"
							style={{
								opacity: index < 4 ? 0.9 : index % 3 === 0 ? 0.35 : 0.55,
							}}
						/>
					))}
				</span>
				<Strong>
					{t("teams.org.apps.runsWeek", "{{count}} runs this week", {
						count: activityOf(stats).week,
					})}
				</Strong>
			</span>
			<Line>
				{t("teams.org.apps.updatedAgo", "Updated {{when}}", {
					when: formatRelativeTime(app.updated_at, "short", ""),
				})}
			</Line>
		</>
	);
}

function AnalyticsBody({ stats }: Readonly<{ stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<Sparkline values={stats.runs} width={130} height={24} />
			<span className="flex items-baseline justify-between gap-1">
				<Strong>
					{t("teams.org.apps.dashboards", "{{count}} dashboards", {
						count: stats.pages,
					})}
				</Strong>
			</span>
		</>
	);
}

function LockedBody({ app }: Readonly<{ app: IOrgApp }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			{app.description && (
				<span className="line-clamp-3 text-[10px] leading-3.25 text-muted-foreground">
					{app.description}
				</span>
			)}
			<span
				className={cn(
					"text-[10px] leading-3.25 font-semibold",
					app.can_request ? "text-primary" : "text-muted-foreground",
				)}
			>
				{app.can_request
					? t("teams.org.apps.requestAccess", "Request access")
					: t("teams.org.apps.noAccess", "No access")}
			</span>
		</>
	);
}

function CardBody({ app }: Readonly<{ app: IOrgApp }>) {
	const stats = app.stats;
	if (!app.can_open) return <LockedBody app={app} />;
	if (!stats) {
		return app.description ? (
			<span className="line-clamp-3 text-[10px] leading-3.25 text-muted-foreground">
				{app.description}
			</span>
		) : null;
	}
	switch (app.app_type) {
		case "DataPipeline":
			return <PipelineBody stats={stats} />;
		case "Agent":
			return <AgentBody stats={stats} />;
		case "CustomInterface":
			return <InterfaceBody app={app} stats={stats} />;
		case "Form":
			return <FormBody stats={stats} />;
		case "DataFocus":
			return <DataBody app={app} stats={stats} />;
		case "Analytics":
			return <AnalyticsBody stats={stats} />;
		default:
			return app.description ? (
				<span className="line-clamp-3 text-[10px] leading-3.25 text-muted-foreground">
					{app.description}
				</span>
			) : null;
	}
}

function CornerMark({ entry }: Readonly<{ entry: DeckEntry }>) {
	const { t } = useTranslation("bots");
	const className = "size-3 shrink-0 text-muted-foreground";
	if (entry.pin?.status === "pending") {
		const label = t("teams.org.apps.pendingTitle", "Waiting for a team admin");
		return (
			<HourglassIcon
				role="img"
				aria-label={label}
				className={cn(className, "text-warning")}
			/>
		);
	}
	if (!entry.app.can_open) {
		return (
			<LockIcon
				role="img"
				aria-label={t("teams.org.apps.lockedTitle", "You can't open this app")}
				className={className}
			/>
		);
	}
	if (entry.pin) {
		return (
			<PinIcon
				role="img"
				aria-label={t("teams.org.apps.pinnedTitle", "Pinned to the team")}
				className={className}
			/>
		);
	}
	return null;
}

export function AppCard({
	entry,
	lit,
	dim,
}: Readonly<{ entry: DeckEntry; lit: boolean; dim: boolean }>) {
	const { t } = useTranslation("bots");
	const { select, selection, nameOf, botOf } = useOrgChart();
	const typeWord = useAppTypeWord();
	const { app, pin } = entry;
	const pending = pin?.status === "pending";
	const via = pin
		? null
		: entry.usedBy
				.map((id) => botOf(id))
				.flatMap((bot) => (bot ? [nameOf(bot)] : []))
				.join(", ");
	const selected = selection?.kind === "app" && selection.id === app.id;
	return (
		<button
			type="button"
			aria-pressed={selected}
			onClick={() => select({ kind: "app", id: app.id })}
			style={{ width: APP_CARD_WIDTH }}
			className={cn(
				"flex h-29 flex-col gap-1.5 overflow-hidden rounded-[10px] border bg-card p-2 text-left shadow-xs outline-none transition-[opacity,box-shadow,border-color] focus-visible:ring-2 focus-visible:ring-ring",
				!pin && "border-dashed border-muted-foreground/40",
				pending && "border-dashed border-warning-line",
				(lit || selected) &&
					"border-solid border-primary ring-3 ring-primary/15",
				dim && !lit && !selected && "opacity-45",
			)}
		>
			<span className="flex w-full items-start gap-2">
				<OrgAppMark app={app} size={28} />
				<span className="flex min-w-0 flex-1 flex-col gap-0.5">
					<span className="line-clamp-2 text-xs leading-3.75 font-semibold">
						{app.name}
					</span>
					<span className="flex min-w-0 items-center gap-1 text-[10px] leading-3 text-muted-foreground">
						<span className="shrink-0 font-semibold tracking-wider uppercase">
							{typeWord(app.app_type)}
						</span>
						{via && (
							<span className="truncate">
								{t("teams.org.apps.via", "· via {{bots}}", { bots: via })}
							</span>
						)}
					</span>
				</span>
				<CornerMark entry={entry} />
			</span>
			<span className="mt-auto flex w-full flex-col gap-1">
				{pending ? (
					<>
						{app.description && (
							<span className="line-clamp-2 text-[10px] leading-3.25 text-muted-foreground">
								{app.description}
							</span>
						)}
						<span className="text-[10px] leading-3.25 font-semibold text-warning">
							{t("teams.org.apps.pending", "Waiting for approval")}
						</span>
					</>
				) : (
					<CardBody app={app} />
				)}
			</span>
		</button>
	);
}
