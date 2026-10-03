"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Braces,
	ChevronDown,
	ChevronRight,
	ClipboardList,
	Clock,
	Globe,
	Hash,
	Info,
	Link,
	type LucideIcon,
	Mail,
	MessageSquare,
	Monitor,
	Plug,
	Send,
	Sparkles,
	Timer,
	TriangleAlert,
	Zap,
} from "lucide-react";
import type { ReactNode } from "react";
import type { PlanTarget } from "../../../../lib/device-management/model/deploy-plan";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { cx } from "../primitives/tone";
import { stampOf } from "../shell/attention-popover";
import { useDeviceRows } from "../workspace/use-hub";
import type { DeployDevice } from "./deploy-facts";

/* Small pieces the wizard's steps share: stamps (R5), notes, disclosures and the event type tile. */

/** A block head's count when it carries words ("2 of 4", "2 selected"); `Block.count` takes a number. */
export function HeadChip({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span className="inline-flex h-5 items-center rounded-full bg-muted px-1.5 font-mono text-xs font-medium whitespace-nowrap tabular-nums text-ink-2">
			{children}
		</span>
	);
}

export function HubStamp() {
	const { freshness } = useDeviceRows();
	return <FreshnessStamp {...stampOf(freshness)} />;
}

export function LocalStamp({ text }: Readonly<{ text: string }>) {
	return <FreshnessStamp source="local" age="current" text={text} />;
}

/** Where a block's per-device facts come from: the live reads when every target is connected, the hub list otherwise. */
export function TargetsStamp({
	targets,
	devices,
}: Readonly<{
	targets: readonly PlanTarget[];
	devices: readonly DeployDevice[];
}>) {
	const { t } = useTranslation("devices");
	const picked = devices.filter((device) =>
		targets.some((target) => target.deviceId === device.id),
	);
	if (!picked.length) return <HubStamp />;
	if (picked.every((device) => device.isLive))
		return (
			<FreshnessStamp
				source="live"
				age="live"
				text={t("deploy.stamp.live", "read over the live connection")}
			/>
		);
	return (
		<FreshnessStamp
			source="live"
			age="notloaded"
			text={t("deploy.stamp.notLive", "loads once connected")}
		/>
	);
}

const NOTE_TONE = {
	info: "border-border bg-surface-sunken text-ink-2",
	warning: "border-warning-line bg-warning-bg text-foreground",
	critical: "border-critical-line bg-critical-bg text-foreground",
} as const;

const NOTE_ICON_TONE: Record<keyof typeof NOTE_TONE, string> = {
	info: "text-muted-foreground",
	warning: "text-warning",
	critical: "text-critical",
};

/** A one-paragraph inline note under a control. */
export function Note({
	tone = "info",
	icon,
	className,
	children,
}: Readonly<{
	tone?: keyof typeof NOTE_TONE;
	icon?: LucideIcon;
	className?: string;
	children: ReactNode;
}>) {
	const Icon = icon ?? (tone === "info" ? Info : TriangleAlert);
	return (
		<p
			data-note={tone}
			className={cx(
				"flex items-start gap-2 rounded-lg border px-3 py-2 text-ui",
				NOTE_TONE[tone],
				className,
			)}
		>
			<Icon
				aria-hidden
				className={cx("mt-0.5 size-3.5 shrink-0", NOTE_ICON_TONE[tone])}
			/>
			<span className="min-w-0">{children}</span>
		</p>
	);
}

/** A toggle row that opens advanced content in place. */
export function Disclosure({
	label,
	summary,
	open,
	onToggle,
	children,
}: Readonly<{
	label: ReactNode;
	summary?: ReactNode;
	open: boolean;
	onToggle(): void;
	children?: ReactNode;
}>) {
	const Icon = open ? ChevronDown : ChevronRight;
	return (
		<div className="flex min-w-0 flex-col gap-2">
			<button
				type="button"
				aria-expanded={open}
				onClick={onToggle}
				className="inline-flex w-fit max-w-full items-start gap-1.5 rounded-sm text-left text-ui hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			>
				<Icon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
				<span className="font-medium">{label}</span>
				{summary ? (
					<span className="min-w-0 text-muted-foreground">{summary}</span>
				) : null}
			</button>
			{open ? children : null}
		</div>
	);
}

export interface SelectOption<T extends string> {
	value: T;
	label: string;
	disabled?: boolean;
}

/** One choice out of a few per device (plan line, certificate). */
export function DeploySelect<T extends string>({
	id,
	label,
	value,
	options,
	disabled = false,
	mono = false,
	onChange,
}: Readonly<{
	id: string;
	/** Accessible name ("Plan on edge-berlin-01"). */
	label: string;
	value: T;
	options: readonly SelectOption<T>[];
	disabled?: boolean;
	mono?: boolean;
	onChange(value: T): void;
}>) {
	return (
		<Select
			value={value}
			disabled={disabled}
			onValueChange={(next) => onChange(next as T)}
		>
			<SelectTrigger
				id={id}
				aria-label={label}
				className={cx(
					"h-8.5 w-full min-w-0 rounded-lg border-input bg-card px-2.5 text-ui shadow-none",
					mono && "font-mono",
				)}
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						disabled={option.disabled}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

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
	deeplink: Link,
	geolocation: Globe,
	api: Globe,
};

/** The event's type as a small tile (APP §7.3 device icons); an event with a page reads as Page. */
export function EventTile({
	eventType,
	hasPage,
}: Readonly<{ eventType: string; hasPage: boolean }>) {
	const Icon = EVENT_ICON[hasPage ? "page" : eventType] ?? Sparkles;
	return (
		<span
			aria-hidden
			className="inline-flex size-6 shrink-0 items-center justify-center rounded-md border border-border bg-surface-sunken text-ink-2"
		>
			<Icon className="size-3.5" />
		</span>
	);
}
