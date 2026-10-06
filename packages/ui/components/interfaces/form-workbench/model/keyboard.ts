/*
 * ↵ moves on (spec M2) and where the cursor lands after a run (M1 step 5). Only the decisions;
 * the reducer moves focus and the controls handle the keys.
 */
import type {
	CopyValue,
	FieldKey,
	FieldValues,
	WorkbenchField,
} from "../contracts";
import { type FieldTarget, targets } from "./fields";
import { hasReminder, isEmpty, sameValues, valueAt } from "./values";

type Values = Readonly<Record<string, CopyValue>>;

/** A stop ↵ waits at: a required empty field, or a "Pick again" reminder (S4). */
function needsEntry(target: FieldTarget, values: Values): boolean {
	const value = valueAt(values, target.key);
	if (hasReminder(value)) return true;
	return target.field.required && isEmpty(target.field, value);
}

const isStop = (target: FieldTarget) => target.field.kind !== "unsupported";

/**
 * `flpEnterTarget`: the next stop after `fromKey` that needs an entry, wrapping to the top;
 * `skip` names fields that are no stop (blocked file fields). null means "run now".
 */
export function enterTarget(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	fromKey: FieldKey,
	skip: readonly string[],
): FieldKey | null {
	const stops = targets(fields, skip).filter(isStop);
	const at = stops.findIndex((target) => target.key === fromKey);
	for (let step = 1; step <= stops.length; step += 1) {
		const target = stops[(at + step + stops.length) % stops.length];
		if (target.key !== fromKey && needsEntry(target, values)) return target.key;
	}
	return null;
}

/**
 * `flpEnterMayRun`: ↵ never starts a run whose inputs equal the newest run of this session
 * (`newest`: its copy). The Run button, "Run again" and ⌘↵ are not asked.
 */
export function enterMayRun(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	newest: Values | null,
): boolean {
	return !(newest && sameValues(fields, values, newest));
}

/** Where focus goes for a field: an object's first property. */
function focusKeyOf(field: WorkbenchField): FieldKey {
	return field.kind === "group"
		? (field.props[0]?.key ?? field.key)
		: field.key;
}

/**
 * `flpFocusAfterRun` (desktop): the first per-run field that is required and empty, else the
 * first empty required stop, else the first per-run field; null when nothing is per run.
 */
export function focusAfterRun(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	perRunNames: readonly string[],
	skip: readonly string[],
): FieldKey | null {
	if (perRunNames.length === 0) return null;
	const open = fields.filter((field) => !skip.includes(field.name));
	const perRun = open.filter((field) => perRunNames.includes(field.name));
	const missing = perRun.find(
		(field) => field.required && isEmpty(field, values[field.name]),
	);
	if (missing) return focusKeyOf(missing);
	const empty = targets(open).find(
		(target) =>
			target.field.required &&
			isEmpty(target.field, valueAt(values, target.key)),
	);
	if (empty) return empty.key;
	return perRun[0] ? focusKeyOf(perRun[0]) : null;
}

/** `enterkeyhint` on a phone: "next" while ↵ would move on, else "go". */
export function enterHint(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	key: FieldKey,
	skip: readonly string[],
): "next" | "go" {
	return enterTarget(fields, values, key, skip) === null ? "go" : "next";
}
