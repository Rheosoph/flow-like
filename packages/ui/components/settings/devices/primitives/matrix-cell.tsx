"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	CircleDashed,
	KeyRound,
	Lock,
	type LucideIcon,
	OctagonX,
	Rocket,
	WifiOff,
} from "lucide-react";
import type { ReactNode } from "react";
import { type DevicesT, useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { LatestTag } from "./event-cell";
import { type Gate, GatedAction } from "./gate-notice";
import { PairedPins } from "./paired-pins";
import type { DesiredRun, ObservedRun } from "./requested-actual";
import type { ConvergenceChipKind } from "./status-chip";
import { StatusChip } from "./status-chip";
import { cx } from "./tone";

/** APP §2.10 cell states; byte-identical to MatrixCellState in DM/model/app-plan.ts. */
export type MatrixCellKind =
	| "served"
	| "staged"
	| "not_served"
	| "cant_here"
	| "unknown"
	| "no_access";

export interface MatrixServedCell {
	state: "served" | "staged";
	serviceId: string;
	href?: string;
	desired: DesiredRun;
	observed: ObservedRun;
	conv: ConvergenceChipKind;
	/** Translated actual label ("Running", "Restarting after a crash · last known"). */
	actual: string;
	/** "1.4.0 · flow 2.1.0". */
	pin: string;
	/** The newest version's pin when this one is behind ("1.5.0", "flow 2.2.0"); omitted = newest. */
	target?: string;
	targetTitle?: string;
	/**
	 * Stands in for "newest" when the pin can't be called so: flow edits no
	 * version holds (`info`), or a flow state that isn't known (`unknown`).
	 */
	note?: { text: string; tone: "info" | "unknown" };
	/** The event has no flow pin: the "Follows Latest" tag beside the concrete version the device runs. */
	tag?: boolean;
	/** A schedule: its next run, or why it does not run here. */
	lines?: readonly string[];
	/** ":8081"; `tls` adds the lock. */
	port?: string;
	tls?: boolean;
	/** `staged`: the staged event version. */
	stagedVersion?: string;
	onActivate?: () => void;
	activateGate?: Gate | null;
	/** Other services on the device serving the same event. */
	also?: readonly string[];
}

export interface MatrixNotServedCell {
	state: "not_served";
	deployHref?: string;
	onDeploy?: () => void;
	/** The device's reason Deploy here can't run ("Offline since 11:00"). */
	gate?: Gate | null;
}

export interface MatrixCantHereCell {
	state: "cant_here";
	/** The device's own words. */
	reason: ReactNode;
}

export interface MatrixUnknownCell {
	state: "unknown";
	/** Every kind `AppUnknown` can put on a matrix cell (DM/model/app-plan.ts), plus `snapshot`: readable, but without an event list. */
	why: "locked" | "snapshot" | "nokeys" | "offline" | "notloaded" | "error";
	/** `offline`: unix seconds the device went offline. */
	since?: number;
	/** Unlock… / Diagnose / Restore keys…. */
	action?: ReactNode;
}

export interface MatrixNoAccessCell {
	state: "no_access";
}

export type MatrixCellProps =
	| MatrixServedCell
	| MatrixNotServedCell
	| MatrixCantHereCell
	| MatrixUnknownCell
	| MatrixNoAccessCell;

const CELL =
	"flex min-w-0 flex-col items-start gap-0.75 text-[12.5px] leading-[17px]";
const NONE = "inline-flex items-start gap-1 text-muted-foreground";

function Served(props: Readonly<MatrixServedCell>) {
	const { t } = useTranslation("devices");
	const name = (
		<>
			<PairedPins
				desired={props.desired}
				observed={props.observed}
				conv={props.conv}
			/>
			<span title={props.serviceId} className="min-w-0 truncate font-mono">
				{props.serviceId}
			</span>
		</>
	);
	const nameClass =
		"inline-flex max-w-full items-center gap-1.5 font-semibold text-foreground no-underline";
	return (
		<div data-matrix-cell={props.state} className={CELL}>
			{props.href ? (
				<a
					href={props.href}
					className={cx(nameClass, "hover:[&>.font-mono]:underline")}
				>
					{name}
				</a>
			) : (
				<span className={nameClass}>{name}</span>
			)}
			<span className="text-ink-2">{props.actual}</span>
			<span className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-muted-foreground">
				<span className="font-mono text-[11.5px]">{props.pin}</span>
				{props.target ? (
					<span className="text-info" title={props.targetTitle}>
						{t("view.matrix.target", "→ {{target}}", { target: props.target })}
					</span>
				) : props.note ? (
					<span
						data-pin-note={props.note.tone}
						className={
							props.note.tone === "info" ? "text-info" : "text-muted-foreground"
						}
					>
						{props.note.text}
					</span>
				) : (
					<span className="text-good">{t("view.matrix.newest", "newest")}</span>
				)}
				{props.tag ? <LatestTag /> : null}
			</span>
			{props.lines?.map((line) => (
				<span key={line} data-schedule-run="" className="text-muted-foreground">
					{line}
				</span>
			))}
			{props.port ? (
				<span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-muted-foreground">
					{props.tls ? (
						<Lock
							aria-label={t("view.matrix.tls", "Encrypted")}
							className="size-3"
						/>
					) : null}
					{props.port}
				</span>
			) : null}
			{props.state === "staged" && props.stagedVersion ? (
				<span className="flex flex-wrap items-center gap-1.5">
					<StatusChip tone="info" icon={Rocket}>
						{t("view.matrix.staged", "{{version}} staged", {
							version: props.stagedVersion,
						})}
					</StatusChip>
					{props.onActivate ? (
						<GatedAction gate={props.activateGate}>
							<DvButton size="xs" onClick={props.onActivate}>
								{t("view.matrix.activate", "Activate…")}
							</DvButton>
						</GatedAction>
					) : null}
				</span>
			) : null}
			{props.also?.length ? (
				<span className="text-muted-foreground">
					{t("view.matrix.also", "also {{services}}", {
						services: props.also.join(", "),
					})}
				</span>
			) : null}
		</div>
	);
}

function NotServed(props: Readonly<MatrixNotServedCell>) {
	const { t } = useTranslation("devices");
	const label = t("view.matrix.deployHere", "Deploy here");
	const button = props.deployHref ? (
		<DvButton size="xs" icon={Rocket} asChild>
			<a href={props.deployHref}>{label}</a>
		</DvButton>
	) : (
		<DvButton size="xs" icon={Rocket} onClick={props.onDeploy}>
			{label}
		</DvButton>
	);
	return (
		<div data-matrix-cell="not_served" className={CELL}>
			<span className={NONE}>{t("view.matrix.notServed", "Not served")}</span>
			{props.deployHref || props.onDeploy ? (
				<GatedAction gate={props.gate}>{button}</GatedAction>
			) : null}
		</div>
	);
}

function CantHere({ reason }: Readonly<MatrixCantHereCell>) {
	const { t } = useTranslation("devices");
	return (
		<div data-matrix-cell="cant_here" className={CELL}>
			<span className={cx(NONE, "text-ink-2")}>
				<Ban aria-hidden className="mt-0.5 size-3 shrink-0" />
				{t("view.matrix.cantHere", "Can't run here")}
			</span>
			<span className="text-xs text-muted-foreground">{reason}</span>
		</div>
	);
}

function NoAccess() {
	const { t } = useTranslation("devices");
	return (
		<div data-matrix-cell="no_access" className={CELL}>
			<span className={NONE}>
				<Ban aria-hidden className="mt-0.5 size-3 shrink-0" />
				{t("view.matrix.noAccess", "Shared for another app")}
			</span>
		</div>
	);
}

const UNKNOWN_ICON: Record<MatrixUnknownCell["why"], LucideIcon> = {
	locked: Lock,
	nokeys: KeyRound,
	snapshot: CircleDashed,
	offline: WifiOff,
	notloaded: CircleDashed,
	error: OctagonX,
};

const offlineText = (t: DevicesT, since: string | null) => {
	if (since === null)
		return t("devices:view.matrix.offline", "No status: the device is offline");
	return t("devices:view.matrix.offlineSince", "No status since {{time}}", {
		time: since,
	});
};

/** `since` = the formatted time for `offline`. A kind this client doesn't know reads "Status unknown". */
const unknownText = (
	t: DevicesT,
	why: MatrixUnknownCell["why"],
	since: string | null,
) => {
	if (why === "locked")
		return t("devices:view.matrix.locked", "Unknown until unlocked");
	if (why === "nokeys")
		return t("devices:view.matrix.noKeys", "No keys on this computer");
	if (why === "offline") return offlineText(t, since);
	if (why === "notloaded")
		return t("devices:view.matrix.notLoaded", "Not loaded yet");
	if (why === "error")
		return t("devices:view.matrix.error", "Couldn't read its status");
	if (why === "snapshot")
		return t(
			"devices:view.matrix.snapshot",
			"Unknown: the status snapshot has no event list",
		);
	return t("devices:enum.health.unknown", "Status unknown");
};

function Unknown({ why, since, action }: Readonly<MatrixUnknownCell>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const Icon = UNKNOWN_ICON[why] ?? CircleDashed;
	return (
		<div data-matrix-cell="unknown" data-why={why} className={CELL}>
			<span className={NONE}>
				<Icon
					aria-hidden
					className={cx(
						"mt-0.5 size-3 shrink-0",
						why === "error" && "text-critical",
					)}
				/>
				{unknownText(t, why, since === undefined ? null : time.at(since))}
			</span>
			{action}
		</div>
	);
}

/**
 * APP §2.10: one event × device cell. "Unknown" never means "not deployed";
 * a gated Deploy here stays visible with its reason (R7).
 */
export function MatrixCell(props: Readonly<MatrixCellProps>) {
	switch (props.state) {
		case "served":
		case "staged":
			return <Served {...props} />;
		case "not_served":
			return <NotServed {...props} />;
		case "cant_here":
			return <CantHere {...props} />;
		case "no_access":
			return <NoAccess />;
		default:
			return <Unknown {...props} />;
	}
}
