"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Activity,
	CircleCheck,
	CircleDashed,
	CirclePause,
	CirclePlay,
	CloudUpload,
	Database,
	FileKey,
	History,
	Hourglass,
	KeyRound,
	Layers,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	Package,
	Power,
	Rocket,
	Server,
	Terminal,
	Upload,
	Users,
} from "lucide-react";
import type { ReactNode } from "react";
import { useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { ProgressBar, type ProgressTone } from "./meter";
import { StatusChip } from "./status-chip";
import { type ChipTone, cx } from "./tone";

/** Byte-identical to ActivityKind (DM/workspace/types.ts) plus the multi-device rollout. */
export type TrayKind =
	| "safe_update"
	| "upload"
	| "access_rules"
	| "account_backup"
	| "agent_update"
	| "reboot"
	| "command"
	| "secret_write"
	| "history_readers"
	| "metric_readers"
	| "offline_write_retry"
	| "signing_request"
	| "setup"
	| "event_run"
	| "rollout";

/** Byte-identical to ActivityState. */
export type TrayState =
	| "active"
	| "paused"
	| "waiting"
	| "done"
	| "failed"
	| "unknown";

export const TRAY_KIND_ICON: Record<TrayKind, LucideIcon> = {
	safe_update: Rocket,
	upload: Upload,
	access_rules: Users,
	account_backup: CloudUpload,
	agent_update: Package,
	reboot: Power,
	command: Terminal,
	secret_write: KeyRound,
	history_readers: History,
	metric_readers: Activity,
	offline_write_retry: Database,
	signing_request: FileKey,
	setup: Server,
	event_run: CirclePlay,
	rollout: Layers,
};

const STATE_LOOK: Record<
	TrayState,
	{ tone: ChipTone; icon: LucideIcon; bar: ProgressTone; waiting?: boolean }
> = {
	active: { tone: "info", icon: LoaderCircle, bar: "info" },
	paused: { tone: "warning", icon: CirclePause, bar: "warning" },
	waiting: { tone: "info", icon: Hourglass, bar: "info", waiting: true },
	done: { tone: "good", icon: CircleCheck, bar: "good" },
	failed: { tone: "critical", icon: OctagonX, bar: "critical" },
	unknown: { tone: "unknown", icon: CircleDashed, bar: "unknown" },
};

export interface TrayItemProps {
	kind: TrayKind;
	/** Overrides the kind icon (start/stop/restart: play, square, rotate-cw). */
	icon?: LucideIcon;
	state: TrayState;
	/** "Safe update". */
	title: string;
	/** "edge-berlin-01 › invoice-extractor". */
	sub?: string;
	/** Chip text ("Switching over", "1 of 2 done"); the state word otherwise. */
	chip?: string;
	chipTone?: ChipTone;
	/** 0–100; omitted while active = indeterminate (only running work animates). */
	progress?: number;
	/** "settings v11 → v12 · waiting for instance #0 to be ready". */
	detail?: ReactNode;
	/** Unix seconds. */
	startedAt?: number;
	finishedAt?: number;
	/** "you". */
	by?: string;
	href?: string;
	onOpen?: () => void;
	/** "Check result", "Resume upload". */
	actions?: ReactNode;
	onDismiss?: () => void;
	compact?: boolean;
	className?: string;
}

/** SPEC §4.23: one operation in the activity tray; outcomes also live inline (R9). */
export function TrayItem({
	kind,
	icon,
	state,
	title,
	sub,
	chip,
	chipTone,
	progress,
	detail,
	startedAt,
	finishedAt,
	by,
	href,
	onOpen,
	actions,
	onDismiss,
	compact = false,
	className,
}: Readonly<TrayItemProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const look = STATE_LOOK[state];
	const Icon = icon ?? TRAY_KIND_ICON[kind];
	const stateWord: Record<TrayState, string> = {
		active: t("view.tray.active", "Running"),
		paused: t("view.tray.paused", "Paused"),
		waiting: t("view.tray.waiting", "Waiting"),
		done: t("view.tray.done", "Done"),
		failed: t("view.tray.failed", "Failed"),
		unknown: t("view.tray.unknown", "No reply received"),
	};
	const idle = state === "active" ? undefined : 0;
	const value = state === "done" ? 100 : (progress ?? idle);
	const showBar = !(state === "done" && compact);
	const finished = state === "done" || state === "failed";
	const started =
		startedAt === undefined
			? null
			: by
				? t("view.tray.startedBy", "Started {{time}} by {{by}}", {
						time: time.at(startedAt),
						by,
					})
				: t("view.tray.started", "Started {{time}}", {
						time: time.at(startedAt),
					});
	const meta: ReactNode[] = [
		started,
		finishedAt === undefined
			? null
			: t("view.tray.finished", "finished {{time}}", {
					time: time.at(finishedAt),
				}),
	].filter(Boolean);

	return (
		<article
			data-op={kind}
			data-state={state}
			aria-label={title}
			className={cx(
				"flex min-w-0 flex-col gap-1.5 bg-card",
				compact
					? "border-t border-hairline px-3 py-2.5 first:border-t-0"
					: "rounded-lg border border-border p-3",
				className,
			)}
		>
			<div className="flex min-w-0 items-start gap-2">
				<Icon
					aria-hidden
					className="mt-px size-4 shrink-0 text-muted-foreground"
				/>
				<div className="flex min-w-0 flex-1 flex-col text-ui font-semibold">
					{title}
					{sub ? (
						<span
							title={sub}
							className="truncate font-mono text-xs font-normal text-muted-foreground"
						>
							{sub}
						</span>
					) : null}
				</div>
				<StatusChip
					tone={chipTone ?? look.tone}
					icon={look.icon}
					spin={state === "active"}
				>
					{chip ?? stateWord[state]}
				</StatusChip>
			</div>
			{showBar ? (
				<ProgressBar
					value={value}
					tone={look.bar}
					waiting={look.waiting}
					label={t("view.tray.progress", "{{title}} progress", { title })}
				/>
			) : null}
			{detail || state === "unknown" ? (
				<p className="text-xs text-ink-2">
					{detail ??
						t(
							"view.tray.unknownDetail",
							"It may have run. Check the result before you try again.",
						)}
				</p>
			) : null}
			{actions ? <div className="flex flex-wrap gap-1.5">{actions}</div> : null}
			{meta.length || href || onOpen || (finished && onDismiss) ? (
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
					{meta.map((part, index) => (
						// biome-ignore lint/suspicious/noArrayIndexKey: meta parts are positional
						<span key={index}>{part}</span>
					))}
					{href ? (
						<DvButton variant="link" size="xs" asChild>
							<a href={href}>{t("view.tray.open", "Open")}</a>
						</DvButton>
					) : onOpen ? (
						<DvButton variant="link" size="xs" onClick={onOpen}>
							{t("view.tray.open", "Open")}
						</DvButton>
					) : null}
					{finished && onDismiss ? (
						<DvButton variant="link" size="xs" onClick={onDismiss}>
							{t("view.tray.dismiss", "Dismiss")}
						</DvButton>
					) : null}
				</p>
			) : null}
		</article>
	);
}
