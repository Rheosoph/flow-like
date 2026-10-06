/*
 * Field problems on blur, on ↵ and on press (SURFACE §6, spec M2, F, S4). Typed dates are never
 * parsed here: a date value is ISO or empty, and S-STATE commits typed date text with `readDate`
 * and records the `date` problem itself. A file still sending is no problem at a press: the run
 * takes its slot and waits ("Sending").
 */
import type {
	CopyValue,
	FieldKey,
	FieldProblem,
	FileSlot,
	FormPhase,
	FormSessionState,
	HostCapabilities,
	RailState,
	ViewerHabits,
	WorkbenchField,
} from "../contracts";
import { targets } from "./fields";
import { sentValue } from "./payload";
import { isEmpty, isFileField, isReminder, slotsOf, valueAt } from "./values";

export interface ProblemContext {
	readonly host: Pick<
		HostCapabilities,
		"flowPathFiles" | "uploads" | "inlineFileLimitBytes"
	>;
	readonly viewer: Pick<ViewerHabits, "decimalSign">;
}

const REQUIRED: FieldProblem = { code: "required" };
const FILE_NOT_HERE: FieldProblem = { code: "fileNotHere" };

/** `flpBlocked` for one field: a FlowPath field this host cannot fill. */
export function isBlocked(
	field: WorkbenchField,
	host: Pick<HostCapabilities, "flowPathFiles">,
) {
	return (
		isFileField(field) && field.fileMode === "flowpath" && !host.flowPathFiles
	);
}

/** `flpBlocked`: names of the FlowPath fields this host cannot fill (no ↵ stop, left out when optional). */
export function blockedNames(
	fields: readonly WorkbenchField[],
	host: Pick<HostCapabilities, "flowPathFiles">,
) {
	return fields
		.filter((field) => isBlocked(field, host))
		.map((field) => field.name);
}

/** Labels of the required fields this host cannot fill: "{label} can't be sent from this page." (DockLineInput.blocked). */
export function blockedLabels(
	fields: readonly WorkbenchField[],
	host: Pick<HostCapabilities, "flowPathFiles">,
) {
	return fields
		.filter((field) => field.required && isBlocked(field, host))
		.map((field) => field.label);
}

const tooLarge = (slot: FileSlot, limit: number | null) =>
	limit !== null && slot.size !== null && slot.size > limit;

/** A file that cannot go: a reminder to pick again, a failed upload, too large for an inline host. */
const slotProblem = (
	slots: readonly FileSlot[],
	host: ProblemContext["host"],
): FieldProblem | null => {
	const reminder = slots.find(isReminder);
	if (reminder) return { code: "pickAgain", fileName: reminder.name };
	const failed = slots.find((slot) => slot.state === "failed");
	if (failed) return { code: "fileFailed", fileName: failed.name };
	const limit = host.uploads === "inline" ? host.inlineFileLimitBytes : null;
	const large = slots.find((slot) => tooLarge(slot, limit));
	return large && limit !== null
		? { code: "fileTooLarge", fileName: large.name, limitBytes: limit }
		: null;
};

const fileProblem = (
	field: WorkbenchField,
	value: CopyValue | undefined,
	host: ProblemContext["host"],
): FieldProblem | null =>
	slotProblem(slotsOf(value), host) ??
	(field.required && isEmpty(field, value) ? REQUIRED : null);

/** A date with text not yet committed: the reducer reads it with `readDate` and judges it. */
const typing = (field: WorkbenchField, text: string | undefined) =>
	field.kind === "date" && (text ?? "").trim() !== "";

function valueProblem(
	field: WorkbenchField,
	value: CopyValue | undefined,
	context: ProblemContext,
) {
	const result = sentValue(field, value, {
		decimalSign: context.viewer.decimalSign,
	});
	return result.kind === "problems"
		? (result.problems[field.key] ?? null)
		: null;
}

/**
 * The problem of one field or object property, or null. A date with uncommitted text is the
 * reducer's to judge; a blocked FlowPath field is a problem only when it is required.
 */
export const problemOf = (
	field: WorkbenchField,
	value: CopyValue | undefined,
	text: string | undefined,
	context: ProblemContext,
): FieldProblem | null => {
	if (field.kind === "group" || typing(field, text)) return null;
	if (isBlocked(field, context.host))
		return field.required ? FILE_NOT_HERE : null;
	return isFileField(field)
		? fileProblem(field, value, context.host)
		: valueProblem(field, value, context);
};

/** Every problem of the rail by FieldKey, in form order (objects property by property). */
export function railProblems(
	fields: readonly WorkbenchField[],
	rail: Pick<RailState, "values" | "texts">,
	context: ProblemContext,
) {
	const problems: Record<FieldKey, FieldProblem> = {};
	for (const target of targets(fields)) {
		const value = valueAt(rail.values, target.key);
		const found = problemOf(
			target.field,
			value,
			rail.texts[target.key],
			context,
		);
		if (found) problems[target.key] = found;
	}
	return problems;
}

/** The first problem in form order: where focus goes after an invalid press. */
export function firstProblemKey(
	fields: readonly WorkbenchField[],
	problems: Readonly<Record<FieldKey, FieldProblem>>,
) {
	return targets(fields).find((target) => problems[target.key])?.key ?? null;
}

/** "{n} fields to fill in": required stops still empty; `skip` names blocked fields. */
export function missingCount(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
	skip: readonly string[],
) {
	return targets(fields, skip).filter(
		(target) =>
			target.field.required &&
			target.field.kind !== "unsupported" &&
			isEmpty(target.field, valueAt(values, target.key)),
	).length;
}

const isUploading = (slot: FileSlot) =>
	slot.state === "waiting" || slot.state === "sending";

/** The current entry's files, in form order (next files and left-out files are not the entry). */
export function railSlots(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
) {
	return fields
		.filter(isFileField)
		.flatMap((field) => slotsOf(values[field.name]));
}

/** The rail as a whole: files of the current entry sending, else problems after a press, else idle. */
export function formPhase(
	state: Pick<FormSessionState, "form" | "rail">,
): FormPhase {
	if (railSlots(state.form.fields, state.rail.values).some(isUploading))
		return "uploading";
	const problems = Object.keys(state.rail.problems).length;
	return state.rail.pressed && problems > 0 ? "invalid" : "idle";
}
