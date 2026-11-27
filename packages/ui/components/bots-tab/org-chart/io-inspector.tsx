"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarDaysIcon,
	CalendarSyncIcon,
	CheckIcon,
	CopyIcon,
	MailIcon,
	WebhookIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { activeBots } from "../../../lib/bot-teams/org-model";
import { formatRelativeTime } from "../../../lib/date";
import {
	type IOrgBot,
	type ITeam,
	ORG_MAIL_WINDOW_DAYS,
} from "../../../state/backend-state/bot-team-state";
import { useCopy } from "../../settings/devices/primitives/use-copy";
import { Button } from "../../ui/button";
import { InspectorSection } from "./inspector-parts";
import { useOrgChart } from "./org-chart-context";
import { allInputs, calendarMark, teamFlow } from "./org-chart-io";
import { OrgBotAvatar } from "./org-chart-parts";
import { useProviderName } from "./org-chart-ports";

function Line({
	icon,
	tone,
	children,
	action,
}: Readonly<{
	icon: ReactNode;
	tone: string;
	children: ReactNode;
	action?: ReactNode;
}>) {
	return (
		<li className="flex min-w-0 items-center gap-2 text-sm">
			<span
				className={`flex size-6 shrink-0 items-center justify-center rounded-md ${tone}`}
			>
				{icon}
			</span>
			<span className="flex min-w-0 flex-1 flex-col">{children}</span>
			{action}
		</li>
	);
}

function CopyAddress({ address }: Readonly<{ address: string }>) {
	const { t } = useTranslation("bots");
	const { copied, copy } = useCopy();
	return (
		<Button
			variant="ghost"
			size="icon"
			className="size-7"
			aria-label={t("teams.org.io.copyAddress", "Copy address")}
			onClick={() => void copy(address)}
		>
			{copied ? <CheckIcon aria-hidden /> : <CopyIcon aria-hidden />}
		</Button>
	);
}

function LastCalled({ at }: Readonly<{ at: string | null }>) {
	const { t } = useTranslation("bots");
	return (
		<span className="text-xs text-muted-foreground">
			{at
				? t("teams.org.io.lastCalled", "Last called {{when}}", {
						when: formatRelativeTime(at, "long", ""),
					})
				: t("teams.org.io.neverCalled", "Not called yet")}
		</span>
	);
}

/** How work reaches one bot from outside its chats, and the mail it sends. */
export function BotIoSection({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const providerName = useProviderName();
	const inputs = allInputs(bot);
	const calendars = calendarMark(bot);
	const mail = bot.inputs?.mail;
	if (inputs.length === 0 && !calendars) return null;
	return (
		<InspectorSection
			title={t("teams.org.io.botTitle", "Ways in and out")}
			hint={t(
				"teams.org.io.botHint",
				"What starts its work outside the chats. Counts cover the last {{days}} days.",
				{ days: ORG_MAIL_WINDOW_DAYS },
			)}
		>
			<ul className="flex flex-col gap-2">
				{mail && (
					<Line
						icon={<MailIcon className="size-3.5" aria-hidden />}
						tone="bg-info-bg text-info"
						action={<CopyAddress address={mail.address} />}
					>
						<span className="truncate font-medium">{mail.address}</span>
						<span className="text-xs text-muted-foreground">
							{t(
								"teams.org.io.mailCounts",
								"{{received}} received · {{sent}} sent",
								{ received: mail.received, sent: mail.sent },
							)}
						</span>
					</Line>
				)}
				{inputs.flatMap((port) =>
					port.kind === "webhook"
						? [
								<Line
									key={port.key}
									icon={<WebhookIcon className="size-3.5" aria-hidden />}
									tone="bg-locked-bg text-locked"
								>
									<span className="truncate font-medium">{port.name}</span>
									<LastCalled at={port.lastCalledAt} />
								</Line>,
							]
						: [],
				)}
				{calendars?.own && (
					<Line
						icon={<CalendarDaysIcon className="size-3.5" aria-hidden />}
						tone="bg-warning-bg text-warning"
					>
						<span className="font-medium">
							{t("teams.org.io.ownCalendarLine", "Its own calendar")}
						</span>
						<span className="text-xs text-muted-foreground">
							{t(
								"teams.org.io.ownCalendarHint",
								"Events it keeps wake it before they start",
							)}
						</span>
					</Line>
				)}
				{calendars && calendars.connected.length > 0 && (
					<Line
						icon={<CalendarSyncIcon className="size-3.5" aria-hidden />}
						tone="bg-info-bg text-info"
					>
						<span className="truncate font-medium">
							{calendars.connected.map(providerName).join(", ")}
						</span>
						<span className="text-xs text-muted-foreground">
							{t("teams.org.io.connectedHint", "Connected calendars it reads")}
						</span>
					</Line>
				)}
			</ul>
		</InspectorSection>
	);
}

/** Every way into a team: each active bot's mail address and webhooks, with the week's mail. */
export function TeamIoSection({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { botOf, nameOf, select } = useOrgChart();
	const flow = teamFlow(team, botOf);
	const ways = activeBots(team).flatMap((entry) => {
		const bot = botOf(entry.bot_id);
		return bot ? allInputs(bot).map((port) => ({ bot, port })) : [];
	});
	if (ways.length === 0) return null;
	return (
		<InspectorSection
			title={t("teams.org.io.teamTitle", "Ins and outs")}
			hint={t(
				"teams.org.io.teamHint",
				"{{received}} mails in and {{sent}} out in the last {{days}} days.",
				{
					received: flow.received,
					sent: flow.sent,
					days: ORG_MAIL_WINDOW_DAYS,
				},
			)}
		>
			<ul className="flex flex-col gap-1">
				{ways.map(({ bot, port }) => (
					<li key={`${bot.id}-${port.key}`}>
						<button
							type="button"
							onClick={() => select({ kind: "bot", id: bot.id })}
							className="flex w-full min-w-0 items-center gap-2 rounded-lg px-1 py-1 text-left text-xs outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
						>
							{port.kind === "mail" ? (
								<MailIcon className="size-3.5 shrink-0 text-info" aria-hidden />
							) : (
								<WebhookIcon
									className="size-3.5 shrink-0 text-locked"
									aria-hidden
								/>
							)}
							<span className="min-w-0 flex-1 truncate font-medium">
								{port.kind === "mail" ? port.address : port.name}
							</span>
							<span className="flex shrink-0 items-center gap-1.5 text-muted-foreground">
								{port.kind === "mail" && (
									<span className="tabular-nums">
										{t(
											"teams.org.io.mailCountsShort",
											"{{received}} in · {{sent}} out",
											{
												received: port.received,
												sent: bot.inputs?.mail?.sent ?? 0,
											},
										)}
									</span>
								)}
								<OrgBotAvatar bot={bot} size={18} />
								<span className="max-w-24 truncate">{nameOf(bot)}</span>
							</span>
						</button>
					</li>
				))}
			</ul>
		</InspectorSection>
	);
}
