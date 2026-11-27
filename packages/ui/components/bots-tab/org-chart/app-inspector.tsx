"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpRightIcon,
	CalendarClockIcon,
	LockIcon,
	type LucideIcon,
	PlayIcon,
	ZapIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import { appTypeMeta } from "../../../lib/app-type";
import { isTeamAdmin } from "../../../lib/bot-teams/org-model";
import { formatRelativeTime } from "../../../lib/date";
import type { IAppType } from "../../../lib/schema/app/app";
import type {
	IOrgApp,
	IOrgAppStats,
	IOrgBot,
	ITeam,
	ITeamApp,
} from "../../../state/backend-state/bot-team-state";
import { Button } from "../../ui/button";
import {
	OrgAppMark,
	RunStrip,
	Sparkline,
	Thumbnail,
	useScheduleText,
} from "./app-card";
import { Chip, InspectorSection } from "./inspector-parts";
import {
	type TriggerKind,
	activityOf,
	appPins,
	appUsers,
	canUnpin,
	scheduleOf,
	sumRecent,
	triggerKinds,
} from "./org-chart-apps";
import { useOrgChart } from "./org-chart-context";
import { OrgBotAvatar, PersonName } from "./org-chart-parts";

function useTriggerWord() {
	const { t } = useTranslation("bots");
	return (kind: TriggerKind) =>
		({
			chat: t("teams.org.apps.trigger.chat", "Chat"),
			mail: t("teams.org.apps.trigger.mail", "Mail"),
			form: t("teams.org.apps.trigger.form", "Form"),
			schedule: t("teams.org.apps.trigger.schedule", "Schedule"),
			webhook: t("teams.org.apps.trigger.webhook", "Webhook or API"),
		})[kind];
}

function Banner({ app }: Readonly<{ app: IOrgApp }>) {
	if (app.thumbnail) {
		return (
			<div className="-mx-4 -mt-4 h-32 overflow-hidden border-b bg-muted">
				<Thumbnail app={app} className="h-full rounded-none border-0" />
			</div>
		);
	}
	const shape = appTypeMeta((app.app_type ?? null) as IAppType | null).shape;
	return (
		<div
			aria-hidden
			className="relative -mx-4 -mt-4 h-24 overflow-hidden border-b bg-muted text-primary"
		>
			<span
				className="absolute -top-9 -right-8 size-40 bg-current opacity-20"
				style={shape}
			/>
			<span
				className="absolute top-8 right-32 size-14 bg-current opacity-10"
				style={shape}
			/>
			<span
				className="absolute top-2.5 right-48 size-6 bg-current opacity-10"
				style={shape}
			/>
		</div>
	);
}

interface Fact {
	icon: LucideIcon;
	label: string;
	value: string;
}

function FactList({ facts }: Readonly<{ facts: Fact[] }>) {
	return (
		<dl className="flex flex-col gap-2 text-xs">
			{facts.map(({ icon: Icon, label, value }) => (
				<div
					key={label}
					className="grid grid-cols-[16px_96px_minmax(0,1fr)] items-center gap-x-1.5"
				>
					<Icon className="size-3.5 text-muted-foreground" aria-hidden />
					<dt className="text-muted-foreground">{label}</dt>
					<dd className="truncate font-medium">{value}</dd>
				</div>
			))}
		</dl>
	);
}

function useLiveFacts() {
	const { t } = useTranslation("bots");
	const scheduleText = useScheduleText();
	const triggerWord = useTriggerWord();
	const ago = (value: string | null | undefined) =>
		value
			? formatRelativeTime(value, "long", "")
			: t("teams.org.apps.notRecently", "Not in the last 14 days");
	return (app: IOrgApp, stats: IOrgAppStats): Fact[] => {
		const activity = activityOf(stats);
		const starts = triggerKinds(stats.triggers).map(triggerWord).join(", ");
		const startsFact: Fact[] = starts
			? [
					{
						icon: ZapIcon,
						label: t("teams.org.apps.fact.startsOn", "Starts on"),
						value: starts,
					},
				]
			: [];
		const lastRun = {
			icon: PlayIcon,
			label: t("teams.org.apps.fact.lastRun", "Last run"),
			value: ago(stats.last_run_at),
		};
		const updated = {
			icon: CalendarClockIcon,
			label: t("teams.org.apps.fact.updated", "Updated"),
			value: ago(app.updated_at),
		};
		switch (app.app_type) {
			case "DataPipeline": {
				const total = sumRecent(stats.runs, stats.runs.length);
				const failed = sumRecent(stats.failed, stats.failed.length);
				return [
					{
						icon: CalendarClockIcon,
						label: t("teams.org.apps.fact.schedule", "Schedule"),
						value: stats.schedule
							? scheduleText(scheduleOf(stats.schedule))
							: t("teams.org.apps.onDemand", "When something starts it"),
					},
					...startsFact,
					lastRun,
					{
						icon: PlayIcon,
						label: t("teams.org.apps.fact.fortnight", "14 days"),
						value: t(
							"teams.org.apps.okFailed",
							"{{ok}} ok, {{failed}} failed",
							{
								ok: total - failed,
								failed,
							},
						),
					},
				];
			}
			case "Agent":
				return [
					{
						icon: PlayIcon,
						label: t("teams.org.apps.fact.week", "This week"),
						value: t("teams.org.apps.chatsWeek", "{{count}} chats this week", {
							count: activity.week,
						}),
					},
					...startsFact,
					{
						...lastRun,
						label: t("teams.org.apps.fact.lastAnswer", "Last answer"),
					},
				];
			case "Form":
				return [
					{
						icon: PlayIcon,
						label: t("teams.org.apps.fact.week", "This week"),
						value: t(
							"teams.org.apps.entriesWeek",
							"{{count}} entries this week",
							{ count: activity.week },
						),
					},
					{
						...lastRun,
						label: t("teams.org.apps.fact.lastEntry", "Last entry"),
					},
				];
			case "CustomInterface":
			case "Analytics":
				return [
					{
						icon: CalendarClockIcon,
						label:
							app.app_type === "Analytics"
								? t("teams.org.apps.fact.dashboards", "Dashboards")
								: t("teams.org.apps.fact.pages", "Pages"),
						value: String(stats.pages),
					},
					{
						icon: PlayIcon,
						label: t("teams.org.apps.fact.week", "This week"),
						value: t("teams.org.apps.usesWeek", "{{count}} uses this week", {
							count: activity.week,
						}),
					},
					updated,
				];
			default:
				return [
					{
						icon: PlayIcon,
						label: t("teams.org.apps.fact.week", "This week"),
						value: t("teams.org.apps.runsWeek", "{{count}} runs this week", {
							count: activity.week,
						}),
					},
					...startsFact,
					updated,
				];
		}
	};
}

function useSectionTitle() {
	const { t } = useTranslation("bots");
	return (app: IOrgApp) => {
		switch (app.app_type) {
			case "DataPipeline":
				return t("teams.org.apps.section.runs", "Runs, last 14 days");
			case "Agent":
				return t(
					"teams.org.apps.section.conversations",
					"Conversations, last 14 days",
				);
			case "Form":
				return t("teams.org.apps.section.entries", "Entries, last 14 days");
			case "DataFocus":
				return t("teams.org.apps.section.data", "Data");
			default:
				return t("teams.org.apps.section.activity", "Activity, last 14 days");
		}
	};
}

function LiveSection({
	app,
	stats,
}: Readonly<{ app: IOrgApp; stats: IOrgAppStats }>) {
	const { t } = useTranslation("bots");
	const facts = useLiveFacts()(app, stats);
	const title = useSectionTitle()(app);
	const bars = app.app_type === "DataPipeline" || app.app_type === "Form";
	return (
		<section
			aria-label={title}
			className="flex flex-col gap-3 rounded-[10px] border p-3"
		>
			<h3 className="text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
				{title}
			</h3>
			{bars ? (
				<div className="flex flex-col gap-1">
					<RunStrip stats={stats} height={40} barWidth={16} gap={4} />
					<span className="flex justify-between text-[10px] text-muted-foreground">
						<span>{t("teams.org.apps.daysAgo", "14 days ago")}</span>
						<span>{t("teams.org.apps.today", "Today")}</span>
					</span>
				</div>
			) : (
				<Sparkline values={stats.runs} width={300} height={44} />
			)}
			<FactList facts={facts} />
		</section>
	);
}

function MetaRow({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<>
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="flex min-w-0 flex-wrap items-center gap-1">{children}</dd>
		</>
	);
}

function PinControls({
	app,
	team,
	pin,
}: Readonly<{ app: IOrgApp; team: ITeam; pin: ITeamApp }>) {
	const { t } = useTranslation("bots");
	const { me, actions } = useOrgChart();
	const fail = {
		onError: (error: Error) =>
			toast.error(
				apiErrorMessage(
					error,
					t("teams.org.apps.pinFailed", "Couldn't change the pin."),
				),
			),
	};
	const args = { teamId: team.id, appId: app.id };
	const pending = pin.status === "pending";
	const admin = isTeamAdmin(team, me);
	if (pending && admin) {
		return (
			<div className="flex flex-wrap items-center gap-2">
				<Button
					size="sm"
					className="h-7 text-xs"
					disabled={actions.approveAppPin.isPending}
					onClick={() => actions.approveAppPin.mutate(args, fail)}
				>
					{t("teams.org.apps.approveIn", "Approve for {{team}}", {
						team: team.name,
					})}
				</Button>
				<Button
					size="sm"
					variant="outline"
					className="h-7 text-xs"
					disabled={actions.declineAppPin.isPending}
					onClick={() => actions.declineAppPin.mutate(args, fail)}
				>
					{t("teams.org.apps.decline", "Decline")}
				</Button>
			</div>
		);
	}
	if (!canUnpin(team, pin, app, me)) return null;
	return (
		<Button
			size="sm"
			variant="outline"
			className="h-7 self-start text-xs"
			disabled={actions.unpinApp.isPending}
			onClick={() => actions.unpinApp.mutate(args, fail)}
		>
			{pending
				? t("teams.org.apps.withdrawIn", "Withdraw from {{team}}", {
						team: team.name,
					})
				: t("teams.org.apps.unpinFrom", "Unpin from {{team}}", {
						team: team.name,
					})}
		</Button>
	);
}

function Access({ app }: Readonly<{ app: IOrgApp }>) {
	const { t } = useTranslation("bots");
	const { onOpenApp } = useOrgChart();
	if (app.can_open) {
		return (
			<Button onClick={() => onOpenApp(app.id, "app")}>
				{t("teams.org.apps.open", "Open app")}
				<ArrowUpRightIcon aria-hidden />
			</Button>
		);
	}
	return (
		<div className="flex flex-col gap-2">
			<p className="flex gap-2 rounded-[10px] bg-muted p-3 text-xs leading-relaxed text-muted-foreground">
				<LockIcon className="mt-0.5 size-4 shrink-0" aria-hidden />
				{t(
					"teams.org.apps.lockedHint",
					"You can't open this app. Everyone in the org sees what it is; its live numbers show to its members.",
				)}
			</p>
			{app.can_request ? (
				<Button onClick={() => onOpenApp(app.id, "store")}>
					{t("teams.org.apps.requestAccess", "Request access")}
				</Button>
			) : (
				app.owner_id && (
					<p className="text-xs text-muted-foreground">
						{t("teams.org.apps.askOwner", "Ask its owner to add you:")}{" "}
						<PersonName userId={app.owner_id} className="font-medium" />
					</p>
				)
			)}
		</div>
	);
}

export function AppInspector({ app }: Readonly<{ app: IOrgApp }>) {
	const { t } = useTranslation("bots");
	const { org, botOf } = useOrgChart();
	const meta = appTypeMeta((app.app_type ?? null) as IAppType | null);
	const TypeIcon = meta.icon;
	const pins = appPins(org, app.id);
	const users = appUsers(org, app.id).flatMap((id) => {
		const bot = botOf(id);
		return bot ? [bot] : [];
	});
	return (
		<div className="flex flex-col gap-5">
			<Banner app={app} />
			<header className="-mt-11 flex flex-col gap-1">
				<span className="mb-1 inline-flex self-start rounded-2xl bg-card p-1">
					<OrgAppMark app={app} size={48} badge />
				</span>
				<h2 className="text-base leading-tight font-semibold">{app.name}</h2>
				<span className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
					<TypeIcon className="size-3" aria-hidden />
					{app.app_type
						? meta.label
						: t("teams.org.apps.unclassified", "No type yet")}
				</span>
				{app.description && (
					<p className="mt-1 text-sm leading-relaxed text-muted-foreground">
						{app.description}
					</p>
				)}
			</header>

			<dl className="grid grid-cols-[96px_minmax(0,1fr)] gap-x-2 gap-y-2 text-xs">
				{pins.map(({ team, pin }) => (
					<MetaRow
						key={team.id}
						label={t("teams.org.apps.pinnedTo", "Pinned to")}
					>
						<span className="font-medium">{team.name}</span>
						<span className="text-muted-foreground">
							{pin.status === "pending"
								? t("teams.org.apps.pendingBy", "· waiting, asked by")
								: t("teams.org.apps.by", "· by")}
						</span>
						<PersonName userId={pin.pinned_by} />
					</MetaRow>
				))}
				{app.owner_id && (
					<MetaRow label={t("teams.org.apps.owner", "Owner")}>
						<PersonName userId={app.owner_id} />
					</MetaRow>
				)}
			</dl>

			{pins.length > 0 && (
				<div className="flex flex-col gap-2">
					{pins.map(({ team, pin }) => (
						<PinControls key={team.id} app={app} team={team} pin={pin} />
					))}
				</div>
			)}

			{users.length > 0 && (
				<InspectorSection
					title={t("teams.org.apps.usedBy", "Bots that use it")}
				>
					<div className="flex flex-wrap gap-1.5">
						{users.map((bot) => (
							<BotUserChip key={bot.id} bot={bot} />
						))}
					</div>
				</InspectorSection>
			)}

			{app.can_open && app.stats && <LiveSection app={app} stats={app.stats} />}
			<Access app={app} />
		</div>
	);
}

function BotUserChip({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { nameOf, select } = useOrgChart();
	return (
		<Chip onClick={() => select({ kind: "bot", id: bot.id })}>
			<OrgBotAvatar bot={bot} size={20} />
			<span className="truncate">{nameOf(bot)}</span>
		</Chip>
	);
}
