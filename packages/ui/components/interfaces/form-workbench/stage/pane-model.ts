/*
 * Everything one run's pane shows, worked out once: the bar, its buttons, the change chips, the
 * sections and their links, the notes, and the Inputs list. Spec M1/M5/S4 and the canvas `paneVM`.
 * The React side only draws it. Pure.
 */
import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import type {
	FieldChange,
	FormSessionState,
	RunEntry,
	RunStatus,
	ShortWords,
	WorkbenchLayout,
} from "../contracts";
import { perRunNamesOf } from "../model/markers";
import { linePlaces } from "../model/queue";
import {
	type IsSecret,
	diffValues,
	hasReminder,
	sameValues,
	valueKey,
} from "../model/values";
import { withNumberText } from "../run/format";
import { pendingInteractionsOf } from "../run/run-view";
import { type BarActions, type CopyKind, barActionsOf } from "./bar-actions";
import { type BarView, barViewOf } from "./bar-model";
import { type NoteView, hasContent, notesOf } from "./notes-model";
import { stepsOf } from "./run-steps";
import { type ValueText, valueTextOf } from "./stage-text";

export type SectionKey = "steps" | "answer" | "result" | "files" | "inputs";

export interface PaneContext {
	readonly state: FormSessionState;
	readonly layout: WorkbenchLayout;
	readonly now: number;
	readonly words: ShortWords;
	readonly isSecret: IsSecret;
	/** A typed number in the viewer's reading, for the Inputs list and the change chips. */
	readonly number: (text: string) => string;
	/** Two panes side by side. */
	readonly comparing: boolean;
	readonly canPin: boolean;
}

export interface ChipsView {
	/** The number of the run before this one in the list, when something changed against it. */
	readonly sinceRun: number | null;
	readonly changes: readonly FieldChange[];
	/** The rail was edited since this run (the run is the one the rail is compared with). */
	readonly edited: boolean;
}

export interface InputRowView {
	readonly key: string;
	readonly label: string;
	readonly text: ValueText;
	/** The 2 px bar: differs from the rail (this is the compared run; per-run fields never). */
	readonly differs: boolean;
}

export interface InputsView {
	readonly rows: readonly InputRowView[];
	readonly presetName: string | null;
	/** "Same as the form": the rail matches and nothing is per run. */
	readonly sameAsForm: boolean;
	/** "Use these inputs" in the section head: the run ended and the rail differs. */
	readonly canUse: boolean;
}

export interface PaneView {
	readonly run: RunEntry;
	readonly bar: BarView;
	readonly actions: BarActions;
	readonly copy: CopyKind | null;
	readonly chips: ChipsView | null;
	/** Link order: Steps, Answer, Result, Files, Inputs; only those the run has. */
	readonly sections: readonly SectionKey[];
	readonly showLinks: boolean;
	readonly notes: readonly NoteView[];
	/** Questions waiting for the person ("Waiting for you"). */
	readonly waiting: readonly IInteractionRequest[];
	/** A form without inputs and a run with nothing to read: one centred card. */
	readonly centered: boolean;
	readonly inputs: InputsView | null;
	/** A finished run offers the configured routes ("Go to Support chat"). */
	readonly offersRoutes: boolean;
}

/** No second line for a run that has not started. */
const NO_CHIPS: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"queued",
	"sending",
	"notStarted",
]);
const CALM_NOTES: ReadonlySet<NoteView["kind"]> = new Set<NoteView["kind"]>([
	"pending",
	"nothing",
	"stopped",
]);
const CENTERED: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"starting",
	"running",
	"streaming",
	"done",
	"empty",
	"stopped",
]);
const ENDED: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"done",
	"empty",
	"failed",
	"stopped",
	"notStarted",
	"unknown",
]);

/** The rail differs from what this run was given, per-run fields left out; a file to pick again counts. */
export function railDiffersFrom(
	state: Pick<FormSessionState, "form" | "rail" | "memory">,
	run: Pick<RunEntry, "copy">,
) {
	const skip = perRunNamesOf(state);
	const fields = state.form.fields.filter(
		(field) => !skip.includes(field.name),
	);
	const rail = state.rail.values;
	return (
		!sameValues(fields, rail, run.copy.values) ||
		fields.some((field) => hasReminder(rail[field.name]))
	);
}

export const copyKindOf = (run: Pick<RunEntry, "output">): CopyKind | null => {
	if (!run.output) return null;
	if (run.output.answer.trim() !== "") return "answer";
	return run.output.result ? "result" : null;
};

function sectionsOf(run: RunEntry, hasFields: boolean) {
	const { output } = run;
	const keys: (SectionKey | null)[] = [
		stepsOf(run).length > 0 ? "steps" : null,
		output && output.answer.trim() !== "" ? "answer" : null,
		output?.result ? "result" : null,
		output && output.attachments.length > 0 ? "files" : null,
		hasFields ? "inputs" : null,
	];
	return keys.filter((key): key is SectionKey => key !== null);
}

function chipsOf(
	run: RunEntry,
	context: PaneContext,
	railDiffers: boolean,
): ChipsView | null {
	const { state } = context;
	if (
		state.form.fields.length === 0 ||
		context.comparing ||
		NO_CHIPS.has(run.status)
	)
		return null;
	const index = state.runs.indexOf(run);
	const before = index >= 0 ? state.runs[index + 1] : undefined;
	const changes = before
		? withNumberText(
				state.form.fields,
				diffValues(state.form.fields, before.copy.values, run.copy.values, {
					words: context.words,
					isSecret: context.isSecret,
				}),
				context.number,
			)
		: [];
	const edited = state.rail.comparedRunId === run.id && railDiffers;
	if (changes.length === 0 && !edited) return null;
	return {
		sinceRun: changes.length > 0 && before ? before.n : null,
		changes,
		edited,
	};
}

function inputsOf(
	run: RunEntry,
	context: PaneContext,
	railDiffers: boolean,
): InputsView | null {
	const { state } = context;
	if (state.form.fields.length === 0) return null;
	const perRun = perRunNamesOf(state);
	const compared = state.rail.comparedRunId === run.id;
	const rows = state.form.fields.map((field): InputRowView => {
		const value = run.copy.values[field.name];
		const rail = state.rail.values[field.name];
		const differs =
			compared &&
			!perRun.includes(field.name) &&
			(valueKey(field, rail) !== valueKey(field, value) || hasReminder(rail));
		return {
			key: field.key,
			label: field.label,
			text: valueTextOf(field, value, context),
			differs,
		};
	});
	return {
		rows,
		presetName: run.copy.presetName,
		sameAsForm: !railDiffers && perRun.length === 0,
		canUse: railDiffers && ENDED.has(run.status),
	};
}

export function paneViewOf(run: RunEntry, context: PaneContext): PaneView {
	const { state, now } = context;
	const hasFields = state.form.fields.length > 0;
	const railDiffers = hasFields && railDiffersFrom(state, run);
	const copy = copyKindOf(run);
	const notes = notesOf(run, state, now);
	const calm = notes.every((note) => CALM_NOTES.has(note.kind));
	const sections = sectionsOf(run, hasFields);
	return {
		run,
		bar: barViewOf(run, { now, places: linePlaces(state.runs) }),
		actions: barActionsOf({
			run,
			now,
			hasFields,
			fields: state.form.fields,
			compact: context.comparing || !context.layout.split,
			comparing: context.comparing,
			canPin: context.canPin,
			pinned: state.view.pinnedRunId === run.id,
			railDiffers,
			copy,
		}),
		copy,
		chips: chipsOf(run, context, railDiffers),
		sections,
		showLinks: sections.length >= 2,
		notes,
		waiting: pendingInteractionsOf(run.output, now),
		centered:
			!hasFields && !hasContent(run) && calm && CENTERED.has(run.status),
		inputs: inputsOf(run, context, railDiffers),
		offersRoutes: run.status === "done" || run.status === "empty",
	};
}
