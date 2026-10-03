"use client";

import { useTranslation } from "@flow-like/locales";
import { CirclePlay, Square } from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { formatCountdown } from "../../../../lib/date";
import { runPayloadProblem } from "../../../../lib/device-management/event-run";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	EventForm,
	FixAction,
	GateFailure,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	type FieldProblem,
	type FieldValue,
	formPayload,
	formSupport,
	seedValues,
} from "../../../../lib/event-form";
import { currentApproval } from "../cloud/cloud-model";
import { useDeviceApprovals } from "../cloud/use-cloud";
import { errorCopy } from "../copy/error-copy";
import { gateCopy } from "../copy/gate-copy";
import type { OverlaySheetProps } from "../overlays/area-overlays";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { type Gate, GateNotice, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "../workspace/device-workspace-provider";
import type { RunNowRequest } from "../workspace/overlay-store";
import { serviceGateExtra } from "../workspace/service-commands";
import type { GateTarget } from "../workspace/use-attention";
import { useServiceView } from "../workspace/use-fleet";
import { useFixAction, useGate } from "../workspace/use-gate";
import {
	type RunNames,
	cap,
	payloadProblemSentence,
	rejectionSentence,
	supportSentence,
	versionText,
} from "./run-copy";
import { RunFields } from "./run-fields";
import { RunResult } from "./run-result";
import {
	type RunEntry,
	type RunLookup,
	type RunStore,
	runsOf,
} from "./run-store";
import { type FormRead, useEventForm } from "./use-event-form";

export interface RunNowSheetProps extends OverlaySheetProps, RunNowRequest {}

type Values = Record<string, FieldValue>;

/** What "Run again" fills in, in this page's memory only and without the fields marked sensitive. */
const keptInputs = new Map<string, Values>();
const KEPT_MAX = 32;

function keep(operationId: string, form: EventForm, values: Values) {
	const kept: Values = {};
	for (const field of form.fields) {
		const value = values[field.name];
		if (!field.sensitive && value !== undefined) kept[field.name] = value;
	}
	keptInputs.set(operationId, kept);
	for (const oldest of [...keptInputs.keys()].slice(0, -KEPT_MAX))
		keptInputs.delete(oldest);
}

/** A form's defaults, with what was entered before for the fields it still has. */
function withDefaults(form: EventForm, kept: Values) {
	const values: Values = seedValues(form.fields);
	for (const field of form.fields) {
		const value = kept[field.name];
		if (value !== undefined) values[field.name] = value;
	}
	return values;
}

/** The fields the device refused in a run that ended with `invalid_fields`. */
function refusedIn(entry: RunEntry | undefined) {
	const phase = entry?.phase;
	const none: readonly string[] = [];
	if (phase?.kind !== "ended" || phase.run.code !== "invalid_fields")
		return none;
	return phase.run.fields ?? none;
}

/** What `run_event` is checked against for one service. */
export function runGateTarget(
	serviceId: string,
	view: ServiceView | undefined,
): GateTarget {
	return {
		placementId: serviceId,
		...(view?.projectId ? { projectId: view.projectId } : {}),
		labels: { service: serviceId },
		extra: serviceGateExtra(view),
	};
}

function useRunEntry(
	store: RunStore,
	operationId: string | undefined,
): RunEntry | undefined {
	const read = useCallback(
		() => (operationId ? store.get(operationId) : undefined),
		[store, operationId],
	);
	return useManagerValue(store.subscribe, read);
}

/** The run the sheet shows; one started earlier reads as being read until the store has it. */
function useShownRun(
	store: RunStore,
	operationId: string | undefined,
	ids: Omit<RunLookup, "operationId">,
) {
	const stored = useRunEntry(store, operationId);
	if (stored || !operationId) return stored;
	const reading: RunEntry = {
		operationId,
		...ids,
		phase: { kind: "reading" },
		since: 0,
	};
	return reading;
}

/** Seconds on a clock that ticks while `active`. */
function useTicking(active: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!active) return;
		setNow(Date.now());
		const timer = setInterval(() => setNow(Date.now()), 1_000);
		return () => clearInterval(timer);
	}, [active]);
	return now / 1000;
}

function RunGate({
	gate,
	onFix,
}: Readonly<{ gate: GateFailure; onFix(fix: FixAction): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const copy = gateCopy(t, gate, time);
	const { fix } = gate;
	return (
		<GateNotice
			kind={gate.kind}
			title={copy.title}
			text={copy.text}
			have={copy.have}
			actions={
				fix && copy.fix ? (
					<DvButton size="sm" onClick={() => onFix(fix)}>
						{copy.fix}
					</DvButton>
				) : undefined
			}
		/>
	);
}

interface ConsequenceFacts {
	names: RunNames;
	source: ServiceView["source"] | undefined;
	spending: boolean;
	form: boolean;
}

function consequenceRows(
	t: DevicesT,
	{ names, source, spending, form }: ConsequenceFacts,
): ConsequenceRows {
	const who =
		source === "online"
			? t(
					"devices:runNow.conseq.online",
					"It runs with {{service}}'s cloud access, not as you.",
					names,
				)
			: source === "offline"
				? t(
						"devices:runNow.conseq.local",
						"It uses {{device}}'s own copy of the app's data.",
						names,
					)
				: t(
						"devices:runNow.conseq.unknownSource",
						"It runs as {{service}}, not as you.",
						names,
					);
	const limit = spending
		? ` ${t(
				"devices:runNow.conseq.spending",
				"It can use hosted models within {{service}}'s spending limit.",
				names,
			)}`
		: "";
	return {
		what: t(
			"devices:runNow.conseq.what",
			"Runs the flow once on {{device}}.",
			names,
		),
		who: `${who}${limit}`,
		...(form
			? {
					stays: t(
						"devices:runNow.recorded",
						"What you enter becomes the run's input. {{service}} keeps it in its run records on {{device}}.",
						names,
					),
				}
			: {}),
		when: t(
			"devices:runNow.conseq.when",
			"Now. A running instance of {{service}} takes it within 30 seconds, or it doesn't run.",
			names,
		),
		undo: {
			reversible: null,
			text: t("devices:runNow.conseq.undo", "No."),
		},
	};
}

interface FormBodyProps {
	form: EventForm;
	names: RunNames;
	values: Values;
	problems: Record<string, FieldProblem>;
	refused: readonly string[];
	payloadProblem: "too_large" | "too_many" | null;
	consequence: ConsequenceRows;
	onChange(name: string, value: FieldValue): void;
	onEndpoint(): void;
}

function FormBody({
	form,
	names,
	values,
	problems,
	refused,
	payloadProblem,
	consequence,
	onChange,
	onEndpoint,
}: Readonly<FormBodyProps>) {
	const { t } = useTranslation("devices");
	const support = formSupport(form);
	return (
		<>
			{form.description ? (
				<p className="text-ui text-ink-2">{cap(form.description)}</p>
			) : null}
			{!support.ok ? (
				<StateView
					kind="unsupported"
					title={supportSentence(t, support.reason)}
					actions={
						support.reason === "unknown_field" ? undefined : (
							<DvButton size="sm" onClick={onEndpoint}>
								{t("runNow.support.endpoint", "Open Endpoint")}
							</DvButton>
						)
					}
				/>
			) : form.fields.length ? (
				<RunFields
					fields={form.fields}
					values={values}
					problems={problems}
					refused={refused}
					device={names.device}
					onChange={onChange}
				/>
			) : (
				<p data-run-no-input="" className="text-ui text-muted-foreground">
					{t("runNow.noInput", "This action takes no input.")}
				</p>
			)}
			{payloadProblem ? (
				<InlineResult tone="critical">
					{payloadProblemSentence(t, payloadProblem)}
				</InlineResult>
			) : null}
			{support.ok ? (
				<ConsequencePreview
					compact
					rows={consequence}
					labels={{ who: t("runNow.conseq.whoLabel", "How it runs") }}
				/>
			) : null}
		</>
	);
}

/** Once the form is there, its first field takes the focus, or Run for an action (the sheet opened on its close button). */
function useFocusFirstField(
	body: RefObject<HTMLDivElement | null>,
	form: EventForm | undefined,
) {
	useEffect(() => {
		const root = body.current;
		if (!form || !root) return;
		const first =
			root.querySelector<HTMLElement>(
				"[data-run-field] input, [data-run-field] textarea, [data-run-field] button",
			) ??
			root
				.closest("[role=dialog]")
				?.querySelector<HTMLElement>("[data-dv-primary]:not(:disabled)");
		first?.focus();
	}, [body, form]);
}

function FormState({
	read,
	names,
	onReload,
}: Readonly<{ read: FormRead; names: RunNames; onReload(): void }>) {
	const { t } = useTranslation("devices");
	switch (read.state) {
		case "idle":
		case "loading":
			return (
				<StateView
					kind="loading"
					title={t(
						"runNow.loading",
						"Reading the form from {{device}}…",
						names,
					)}
				/>
			);
		case "unsupported":
			return (
				<StateView
					kind="unsupported"
					title={rejectionSentence(t, "unsupported", names)}
				/>
			);
		case "changed":
			return (
				<StateView
					kind="notloaded"
					title={rejectionSentence(t, "revision_conflict", names)}
					actions={
						<DvButton size="sm" onClick={onReload}>
							{t("runNow.reload", "Reload the form")}
						</DvButton>
					}
				/>
			);
		case "failed":
			return (
				<StateView
					kind="error"
					title={t(
						"runNow.readFailed",
						"Couldn't read the form from {{device}}",
						names,
					)}
					text={errorCopy(t, read.failure.code)}
					actions={
						<DvButton size="sm" onClick={onReload}>
							{t("runNow.tryAgain", "Try again")}
						</DvButton>
					}
				/>
			);
		default:
			return null;
	}
}

function OpenRun({
	entry,
	names,
	onStop,
}: Readonly<{ entry: RunEntry; names: RunNames; onStop(): void }>) {
	const { t } = useTranslation("devices");
	const { phase } = entry;
	const running = phase.kind === "open" && phase.run.run === "running";
	const now = useTicking(running);
	if (phase.kind !== "open") return null;
	const startedAt = phase.run.startedAt ?? entry.since / 1000;
	const line = running
		? t("runNow.progress.running", "Running · {{elapsed}}", {
				elapsed: formatCountdown(now - startedAt),
			})
		: t("runNow.progress.waiting", "Waiting for {{device}}", names);
	return (
		<div className="flex flex-wrap items-center gap-2">
			<InlineResult tone="info" className="flex-1">
				{line}
			</InlineResult>
			{running ? (
				<DvButton
					size="sm"
					variant="danger-ghost"
					icon={Square}
					busy={entry.stopping}
					onClick={onStop}
				>
					{entry.stopping
						? t("runNow.stopping", "Stopping…")
						: t("runNow.stop", "Stop this run")}
				</DvButton>
			) : null}
		</div>
	);
}

function RunNotes({
	entry,
	names,
}: Readonly<{ entry: RunEntry; names: RunNames }>) {
	const { t } = useTranslation("devices");
	const notes: { key: string; tone: "critical" | "unknown"; text: string }[] =
		[];
	if (entry.stopFailure)
		notes.push({
			key: "stop",
			tone: "critical",
			text: t("runNow.stopFailed", "Couldn't stop the run. {{reason}}", {
				reason: errorCopy(t, entry.stopFailure.code),
			}),
		});
	if (entry.readFailure && entry.paused)
		notes.push({
			key: "paused",
			tone: "unknown",
			text: t(
				"runNow.paused",
				"Lost touch with the run. It may still be running on {{device}}. {{reason}}",
				{ ...names, reason: errorCopy(t, entry.readFailure.code) },
			),
		});
	else if (entry.paused)
		notes.push({
			key: "locked",
			tone: "unknown",
			text: t(
				"runNow.pausedLocked",
				"Reading stopped while {{device}} is locked. The run goes on there.",
				names,
			),
		});
	else if (entry.readFailure)
		notes.push({
			key: "read",
			tone: "unknown",
			text: t(
				"runNow.readRetry",
				"Couldn't read the run just now; trying again. {{reason}}",
				{ reason: errorCopy(t, entry.readFailure.code) },
			),
		});
	return (
		<>
			{notes.map((note) => (
				<InlineResult key={note.key} tone={note.tone}>
					{note.text}
				</InlineResult>
			))}
		</>
	);
}

function RunPhaseView({
	entry,
	names,
	onStop,
}: Readonly<{ entry: RunEntry; names: RunNames; onStop(): void }>) {
	const { t } = useTranslation("devices");
	const { phase } = entry;
	const region = useRef<HTMLDivElement>(null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: each step moves the focus here, as the control that started it is gone
	useEffect(() => {
		region.current?.focus({ preventScroll: true });
	}, [phase.kind]);
	const lines: Record<
		Exclude<RunEntry["phase"]["kind"], "open" | "ended">,
		ReactNode
	> = {
		sending: (
			<InlineResult tone="info">
				{t("runNow.progress.sent", "Sent")}
			</InlineResult>
		),
		reading: (
			<InlineResult tone="info">
				{t(
					"runNow.progress.reading",
					"Reading the run from {{device}}…",
					names,
				)}
			</InlineResult>
		),
		rejected:
			phase.kind === "rejected" ? (
				<InlineResult tone="critical">
					{rejectionSentence(t, phase.code, names, phase.rejection.error)}
				</InlineResult>
			) : null,
		not_sent:
			phase.kind === "not_sent" ? (
				<InlineResult tone="critical">
					{t(
						"runNow.notSent",
						"It didn't reach {{device}}, so nothing ran. {{reason}}",
						{ ...names, reason: errorCopy(t, phase.failure.code) },
					)}
				</InlineResult>
			) : null,
		no_reply: (
			<InlineResult tone="unknown">
				{t(
					"runNow.noReply",
					"No reply from {{device}}. The run may have started: check before you run it again.",
					names,
				)}
			</InlineResult>
		),
		unknown: (
			<InlineResult tone="unknown">
				{t(
					"runNow.unknownRun",
					"{{device}} has no record of this run. It didn't arrive, or it is more than a day old.",
					names,
				)}
			</InlineResult>
		),
	};
	return (
		<div
			ref={region}
			tabIndex={-1}
			data-run-phase={phase.kind}
			data-run-state={
				phase.kind === "open" || phase.kind === "ended"
					? phase.run.run
					: undefined
			}
			className="flex min-w-0 flex-col gap-2.5 outline-none"
		>
			{phase.kind === "open" ? (
				<OpenRun entry={entry} names={names} onStop={onStop} />
			) : phase.kind === "ended" ? (
				<RunResult run={phase.run} names={names} />
			) : (
				lines[phase.kind]
			)}
			<RunNotes entry={entry} names={names} />
		</div>
	);
}

interface FootProps {
	entry: RunEntry | undefined;
	form: EventForm | undefined;
	runGate: Gate | null;
	canRun: boolean;
	onClose(): void;
	onRun(): void;
	onAgain(): void;
	onCheck(): void;
	onReload(): void;
}

function SheetFoot({
	entry,
	form,
	runGate,
	canRun,
	onClose,
	onRun,
	onAgain,
	onCheck,
	onReload,
}: Readonly<FootProps>) {
	const { t } = useTranslation("devices");
	const close = (
		<DvButton onClick={onClose}>
			{entry ? t("runNow.close", "Close") : t("runNow.cancel", "Cancel")}
		</DvButton>
	);
	if (!entry)
		return (
			<>
				{close}
				<GatedAction gate={runGate}>
					<DvButton
						variant="primary"
						icon={CirclePlay}
						disabled={!canRun}
						onClick={onRun}
					>
						{t("runNow.run", "Run")}
					</DvButton>
				</GatedAction>
			</>
		);
	const { phase } = entry;
	const checkable =
		phase.kind === "no_reply" || (phase.kind === "open" && entry.paused);
	const conflict =
		phase.kind === "rejected" && phase.code === "revision_conflict";
	const again =
		phase.kind === "ended" ||
		phase.kind === "not_sent" ||
		phase.kind === "unknown" ||
		(phase.kind === "rejected" && !conflict);
	return (
		<>
			{close}
			{checkable ? (
				<DvButton variant="primary" onClick={onCheck}>
					{t("runNow.checkAgain", "Check again")}
				</DvButton>
			) : null}
			{conflict ? (
				<DvButton variant="primary" onClick={onReload}>
					{t("runNow.reload", "Reload the form")}
				</DvButton>
			) : null}
			{again ? (
				<GatedAction gate={runGate}>
					<DvButton variant="primary" icon={CirclePlay} onClick={onAgain}>
						{form?.fields.length
							? t("runNow.again", "Run again…")
							: t("runNow.againAction", "Run again")}
					</DvButton>
				</GatedAction>
			) : null}
		</>
	);
}

/** "Run now…" (design R2 §6.5): one quick action or form a service runs, started by a person. */
export function RunNowSheet({
	deviceId,
	serviceId,
	eventId,
	operationId: shown,
	onNavigate,
	onClose,
}: Readonly<RunNowSheetProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const store = runsOf(workspace);
	const fix = useFixAction();
	const { service, device } = useServiceView(deviceId, serviceId);
	const names = useMemo<RunNames>(
		() => ({
			device: device ? deviceName(device.row) : deviceId.slice(0, 8),
			service: serviceId,
		}),
		[device, deviceId, serviceId],
	);
	const target = useMemo(
		() => runGateTarget(serviceId, service),
		[serviceId, service],
	);
	const gate = useGate("run_event", deviceId, target);
	const approvals = useDeviceApprovals(deviceId);
	const spending =
		currentApproval(approvals.rows, serviceId)?.limit?.state === "active";
	const [operationId, setOperationId] = useState(shown);
	const entry = useShownRun(store, operationId, {
		deviceId,
		serviceId,
		eventId,
	});
	const { read, reload } = useEventForm({
		deviceId,
		serviceId,
		eventId,
		features: device?.features,
		enabled: gate.ok,
	});
	const form = read.state === "ok" ? read.form : undefined;
	const [values, setValues] = useState<Values>(
		() => (shown ? keptInputs.get(shown) : undefined) ?? {},
	);
	const [problems, setProblems] = useState<Record<string, FieldProblem>>({});
	const [refused, setRefused] = useState<readonly string[]>([]);
	const [payloadProblem, setPayloadProblem] = useState<
		"too_large" | "too_many" | null
	>(null);
	const seeded = useRef<EventForm | null>(null);
	const current = useRef(operationId);
	current.current = operationId;
	const body = useRef<HTMLDivElement>(null);
	useFocusFirstField(body, form);

	useEffect(() => {
		if (!form || seeded.current === form) return;
		seeded.current = form;
		setValues((kept) => withDefaults(form, kept));
	}, [form]);

	useEffect(() => {
		if (shown) store.view({ operationId: shown, deviceId, serviceId, eventId });
	}, [store, shown, deviceId, serviceId, eventId]);

	useEffect(
		() => () => {
			if (current.current) store.forgetOutput(current.current);
		},
		[store],
	);

	const gateFailure = gate.ok ? null : gate;
	const runGate = useMemo<Gate | null>(
		() =>
			gateFailure
				? {
						kind: gateFailure.kind,
						reason: gateCopy(t, gateFailure, time).inline,
					}
				: null,
		[gateFailure, t, time],
	);
	const onFix = (action: FixAction) => {
		const outcome = fix(action);
		if (outcome.kind !== "navigate") return;
		onClose();
		onNavigate(outcome.route);
	};
	const openEndpoint = () => {
		onClose();
		onNavigate({ screen: "service", deviceId, serviceId, tab: "endpoint" });
	};

	const run = () => {
		if (!form) return;
		const result = formPayload(form.fields, values);
		if (!result.ok) {
			setProblems(result.problems);
			return;
		}
		const payload = form.fields.length ? result.payload : undefined;
		const problem = runPayloadProblem(payload);
		setProblems({});
		setPayloadProblem(problem);
		if (problem) return;
		setRefused([]);
		const started = store.start({
			deviceId,
			deviceName: names.device,
			serviceId,
			...(service?.projectId ? { projectId: service.projectId } : {}),
			eventId,
			expectedRevision: form.config_revision,
			...(payload ? { payload } : {}),
		});
		keep(started, form, values);
		setOperationId(started);
	};
	const backToForm = () => {
		if (operationId) store.forgetOutput(operationId);
		setOperationId(undefined);
	};
	const again = () => {
		setRefused(refusedIn(entry));
		backToForm();
		if (!form) reload();
	};
	const reloadForm = () => {
		backToForm();
		reload();
	};

	const support = form ? formSupport(form) : null;
	const canRun = gate.ok && Boolean(form && support?.ok);
	const consequence = consequenceRows(t, {
		names,
		source: service?.source,
		spending,
		form: (form?.fields.length ?? 0) > 0,
	});
	const title = form
		? t("runNow.title", "Run {{event}} on {{device}}", {
				event: cap(form.name || eventId, 120),
				device: names.device,
			})
		: t("runNow.titleFallback", "Run an action or form on {{device}}", names);
	const sub = form
		? t("runNow.sub", "{{service}} · event {{event}} · flow {{flow}}", {
				service: serviceId,
				event: versionText(form.event_version),
				flow: versionText(form.board_version),
			})
		: serviceId;
	const open = entry?.phase.kind === "open" || entry?.phase.kind === "sending";

	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={CirclePlay}
			title={title}
			sub={sub}
			closeOnOutside={!open}
			footNote={
				open
					? t(
							"runNow.closeKeepsRunning",
							"Closing doesn't stop the run. Its outcome stays under Activity.",
						)
					: undefined
			}
			foot={
				<SheetFoot
					entry={entry}
					form={form}
					runGate={entry ? runGate : null}
					canRun={canRun}
					onClose={onClose}
					onRun={run}
					onAgain={again}
					onCheck={() => operationId && store.check(operationId)}
					onReload={reloadForm}
				/>
			}
		>
			<div
				ref={body}
				data-run-now-sheet={eventId}
				className="flex min-w-0 flex-col gap-3.5"
			>
				{entry ? (
					<RunPhaseView
						entry={entry}
						names={names}
						onStop={() => operationId && store.stop(operationId)}
					/>
				) : gateFailure ? (
					<RunGate gate={gateFailure} onFix={onFix} />
				) : form ? (
					<FormBody
						form={form}
						names={names}
						values={values}
						problems={problems}
						refused={refused}
						payloadProblem={payloadProblem}
						consequence={consequence}
						onChange={(name, value) =>
							setValues((now) => ({ ...now, [name]: value }))
						}
						onEndpoint={openEndpoint}
					/>
				) : (
					<FormState read={read} names={names} onReload={reload} />
				)}
			</div>
		</DvSheet>
	);
}
