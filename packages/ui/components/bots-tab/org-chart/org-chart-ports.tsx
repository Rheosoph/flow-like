"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarDaysIcon,
	CalendarSyncIcon,
	MailIcon,
	SendIcon,
	WebhookIcon,
} from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import { formatRelativeTime } from "../../../lib/date";
import { cn } from "../../../lib/utils";
import {
	type IOrgBot,
	type ITeam,
	ORG_MAIL_WINDOW_DAYS,
	type OrgCalendarProvider,
} from "../../../state/backend-state/bot-team-state";
import { useOrgChart } from "./org-chart-context";
import {
	type InputPort,
	type OutputPort,
	botPorts,
	calendarMark,
	teamFlow,
} from "./org-chart-io";

const TONE = {
	mail: "border-info-line bg-info-bg text-info",
	webhook: "border-locked-line bg-locked-bg text-locked",
	sent: "border-good-line bg-good-bg text-good",
	more: "border-dashed border-border bg-card text-muted-foreground",
} as const;

export function useProviderName() {
	const { t } = useTranslation("bots");
	return (provider: OrgCalendarProvider) =>
		({
			google: t("teams.org.io.google", "Google Calendar"),
			microsoft: t("teams.org.io.microsoft", "Outlook calendar"),
			caldav: t("teams.org.io.caldav", "CalDAV calendar"),
		})[provider];
}

function MailAddress({ address }: Readonly<{ address: string }>) {
	const at = address.indexOf("@");
	if (at < 0) return <span className="truncate">{address}</span>;
	return (
		<span className="min-w-0 truncate">
			<span className="font-medium">{address.slice(0, at)}</span>
			<span className="opacity-70">{address.slice(at)}</span>
		</span>
	);
}

function Pill({
	tone,
	title,
	children,
	className,
}: Readonly<{
	tone: keyof typeof TONE;
	title: string;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<span
			title={title}
			className={cn(
				"inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-full border px-2 text-[11px] leading-none whitespace-nowrap",
				TONE[tone],
				className,
			)}
		>
			{children}
		</span>
	);
}

function useInputText() {
	const { t } = useTranslation("bots");
	return (port: InputPort) => {
		if (port.kind === "mail") {
			return t(
				"teams.org.io.mailInTitle",
				"Mail to {{address}} starts its work: {{count}} received in the last {{days}} days",
				{
					address: port.address,
					count: port.received,
					days: ORG_MAIL_WINDOW_DAYS,
				},
			);
		}
		return port.lastCalledAt
			? t(
					"teams.org.io.webhookTitle",
					"Webhook {{name}} starts its work. Last called {{when}}",
					{
						name: port.name,
						when: formatRelativeTime(port.lastCalledAt, "long", ""),
					},
				)
			: t(
					"teams.org.io.webhookNeverTitle",
					"Webhook {{name}} starts its work. Not called yet",
					{ name: port.name },
				);
	};
}

function InputPill({ port }: Readonly<{ port: InputPort }>) {
	const title = useInputText()(port);
	if (port.kind === "mail") {
		return (
			<Pill tone="mail" title={title}>
				<MailIcon className="size-3 shrink-0" aria-hidden />
				<MailAddress address={port.address} />
				{port.received > 0 && (
					<span className="shrink-0 tabular-nums opacity-80">
						{port.received}
					</span>
				)}
			</Pill>
		);
	}
	return (
		<Pill tone="webhook" title={title}>
			<WebhookIcon className="size-3 shrink-0" aria-hidden />
			<span className="min-w-0 truncate font-medium">{port.name}</span>
			{port.lastCalledAt && (
				<span className="shrink-0 opacity-70">
					{formatRelativeTime(port.lastCalledAt, "narrow", "")}
				</span>
			)}
		</Pill>
	);
}

function OutputPill({ port }: Readonly<{ port: OutputPort }>) {
	const { t } = useTranslation("bots");
	return (
		<Pill
			tone="sent"
			className={port.sent === 0 ? "opacity-60" : undefined}
			title={t(
				"teams.org.io.sentTitle",
				"Sent {{count}} mails from {{address}} in the last {{days}} days",
				{
					count: port.sent,
					address: port.address,
					days: ORG_MAIL_WINDOW_DAYS,
					defaultValue_one:
						"Sent {{count}} mail from {{address}} in the last {{days}} days",
				},
			)}
		>
			<SendIcon className="size-3 shrink-0" aria-hidden />
			<span className="tabular-nums">
				{t("teams.org.io.sent", "{{count}} sent", { count: port.sent })}
			</span>
		</Pill>
	);
}

function MorePill({ count }: Readonly<{ count: number }>) {
	const { t } = useTranslation("bots");
	return (
		<Pill
			tone="more"
			title={t(
				"teams.org.io.moreTitle",
				"{{count}} more ways in. Select the bot to see all",
				{
					count,
					defaultValue_one: "{{count}} more way in. Select the bot to see all",
				},
			)}
		>
			{t("teams.org.io.more", "+{{count}} more", { count })}
		</Pill>
	);
}

type Side = "in" | "out";

const WIRE_GAP = { in: "left-full", out: "right-full" } as const;
const BUS_AT = {
	in: "left-[calc(100%+0.625rem)]",
	out: "right-[calc(100%+0.625rem)]",
} as const;
const DOT_AT = {
	in: "left-[calc(100%+1.25rem)] -translate-x-1/2",
	out: "right-[calc(100%+1.25rem)] translate-x-1/2",
} as const;

function wireTone(active: boolean) {
	return active ? "bg-primary" : "bg-border";
}

/** One pill of a stack with its wire to the stack's bus. */
function PortSlot({
	side,
	active,
	wired,
	children,
}: Readonly<{
	side: Side;
	active: boolean;
	wired: boolean;
	children: ReactNode;
}>) {
	return (
		<span className="relative flex max-w-full min-w-0">
			{children}
			{wired && (
				<span
					aria-hidden
					className={cn(
						"absolute top-1/2 h-px w-2.5",
						WIRE_GAP[side],
						wireTone(active),
					)}
				/>
			)}
		</span>
	);
}

/**
 * Pills stacked beside a bot row. A single pill wires straight to the team box; several meet on
 * a bus first, so every way in visibly ends at the same bot.
 */
function PortStack({
	side,
	active,
	count,
	children,
}: Readonly<{
	side: Side;
	active: boolean;
	count: number;
	children: ReactNode;
}>) {
	const tone = wireTone(active);
	return (
		<div
			className={cn(
				"relative flex max-w-full min-w-0 flex-col gap-1",
				side === "in" ? "items-end" : "items-start",
			)}
		>
			{children}
			{count > 1 && (
				<span
					aria-hidden
					className={cn("absolute top-3 bottom-3 w-px", BUS_AT[side], tone)}
				/>
			)}
			<span
				aria-hidden
				className={cn(
					"absolute top-1/2 h-px",
					count > 1 ? cn(BUS_AT[side], "w-2.5") : cn(WIRE_GAP[side], "w-5"),
					tone,
				)}
			/>
			<span
				aria-hidden
				className={cn(
					"absolute top-1/2 size-1.5 -translate-y-1/2 rounded-full",
					DOT_AT[side],
					tone,
				)}
			/>
		</div>
	);
}

/** The left rail of one bot row: its mail address and webhooks, wired into the bot. */
export function InputRail({
	bot,
	active,
	style,
}: Readonly<{
	bot: IOrgBot | undefined;
	active: boolean;
	style?: CSSProperties;
}>) {
	const ports = bot ? botPorts(bot) : null;
	const count = (ports?.inputs.length ?? 0) + (ports?.moreInputs ? 1 : 0);
	const wired = count > 1;
	return (
		<div
			style={style}
			className="col-start-1 flex min-w-0 items-center justify-end py-0.5 pr-5"
		>
			{ports && count > 0 && (
				<PortStack side="in" active={active} count={count}>
					{ports.inputs.map((port) => (
						<PortSlot key={port.key} side="in" active={active} wired={wired}>
							<InputPill port={port} />
						</PortSlot>
					))}
					{ports.moreInputs > 0 && (
						<PortSlot side="in" active={active} wired={wired}>
							<MorePill count={ports.moreInputs} />
						</PortSlot>
					)}
				</PortStack>
			)}
		</div>
	);
}

/** The right rail of one bot row: the mail it sent. */
export function OutputRail({
	bot,
	active,
	style,
}: Readonly<{
	bot: IOrgBot | undefined;
	active: boolean;
	style?: CSSProperties;
}>) {
	const outputs = bot ? botPorts(bot).outputs : [];
	return (
		<div
			style={style}
			className="col-start-3 flex min-w-0 items-center justify-start py-0.5 pl-5"
		>
			{outputs.length > 0 && (
				<PortStack side="out" active={active} count={outputs.length}>
					{outputs.map((port) => (
						<PortSlot
							key={port.key}
							side="out"
							active={active}
							wired={outputs.length > 1}
						>
							<OutputPill port={port} />
						</PortSlot>
					))}
				</PortStack>
			)}
		</div>
	);
}

function RailStat({
	icon,
	value,
	title,
}: Readonly<{ icon: ReactNode; value: number; title: string }>) {
	return (
		<span
			title={title}
			className="inline-flex items-center gap-0.5 tabular-nums"
		>
			{icon}
			{value}
		</span>
	);
}

/** The heads of a team's rails: what flows into the team and what it sends out. */
export function RailHeads({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { botOf } = useOrgChart();
	const flow = teamFlow(team, botOf);
	const days = ORG_MAIL_WINDOW_DAYS;
	const label =
		"text-[10px] font-semibold tracking-wider text-muted-foreground uppercase";
	return (
		<>
			<div className="col-start-1 row-start-1 flex items-center justify-end gap-2 pr-5 text-[11px] text-muted-foreground">
				{flow.received > 0 && (
					<RailStat
						icon={<MailIcon className="size-3" aria-hidden />}
						value={flow.received}
						title={t(
							"teams.org.io.teamReceived",
							"{{count}} mails reached {{team}} in the last {{days}} days",
							{
								count: flow.received,
								team: team.name,
								days,
								defaultValue_one:
									"{{count}} mail reached {{team}} in the last {{days}} days",
							},
						)}
					/>
				)}
				{flow.webhooks > 0 && (
					<RailStat
						icon={<WebhookIcon className="size-3" aria-hidden />}
						value={flow.webhooks}
						title={t(
							"teams.org.io.teamWebhooks",
							"{{count}} webhooks lead into {{team}}",
							{
								count: flow.webhooks,
								team: team.name,
								defaultValue_one: "{{count}} webhook leads into {{team}}",
							},
						)}
					/>
				)}
				<span className={label}>{t("teams.org.io.in", "In")}</span>
			</div>
			<div className="col-start-3 row-start-1 flex items-center gap-2 pl-5 text-[11px] text-muted-foreground">
				<span className={label}>{t("teams.org.io.out", "Out")}</span>
				{flow.sent > 0 && (
					<RailStat
						icon={<SendIcon className="size-3" aria-hidden />}
						value={flow.sent}
						title={t(
							"teams.org.io.teamSent",
							"{{team}} sent {{count}} mails in the last {{days}} days",
							{
								count: flow.sent,
								team: team.name,
								days,
								defaultValue_one:
									"{{team}} sent {{count}} mail in the last {{days}} days",
							},
						)}
					/>
				)}
			</div>
		</>
	);
}

/** Its mail address and webhooks as small chips under the bot's name, where the chart has no room for rails. */
export function InlinePorts({ bot }: Readonly<{ bot: IOrgBot }>) {
	const ports = botPorts(bot);
	if (ports.inputs.length === 0) return null;
	return (
		<span className="mt-1 flex min-w-0 flex-wrap gap-1">
			{ports.inputs.map((port) => (
				<InputPill key={port.key} port={port} />
			))}
			{ports.moreInputs > 0 && <MorePill count={ports.moreInputs} />}
			{ports.outputs.map((port) => (
				<OutputPill key={port.key} port={port} />
			))}
		</span>
	);
}

/** Marks a bot that keeps its own calendar, and one that reads connected calendars. */
export function CalendarMarks({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const providerName = useProviderName();
	const mark = calendarMark(bot);
	if (!mark) return null;
	const ownLabel = t(
		"teams.org.io.ownCalendar",
		"Own calendar: its events wake it",
	);
	const connectedLabel = t(
		"teams.org.io.connectedCalendars",
		"Connected calendars: {{names}}",
		{ names: mark.connected.map(providerName).join(", ") },
	);
	return (
		<span className="flex shrink-0 items-center gap-0.5">
			{mark.own && (
				<span
					role="img"
					aria-label={ownLabel}
					title={ownLabel}
					className="inline-flex text-warning"
				>
					<CalendarDaysIcon className="size-3" aria-hidden />
				</span>
			)}
			{mark.connected.length > 0 && (
				<span
					role="img"
					aria-label={connectedLabel}
					title={connectedLabel}
					className="inline-flex items-center gap-0.5 rounded-full bg-info-bg px-1 text-[10px] leading-4 text-info"
				>
					<CalendarSyncIcon className="size-3" aria-hidden />
					{mark.connected.length > 1 && (
						<span className="tabular-nums">{mark.connected.length}</span>
					)}
				</span>
			)}
		</span>
	);
}
