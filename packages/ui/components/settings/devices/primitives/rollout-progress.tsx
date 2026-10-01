"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	CircleCheck,
	CircleSlash,
	Hourglass,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	Rocket,
	RotateCcw,
} from "lucide-react";
import type { ReactNode } from "react";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "./area-context";
import { KeyValueList, KvRow } from "./key-value-list";
import { Meter, StepBar, type StepState } from "./meter";
import { StatusChip } from "./status-chip";
import { TONE_TEXT, type Tone, cx } from "./tone";

/** Display phase of one safe update; the screen maps the wire rollout (and its failure code) onto it. */
export type RolloutPhase =
	| "staged"
	| "validating"
	| "activating"
	| "rolling_back"
	| "healthy"
	| "rolled_back"
	| "not_applied"
	| "failed_stopped";

export interface RolloutTimes {
	/** Unix seconds per step: staged, checked, switched, done. */
	staged?: number;
	validated?: number;
	switched?: number;
	done?: number;
}

export interface RolloutProgressProps {
	phase: RolloutPhase;
	/** "settings v11" → "settings v12". */
	from: string;
	to: string;
	times?: RolloutTimes;
	/** Unix seconds by which the new version must be ready. */
	deadlineAt?: number;
	/** Unix seconds the new instances became ready. */
	stableSince?: number;
	/** Seconds the new version must stay healthy. */
	stabilizeSec?: number;
	instances?: { previous: number; candidate: number };
	/** "instance #0". */
	waitingFor?: string;
	/** The "If it fails" row; a default sentence otherwise. */
	ifFails?: ReactNode;
	/** End states: the failure-code sentence for "What happened". */
	outcome?: ReactNode;
	title?: string;
	stamp?: ReactNode;
	actions?: ReactNode;
	className?: string;
}

interface EndLook {
	failAt: number;
	tone: Tone;
	icon: LucideIcon;
}

const END: Partial<Record<RolloutPhase, EndLook>> = {
	rolled_back: { failAt: 3, tone: "warning", icon: RotateCcw },
	not_applied: { failAt: 1, tone: "warning", icon: CircleSlash },
	failed_stopped: { failAt: 3, tone: "critical", icon: OctagonX },
};

const ACTIVE_STEP: Partial<Record<RolloutPhase, number>> = {
	staged: 0,
	validating: 1,
	activating: 2,
	rolling_back: 3,
};

const stepState = (phase: RolloutPhase, index: number): StepState => {
	const end = END[phase];
	const pivot = end
		? end.failAt
		: phase === "healthy"
			? 4
			: (ACTIVE_STEP[phase] ?? 0);
	if (index < pivot) return "done";
	if (index > pivot) return "todo";
	return end ? "fail" : "active";
};

/** SPEC §4.9: one service's safe update. Countdowns tick with the area clock. */
export function RolloutProgress({
	phase,
	from,
	to,
	times = {},
	deadlineAt,
	stableSince,
	stabilizeSec = 10,
	instances,
	waitingFor,
	ifFails,
	outcome,
	title,
	stamp,
	actions,
	className,
}: Readonly<RolloutProgressProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const end = END[phase];

	const endLabel: Partial<Record<RolloutPhase, string>> = {
		rolled_back: enumLabel(t, "rollout", "rolled_back"),
		not_applied: enumLabel(t, "rollout", "failed"),
		failed_stopped: enumLabel(t, "rollout", "failed", {
			failureCode: "rollback_failed",
		}),
	};
	const labels = [
		enumLabel(t, "rollout", "staged"),
		enumLabel(t, "rollout", "validating"),
		enumLabel(t, "rollout", "activating"),
		phase === "rolling_back"
			? enumLabel(t, "rollout", "rolling_back")
			: enumLabel(t, "rollout", "healthy"),
	];
	const stepTimes = [times.staged, times.validated, times.switched, times.done];
	const steps = labels.map((label, index) => {
		const state = stepState(phase, index);
		const at = stepTimes[index];
		return {
			id: index,
			state,
			label: state === "fail" && end ? (endLabel[phase] ?? label) : label,
			time:
				state === "active"
					? t("view.rollout.now", "now")
					: state === "todo" || at === undefined
						? "–"
						: time.clock(at),
		};
	});

	const chip = end
		? {
				tone: end.tone,
				icon: end.icon,
				text: endLabel[phase] ?? "",
				spin: false,
			}
		: phase === "healthy"
			? {
					tone: "good" as Tone,
					icon: CircleCheck,
					text: enumLabel(t, "rollout", "healthy"),
					spin: false,
				}
			: {
					tone: (phase === "rolling_back" ? "warning" : "info") as Tone,
					icon: phase === "staged" ? Hourglass : LoaderCircle,
					text: labels[ACTIVE_STEP[phase] ?? 0],
					spin: phase !== "staged",
				};

	const instancesRow = instances ? (
		<KvRow label={t("view.rollout.instancesLabel", "Instances")}>
			{t(
				"view.rollout.instancesValue",
				"previous {{previous}} → new {{candidate}}",
				{
					previous: instances.previous,
					candidate: instances.candidate,
				},
			)}
		</KvRow>
	) : null;
	const ifFailsRow = (
		<KvRow label={t("view.rollout.ifFails", "If it fails")}>
			{ifFails ??
				t(
					"view.rollout.ifFailsDefault",
					"The device restores {{from}} and keeps the service running.",
					{ from },
				)}
		</KvRow>
	);

	const facts = (): ReactNode => {
		switch (phase) {
			case "healthy":
				return (
					<>
						<KvRow label={t("view.rollout.result", "Result")}>
							{times.done === undefined
								? t("view.rollout.resultNoTime", "{{to}} is running.", { to })
								: t(
										"view.rollout.resultAt",
										"Updated at {{time}}. {{to}} is running.",
										{
											time: time.clock(times.done),
											to,
										},
									)}
						</KvRow>
						<KvRow label={t("view.rollout.stayedHealthy", "Stayed healthy")}>
							{t(
								"view.rollout.stayedHealthyValue",
								"{{count}} s after the new version was ready",
								{ count: stabilizeSec },
							)}
						</KvRow>
						{instancesRow}
					</>
				);
			case "rolled_back":
				return (
					<>
						<KvRow label={t("view.rollout.whatHappened", "What happened")}>
							{outcome ??
								t(
									"view.rollout.rolledBackText",
									"The new version crashed after switching. The previous version is running again.",
								)}
						</KvRow>
						<KvRow label={t("view.rollout.running", "Running")}>{from}</KvRow>
						<KvRow label={t("view.rollout.next", "Next step")}>
							{t(
								"view.rollout.rolledBackNext",
								"Check the service logs for the crash, then deploy a fixed version.",
							)}
						</KvRow>
					</>
				);
			case "not_applied":
				return (
					<>
						<KvRow label={t("view.rollout.whatHappened", "What happened")}>
							{outcome ??
								t(
									"view.rollout.notAppliedText",
									"The new version failed its checks. The current version is still running.",
								)}
						</KvRow>
						<KvRow label={t("view.rollout.running", "Running")}>{from}</KvRow>
					</>
				);
			case "failed_stopped":
				return (
					<>
						<KvRow label={t("view.rollout.whatHappened", "What happened")}>
							{outcome ??
								t(
									"view.rollout.failedStoppedText",
									"The update failed and the previous version couldn't be restored. The service is stopped.",
								)}
						</KvRow>
						<KvRow label={t("view.rollout.next", "Next step")}>
							{t(
								"view.rollout.failedStoppedNext",
								"Start runs {{from}} again. Check the logs first.",
								{ from },
							)}
						</KvRow>
					</>
				);
			case "staged":
				return (
					<>
						<KvRow label={t("view.rollout.staged", "Staged")}>
							{times.staged === undefined ? "–" : time.clock(times.staged)}
						</KvRow>
						<KvRow label={t("view.rollout.expires", "Expires")}>
							{t(
								"view.rollout.expiresText",
								"Discarded if not activated within 24 h.",
							)}
						</KvRow>
						{ifFailsRow}
					</>
				);
			case "rolling_back":
				return (
					<>
						<KvRow label={t("view.rollout.whatHappened", "What happened")}>
							{outcome ??
								t(
									"view.rollout.rollingBackText",
									"The new version didn't stay healthy. The device is restoring {{from}}.",
									{ from },
								)}
						</KvRow>
						{instancesRow}
					</>
				);
			default: {
				const stable =
					stableSince === undefined
						? 0
						: Math.max(0, Math.min(stabilizeSec, time.nowS - stableSince));
				return (
					<>
						<KvRow label={t("view.rollout.deadline", "Time limit to start")}>
							{stableSince !== undefined && deadlineAt !== undefined ? (
								t(
									"view.rollout.deadlineMet",
									"Met at {{time}} · limit was {{limit}}",
									{
										time: time.clock(stableSince),
										limit: time.clock(deadlineAt),
									},
								)
							) : deadlineAt !== undefined ? (
								<>
									<b
										data-countdown=""
										className="font-mono font-semibold tabular-nums"
										title={time.abs(deadlineAt)}
									>
										{t("view.rollout.left", "{{time}} left", {
											time: time.countdown(deadlineAt),
										})}
									</b>{" "}
									{t("view.rollout.until", "· until {{time}}", {
										time: time.clock(deadlineAt),
									})}
								</>
							) : (
								"–"
							)}
						</KvRow>
						<KvRow
							label={t("view.rollout.stayHealthy", "Must stay healthy for")}
						>
							<span className="inline-flex flex-wrap items-center gap-2">
								<Meter
									className="w-24"
									label={t("view.rollout.stableOf", "{{done}} of {{total}} s", {
										done: Math.floor(stable),
										total: stabilizeSec,
									})}
									segments={[
										{ value: (stable / stabilizeSec) * 100, tone: "good" },
									]}
								/>
								<span className="font-mono tabular-nums">
									{stableSince === undefined
										? t("view.rollout.waitingFor", "waiting for {{what}}", {
												what:
													waitingFor ??
													t("view.rollout.newVersion", "the new version"),
											})
										: t("view.rollout.stableOf", "{{done}} of {{total}} s", {
												done: Math.floor(stable),
												total: stabilizeSec,
											})}
								</span>
							</span>
						</KvRow>
						{instancesRow}
						{ifFailsRow}
					</>
				);
			}
		}
	};

	return (
		<section
			aria-label={title ?? t("view.rollout.title", "Safe update")}
			data-rollout={phase}
			className={cx(
				"flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-card px-4 py-3",
				className,
			)}
		>
			<header className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
				<Rocket aria-hidden className="size-4 text-ink-2" />
				<b className="font-semibold">
					{title ?? t("view.rollout.title", "Safe update")}
				</b>
				<span className="font-mono text-xs text-muted-foreground">
					{from} → {to}
				</span>
				<StatusChip tone={chip.tone} icon={chip.icon} spin={chip.spin}>
					{chip.text}
				</StatusChip>
				<span className="flex-1" />
				{stamp}
			</header>
			<ol className="grid grid-cols-4 gap-2">
				{steps.map((step) => (
					<li
						key={step.id}
						data-s={step.state}
						aria-current={step.state === "active" ? "step" : undefined}
						className="grid min-w-0 content-start gap-1 text-xs"
					>
						<StepBar
							state={step.state}
							failTone={end?.tone === "warning" ? "warning" : "critical"}
						/>
						<span
							className={cx(
								"flex min-w-0 items-start gap-1 font-medium",
								step.state === "active" && "font-semibold text-info",
								step.state === "todo" && "text-muted-foreground",
								step.state === "fail" &&
									end &&
									cx("font-semibold", TONE_TEXT[end.tone]),
							)}
						>
							{step.state === "done" ? (
								<Check aria-hidden className="mt-0.5 size-3 shrink-0" />
							) : step.state === "fail" && end ? (
								<end.icon aria-hidden className="mt-0.5 size-3 shrink-0" />
							) : null}
							<span className="min-w-0">{step.label}</span>
						</span>
						<span className="font-mono text-muted-foreground tabular-nums">
							{step.time}
						</span>
					</li>
				))}
			</ol>
			<KeyValueList>{facts()}</KeyValueList>
			{actions ? (
				<div className="flex flex-wrap items-start gap-2">{actions}</div>
			) : null}
		</section>
	);
}
