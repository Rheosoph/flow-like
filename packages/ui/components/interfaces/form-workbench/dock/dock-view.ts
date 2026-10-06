/*
 * What the dock reads off the session state (pure: no React, no `t`): the status line, which run Stop acts on, the Run
 * button's label and cap state, the after-run line and the rows of the "Per run" panel (spec M1, M5).
 */
import {
	type AfterRunRow,
	BUSY_RUN_STATUSES,
	type DockLine,
	type DockProps,
	FORM_LIMITS,
	type FieldKey,
	type FormSessionState,
	LIVE_RUN_STATUSES,
	type LeftOutFile,
	type RunEntry,
	type RunStatus,
	type ShortWords,
	type WorkbenchLayout,
} from "../contracts";
import { dockLine, dockLineInputOf } from "../model/dock";
import { targets } from "../model/fields";
import { activePresetOf, comparedRunOf, perRunNamesOf } from "../model/markers";
import { afterRunRows, filesAndDates } from "../model/per-run";
import { foldText, isSecretField, looksSecret } from "../model/secrets";
import { blockedNames, missingCount } from "../model/validate";
import { type IsSecret, diffValues, isEmpty, valueAt } from "../model/values";
import { stageRunOf } from "../run/run-view";
import { RUN_STATUS_SPINS } from "../status-look";

const LIVE: ReadonlySet<RunStatus> = new Set(LIVE_RUN_STATUSES);
const BUSY: ReadonlySet<RunStatus> = new Set(BUSY_RUN_STATUSES);

/** A field or value kept out of sight ("••••"): flagged, "Don't save", or looking like a key. */
export const secretCheckOf =
	(noSave: readonly string[]): IsSecret =>
	(field, value) =>
		isSecretField(field, noSave) || looksSecret(value);

/**
 * The dock's one status line (`dockLine`): the model helpers fill what the session state does not carry. A form without
 * fields never queues, so a hold means nothing there and is left out.
 */
export function dockLineOf(
	state: FormSessionState,
	words: ShortWords,
): DockLine {
	const { fields, host } = state.form;
	const blocked = blockedNames(fields, host);
	const compared = comparedRunOf(state);
	const comparedChanges = compared
		? diffValues(fields, compared.copy.values, state.rail.values, {
				skip: perRunNamesOf(state),
				words,
				isSecret: secretCheckOf(state.memory.prefs.noSave),
			}).length
		: null;
	const missing = missingCount(fields, state.rail.values, blocked);
	const read =
		fields.length === 0 && state.queue.hold
			? { ...state, queue: { ...state.queue, hold: null } }
			: state;
	return dockLine(dockLineInputOf(read, { comparedChanges, missing, blocked }));
}

/** The run Stop acts on: the one on the stage while it can still be stopped or taken out of the queue. */
export function stopTargetOf(state: FormSessionState): RunEntry | null {
	const run = stageRunOf(state);
	return run && LIVE.has(run.status) ? run : null;
}

/** A form without fields at its cap: its Run waits for a run to end (spec M5, `flpQuickCapText`). */
export function quickCapOf(state: FormSessionState) {
	const { cap } = state.queue;
	if (state.form.fields.length > 0 || cap < 0) return null;
	const running = state.runs.filter((run) => BUSY.has(run.status)).length;
	return running >= cap ? { running } : null;
}

export type RunLabelKind = "run" | "runAgain";

/** "Run" until a run has ended, then "Run again" on a form's own Run for a form without fields (strip, phone). */
export function runLabelKindOf(
	state: FormSessionState,
	variant: DockProps["variant"],
): RunLabelKind {
	const ended = state.runs.some((run) => !LIVE.has(run.status));
	if (variant === "strip") return ended ? "runAgain" : "run";
	const bare = state.form.fields.length === 0;
	return variant === "phone" && bare && ended ? "runAgain" : "run";
}

export type PhoneLine =
	| DockLine
	| { readonly kind: "capped"; readonly running: number };

const PHONE_KINDS: ReadonlySet<DockLine["kind"]> = new Set([
	"sending",
	"problems",
	"hold",
	"message",
	"failure",
	"question",
	"queue",
	"blocked",
]);

/**
 * What the narrow layout shows above its dock bar: problems, sending, a message, an unseen failure, the hold, the
 * queue, the questions, a blocked file field and a refused press of a form at its cap. Never "Ready" or counts.
 */
export function phoneLineOf(
	state: FormSessionState,
	line: DockLine,
): PhoneLine | null {
	const capped = quickCapOf(state);
	if (capped && (line.kind === "queue" || line.kind === "ready"))
		return { kind: "capped", running: capped.running };
	return PHONE_KINDS.has(line.kind) ? line : null;
}

/** The Output segment's sign: a spinner while work goes on, a dot for a result or question waiting on Inputs. */
export function outputSignOf(
	state: FormSessionState,
): "spinner" | "dot" | null {
	if (state.runs.some((run) => RUN_STATUS_SPINS[run.status])) return "spinner";
	const waiting =
		state.view.outputUnseen ||
		state.runs.some((run) => run.status === "asking");
	return waiting && state.view.pane === "inputs" ? "dot" : null;
}

export interface AfterRunView {
	readonly show: boolean;
	/** Fields that are per run now (this person's and the current series'), by name. */
	readonly names: readonly string[];
	readonly keyboardIcon: boolean;
}

/** The per-run names that are fields of this form (settings of fields the form no longer has are ignored). */
export function knownPerRunNames(state: FormSessionState) {
	const known = new Set(state.form.fields.map((field) => field.name));
	return perRunNamesOf(state).filter((name) => known.has(name));
}

/** The after-run line (spec M1 b): fields, the split layout, and the setting introduced on this device. */
export function afterRunOf(
	state: FormSessionState,
	layout: WorkbenchLayout,
): AfterRunView {
	const { fields } = state.form;
	const names = knownPerRunNames(state);
	const waiting = Object.values(state.rail.nextFiles).some(
		(files) => files.length > 0,
	);
	const introduced =
		state.memory.prefs.introduced || names.length > 0 || waiting;
	return {
		show: fields.length > 0 && layout.split && introduced,
		names,
		keyboardIcon:
			fields.length >= FORM_LIMITS.shortcutsIconFromFields &&
			layout.finePointer,
	};
}

/** The rows of the "Per run" panel: checked state and what each field goes back to under the active preset. */
export function perRunRowsOf(
	state: FormSessionState,
	names: readonly string[],
	words: ShortWords,
): readonly AfterRunRow[] {
	const { fields } = state.form;
	const nextField =
		fields.find((field) => (state.rail.nextFiles[field.name]?.length ?? 0) > 0)
			?.name ?? null;
	return afterRunRows(fields, names, nextField, activePresetOf(state), words);
}

/** The rows whose label holds the typed text, ignoring case and accents. */
export function matchingRows(
	rows: readonly AfterRunRow[],
	query: string,
): readonly AfterRunRow[] {
	const needle = foldText(query.trim());
	return needle === ""
		? rows
		: rows.filter((row) => foldText(row.label).includes(needle));
}

/** "Make files and dates per run" is offered while nothing is per run and the form has such fields. */
export const makeOffered = (
	state: FormSessionState,
	names: readonly string[],
) => names.length === 0 && filesAndDates(state.form.fields).length > 0;

/**
 * The fields the ▲▼ buttons visit, in form order: the ones with a message after a press, or the required ones still
 * empty (blocked and unsupported fields are no stop).
 */
export function stepKeysOf(
	state: FormSessionState,
	line: DockLine,
): readonly FieldKey[] {
	const { fields, host } = state.form;
	if (line.kind === "problems")
		return targets(fields)
			.filter((target) => state.rail.problems[target.key])
			.map((target) => target.key);
	if (line.kind !== "missing") return [];
	return targets(fields, blockedNames(fields, host))
		.filter(
			(target) =>
				target.field.required &&
				target.field.kind !== "unsupported" &&
				isEmpty(target.field, valueAt(state.rail.values, target.key)),
		)
		.map((target) => target.key);
}

/** The field a "was already sent … left out" note belongs to: where the left-out files wait ("Add it"). */
export function leftOutFieldOf(
	state: FormSessionState,
	files: readonly LeftOutFile[],
): string | null {
	const ids = new Set(files.map((file) => file.slot.id));
	const found = Object.entries(state.rail.leftOut).find(([, held]) =>
		held.some((file) => ids.has(file.slot.id)),
	);
	return found ? found[0] : null;
}
