"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CirclePause,
	CircleSlash,
	Hourglass,
	Layers,
	LoaderCircle,
	Lock,
	type LucideIcon,
	OctagonX,
	RotateCcw,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import type { DevicesT } from "./area-context";
import { useAreaTime } from "./area-context";
import { type CheckState, Checklist } from "./checklist";
import { StepBar, type StepState } from "./meter";
import { StatusChip } from "./status-chip";
import { type ChipTone, cx } from "./tone";

/** APP §7.8 row states; `rolled_back` is a failed update that restored the previous version. */
export type FleetRowState =
	| "waiting"
	| "active"
	| "done"
	| "failed"
	| "rolled_back"
	| "blocked"
	| "skipped"
	| "held"
	| "not_started";

export interface FleetRolloutRow {
	id: string;
	device: string;
	service?: string;
	/** Translated phase labels, one mini step each. */
	phases: readonly string[];
	/** Index of the current (active) or failed phase. */
	step: number;
	state: FleetRowState;
	/** Overrides the default row sentence. */
	text?: ReactNode;
	/** Unix seconds the row finished, failed or rolled back. */
	at?: number;
	/** Failure sentence for failed and rolled-back rows. */
	reason?: string;
	/** Phase wording after "Failed while …" (lower-case form); the phase label otherwise. */
	failedWhile?: string;
	/** Version running again after a rollback. */
	from?: string;
	/** Version a finished update runs. */
	version?: string;
	progress?: { done: number; total: number };
	/** Row controls: Open, Retry this device, Skip, Unlock…, Continue with the rest, Stop here. */
	actions?: ReactNode;
}

export interface FleetSharedStep {
	label: ReactNode;
	state: "waiting" | "active" | "done" | "failed";
	source: "hub" | "local";
}

export interface FleetRolloutProps {
	title: string;
	/** "Invoice AI v1.5.0 · runs online · one device at a time". */
	subtitle?: ReactNode;
	/** `update` changes the done sentence and the foot note. */
	kind?: "deploy" | "update";
	rows: readonly FleetRolloutRow[];
	/** When a device fails, the rest wait for you (default) or continue. */
	stopOnFail?: boolean;
	/** The failed device the held rows wait for; the first failed row otherwise. */
	heldBy?: string;
	/** Runs once before the device rows (the offline copy, the approved definitions). */
	shared?: FleetSharedStep;
	/** Overrides the "k of n done" chip. */
	chip?: { tone: ChipTone; text: string };
	/** Sentence for done rows; defaults to "Deployed at …" / "Updated at …". */
	doneText?: (row: FleetRolloutRow) => ReactNode;
	/** The InlineResult once finished. */
	result?: ReactNode;
	stamp?: ReactNode;
	note?: ReactNode;
	className?: string;
}

const ROW_ICON: Record<FleetRowState, LucideIcon> = {
	waiting: Hourglass,
	active: LoaderCircle,
	done: CircleCheck,
	failed: OctagonX,
	rolled_back: RotateCcw,
	blocked: Lock,
	skipped: CircleSlash,
	held: CirclePause,
	not_started: CircleSlash,
};

const ROW_TEXT: Record<FleetRowState, string> = {
	waiting: "text-muted-foreground",
	active: "text-info",
	done: "text-good",
	failed: "text-critical",
	rolled_back: "text-critical",
	blocked: "text-locked",
	skipped: "text-muted-foreground",
	held: "text-warning",
	not_started: "text-muted-foreground",
};

const OPEN: readonly FleetRowState[] = ["waiting", "active", "blocked", "held"];
const FAILED: readonly FleetRowState[] = ["failed", "rolled_back"];

const miniState = (row: FleetRolloutRow, index: number): StepState => {
	if (row.state === "done") return "done";
	const current =
		row.state === "active"
			? "active"
			: FAILED.includes(row.state)
				? "fail"
				: null;
	if (!current || index > row.step) return "todo";
	return index < row.step ? "done" : current;
};

/** The header chip: "1 of 2 done", or "Failed on 1" (critical only when every device failed). */
export function fleetRolloutChip(
	t: DevicesT,
	rows: readonly Pick<FleetRolloutRow, "state">[],
): { tone: ChipTone; text: string; finished: boolean } {
	const total = rows.length;
	const done = rows.filter((row) => row.state === "done").length;
	const failed = rows.filter((row) => FAILED.includes(row.state)).length;
	const finished = !rows.some((row) => OPEN.includes(row.state));
	if (failed > 0) {
		return {
			tone: failed === total ? "critical" : "warning",
			text: t("devices:view.fleet.chipFailed", "Failed on {{count, number}}", {
				count: failed,
			}),
			finished,
		};
	}
	return {
		tone: finished ? "good" : "info",
		text: t(
			"devices:view.fleet.chipDone",
			"{{done, number}} of {{total, number}} done",
			{
				done,
				total,
			},
		),
		finished,
	};
}

const SHARED_CHECK: Record<FleetSharedStep["state"], CheckState> = {
	waiting: "pending",
	active: "active",
	done: "pass",
	failed: "fail",
};

const HEAD_ICON: Partial<Record<ChipTone, LucideIcon>> = {
	critical: OctagonX,
	warning: TriangleAlert,
};

const MUTED: readonly FleetRowState[] = ["waiting", "skipped", "not_started"];

interface SentenceContext {
	t: DevicesT;
	row: FleetRolloutRow;
	kind: "deploy" | "update";
	/** The current or failed phase label. */
	phase: string;
	/** Formatted `row.at`. */
	at: string;
	heldBy: string;
	/** Waiting rows: the device before in line, or null for the next one to start. */
	before: string | null;
}

const doneSentence = ({ t, row, at, kind }: SentenceContext) => {
	if (kind !== "update")
		return t(
			"devices:view.fleet.deployed",
			"Deployed at {{time}} · {{service}} running",
			{ time: at, service: row.service ?? row.device },
		);
	return row.version
		? t(
				"devices:view.fleet.updatedVersion",
				"Updated at {{time}} · {{version}}",
				{
					time: at,
					version: row.version,
				},
			)
		: t("devices:view.fleet.updated", "Updated at {{time}}", { time: at });
};

/** A failure reason as a clause: the templates end it with their own period. */
const clause = (reason: string | undefined) =>
	(reason ?? "").trim().replace(/[.\s]+$/, "");

const failedSentence = ({ t, row, phase }: SentenceContext) => {
	const params = {
		phase: row.failedWhile ?? phase,
		reason: clause(row.reason),
		device: row.device,
	};
	return params.reason
		? t(
				"devices:view.fleet.failed",
				"Failed while {{phase}}: {{reason}}. Nothing changed on {{device}}.",
				params,
			)
		: t(
				"devices:view.fleet.failedNoReason",
				"Failed while {{phase}}. Nothing changed on {{device}}.",
				params,
			);
};

/** "Rolled back at 14:03: the new version crashed." with whichever facts are known. */
const rolledBackLead = (t: DevicesT, time: string, reason: string) => {
	if (time && reason)
		return t(
			"devices:view.fleet.rolledBackAtReason",
			"Rolled back at {{time}}: {{reason}}.",
			{ time, reason },
		);
	if (time)
		return t("devices:view.fleet.rolledBackAt", "Rolled back at {{time}}.", {
			time,
		});
	if (reason)
		return t(
			"devices:view.fleet.rolledBackReason",
			"Rolled back: {{reason}}.",
			{
				reason,
			},
		);
	return t("devices:view.fleet.rolledBackPlain", "Rolled back.");
};

const rolledBackSentence = ({ t, row, at }: SentenceContext) => {
	const reason = clause(row.reason);
	if (at && reason && row.from)
		return t(
			"devices:view.fleet.rolledBack",
			"Rolled back at {{time}}: {{reason}}. {{from}} is running again.",
			{ time: at, reason, from: row.from },
		);
	return t("devices:view.fleet.rolledBackThen", "{{what}} {{running}}", {
		what: rolledBackLead(t, at, reason),
		running: row.from
			? t("devices:view.fleet.runningAgain", "{{from}} is running again.", {
					from: row.from,
				})
			: t(
					"devices:view.fleet.previousRunning",
					"The previous version is running again.",
				),
	});
};

const SENTENCE: Record<FleetRowState, (c: SentenceContext) => ReactNode> = {
	active: ({ t, row, phase }) =>
		row.progress
			? t(
					"devices:view.fleet.activeProgress",
					"{{phase}} · {{done, number}} of {{total, number}}",
					{ phase, done: row.progress.done, total: row.progress.total },
				)
			: t("devices:view.fleet.active", "{{phase}}…", { phase }),
	done: doneSentence,
	failed: failedSentence,
	rolled_back: rolledBackSentence,
	blocked: ({ t, row }) =>
		t("devices:view.fleet.blocked", "Waiting: unlock {{device}} to continue.", {
			device: row.device,
		}),
	skipped: ({ t, row }) =>
		t(
			"devices:view.fleet.skipped",
			"Skipped by you. Nothing changed on {{device}}.",
			{ device: row.device },
		),
	held: ({ t, heldBy }) =>
		heldBy
			? t(
					"devices:view.fleet.held",
					"Waiting: {{device}} failed. Continue with the rest or stop here.",
					{ device: heldBy },
				)
			: t(
					"devices:view.fleet.heldAny",
					"Waiting: a device failed. Continue with the rest or stop here.",
				),
	not_started: ({ t, heldBy }) =>
		heldBy
			? t(
					"devices:view.fleet.notStarted",
					"Not started: you stopped after {{device}} failed.",
					{ device: heldBy },
				)
			: t(
					"devices:view.fleet.notStartedAny",
					"Not started: you stopped after a device failed.",
				),
	waiting: ({ t, before }) =>
		before === null
			? t("devices:view.fleet.starting", "Starting…")
			: t("devices:view.fleet.inLine", "In line after {{device}}", {
					device: before,
				}),
};

const FOOT_WAIT = {
	update: (t: DevicesT) =>
		t(
			"devices:view.fleet.noteUpdateWait",
			"If an update rolls back on one device, the rest wait for you. Each device keeps its own settings; only the app version changes.",
		),
	deploy: (t: DevicesT) =>
		t(
			"devices:view.fleet.noteDeployWait",
			"If a device fails, the rest wait for you. Each device gets its own service, settings and cloud access.",
		),
};

const FOOT_CONTINUE = {
	update: (t: DevicesT) =>
		t(
			"devices:view.fleet.noteUpdateContinue",
			"If an update rolls back on one device, the rest continue. Each device keeps its own settings; only the app version changes.",
		),
	deploy: (t: DevicesT) =>
		t(
			"devices:view.fleet.noteDeployContinue",
			"If a device fails, the rest continue. Each device gets its own service, settings and cloud access.",
		),
};

interface FleetRowProps {
	row: FleetRolloutRow;
	sentence: ReactNode;
}

/*
 * One column plan for every row, with no content-sized track, so devices,
 * services, steps and sentences line up down the list whatever a row's
 * actions are. The step bars share a fixed column, so six to eight deploy
 * phases fit as well as four. Below 820 px of list width the sentence and the
 * actions move under the first line; below 520 px the service does too.
 */
const ROW =
	"grid grid-cols-[minmax(0,160px)_minmax(0,150px)_150px_minmax(220px,1fr)_minmax(0,240px)] items-center gap-x-3 gap-y-1.5 border-t border-hairline px-3 py-2 text-ui first:border-t-0 @max-[820px]/rf:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_120px] @max-[520px]/rf:grid-cols-[minmax(0,1fr)_120px]";
const ROW_DEVICE =
	"truncate font-mono @max-[520px]/rf:col-start-1 @max-[520px]/rf:row-start-1";
const ROW_SERVICE =
	"truncate font-mono @max-[520px]/rf:col-span-full @max-[520px]/rf:empty:hidden";
const ROW_STEPS =
	"flex gap-0.5 @max-[820px]/rf:col-start-3 @max-[820px]/rf:row-start-1 @max-[520px]/rf:col-start-2";
const ROW_SENTENCE =
	"inline-flex min-w-0 items-start gap-1 text-xs @max-[820px]/rf:col-span-full";
const ROW_ACTIONS =
	"flex flex-wrap justify-end gap-1.5 @max-[820px]/rf:col-span-full @max-[820px]/rf:justify-start @max-[820px]/rf:empty:hidden";

function FleetRow({ row, sentence }: Readonly<FleetRowProps>) {
	const Icon = ROW_ICON[row.state];
	return (
		<li
			data-device={row.device}
			data-state={row.state}
			className={cx(ROW, MUTED.includes(row.state) && "text-muted-foreground")}
		>
			<span className={ROW_DEVICE} title={row.device}>
				{row.device}
			</span>
			<span className={ROW_SERVICE} title={row.service}>
				{row.service}
			</span>
			<span aria-hidden className={ROW_STEPS}>
				{row.phases.map((phase, phaseIndex) => (
					<StepBar
						// biome-ignore lint/suspicious/noArrayIndexKey: phases are positional
						key={phaseIndex}
						title={phase}
						state={miniState(row, phaseIndex)}
						className="min-w-2 flex-1"
					/>
				))}
			</span>
			<span className={cx(ROW_SENTENCE, ROW_TEXT[row.state])}>
				<Icon
					aria-hidden
					className={cx(
						"mt-px size-3.25 shrink-0",
						row.state === "active" && "animate-spin motion-reduce:animate-none",
					)}
				/>
				<span className="min-w-0">{sentence}</span>
			</span>
			<span className={ROW_ACTIONS}>{row.actions}</span>
		</li>
	);
}

/** SPEC §4.10 generalised (APP §6.3): deploy or update across devices, one row per service. */
export function FleetRollout(props: Readonly<FleetRolloutProps>) {
	const { title, subtitle, rows, shared, stamp, result, className } = props;
	const kind = props.kind ?? "deploy";
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const summary = fleetRolloutChip(t, rows);
	const head = props.chip ?? summary;
	const headIcon = HEAD_ICON[head.tone];
	const firstOpen = rows.findIndex((row) => OPEN.includes(row.state));
	const heldBy =
		props.heldBy ??
		rows.find((row) => FAILED.includes(row.state))?.device ??
		"";
	const foot =
		props.note ??
		(props.stopOnFail === false ? FOOT_CONTINUE : FOOT_WAIT)[kind](t);

	const sentenceOf = (row: FleetRolloutRow, index: number): ReactNode => {
		if (row.text !== undefined) return row.text;
		if (row.state === "done" && props.doneText) return props.doneText(row);
		return SENTENCE[row.state]({
			t,
			row,
			kind,
			phase: row.phases[Math.min(row.step, row.phases.length - 1)] ?? "",
			at: row.at === undefined ? "" : time.clock(row.at),
			heldBy,
			before: index === firstOpen ? null : (rows[index - 1]?.device ?? ""),
		});
	};

	return (
		<section
			aria-label={title}
			data-fleet-rollout={kind}
			className={cx(
				"@container/rf flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-card px-4 py-3",
				className,
			)}
		>
			<header className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
				<Layers aria-hidden className="size-4 text-ink-2" />
				<b className="font-semibold">{title}</b>
				{subtitle ? (
					<span className="text-ui text-muted-foreground">{subtitle}</span>
				) : null}
				<StatusChip
					tone={head.tone}
					icon={headIcon ?? (summary.finished ? CircleCheck : LoaderCircle)}
					spin={!summary.finished && !headIcon}
					data-fleet-chip=""
				>
					{head.text}
				</StatusChip>
				{stamp ? (
					<>
						<span className="flex-1" />
						{stamp}
					</>
				) : null}
			</header>
			{shared ? (
				<Checklist
					items={[
						{
							id: "shared",
							state: SHARED_CHECK[shared.state],
							label: shared.label,
							source: shared.source,
						},
					]}
				/>
			) : null}
			<ul className="flex flex-col rounded-lg border border-hairline">
				{rows.map((row, index) => (
					<FleetRow key={row.id} row={row} sentence={sentenceOf(row, index)} />
				))}
			</ul>
			{result}
			<p className="max-w-[72ch] text-xs text-muted-foreground">{foot}</p>
		</section>
	);
}
