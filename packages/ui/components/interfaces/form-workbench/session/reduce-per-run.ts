/*
 * Per run (spec M1): the setting (a change applies at once, is saved, and acts at the next run), the
 * one-time offer, and the series a multi-file pick makes (`prefs.auto`), which asks once when it ends
 * whether its fields stay per run.
 */
import type { FieldKey, FormPrefs, FormSessionState } from "../contracts";
import { filesAndDates, offerNames } from "../model/per-run";
import { startingValue } from "../model/values";
import {
	type CommandHandlers,
	type Tx,
	clearEdits,
	fieldFocus,
	withFocus,
	withMessage,
	withPrefs,
	withRail,
	withView,
	withoutProblem,
	withoutText,
} from "./reduce-tx";
import {
	activePreset,
	fieldByName,
	isSplit,
	livePerRun,
	secretTest,
} from "./state";

const unique = (names: readonly string[]) => [...new Set(names)];

/** Prefs with `names` per run as the person's own setting (out of the series). */
function perRunWith(prefs: FormPrefs, names: readonly string[]) {
	return {
		perRun: unique([...prefs.perRun, ...names]),
		auto: prefs.auto.filter((name) => !names.includes(name)),
		introduced: true,
	};
}

/** A field changed in the popover leaves `prefs.auto`: it is now the person's own setting. */
const setPerRun: CommandHandlers<"setPerRun">["setPerRun"] = (
	state,
	command,
	tx,
) => {
	const prefs = state.memory.prefs;
	if (command.on)
		return withPrefs(state, tx, perRunWith(prefs, [command.name]));
	return withPrefs(state, tx, {
		perRun: prefs.perRun.filter((name) => name !== command.name),
		auto: prefs.auto.filter((name) => name !== command.name),
	});
};

const uncheckAllPerRun: CommandHandlers<"uncheckAllPerRun">["uncheckAllPerRun"] =
	(state, _command, tx) => {
		const { perRun, auto } = state.memory.prefs;
		if (perRun.length === 0 && auto.length === 0) return state;
		return withPrefs(state, tx, { perRun: [], auto: [] });
	};

/** "Make files and dates per run": every file field and every required date field. */
const perRunFilesAndDates: CommandHandlers<"perRunFilesAndDates">["perRunFilesAndDates"] =
	(state, _command, tx) =>
		withPrefs(
			state,
			tx,
			perRunWith(state.memory.prefs, filesAndDates(state.form.fields)),
		);

/** Where the cursor goes for a field: an object's first property. */
export function focusKeyOf(
	state: FormSessionState,
	name: string,
): FieldKey | null {
	const field = fieldByName(state, name);
	if (!field) return null;
	return field.kind === "group"
		? (field.props[0]?.key ?? field.key)
		: field.key;
}

/** "Yes" makes the fields per run, resets them now and moves the cursor to the first. */
function offerYes(
	state: FormSessionState,
	names: readonly string[],
	tx: Tx,
): FormSessionState {
	const preset = activePreset(state);
	const fields = state.form.fields.filter((field) =>
		names.includes(field.name),
	);
	let next = withPrefs(clearEdits(state), tx, {
		...perRunWith(state.memory.prefs, names),
		offerAnswered: true,
	});
	const values = { ...next.rail.values };
	for (const field of fields) {
		values[field.name] = startingValue(field, preset);
		next = withoutProblem(withoutText(next, field.key), field.key);
	}
	next = withMessage(withRail(next, { values }), tx, {
		kind: "perRunNow",
		labels: fields.map((field) => field.label),
	});
	const first = fields[0] ? focusKeyOf(next, fields[0].name) : null;
	if (!first) return next;
	return withFocus(next, tx, fieldFocus(first, false, !isSplit(next)));
}

/** "Yes" / "No" to the offer or the series-end question; either answer is final. */
const answerQuestion: CommandHandlers<"answerQuestion">["answerQuestion"] = (
	state,
	command,
	tx,
) => {
	const question = state.view.question;
	if (!question) return state;
	const asked = withView(state, { question: null });
	const prefs = asked.memory.prefs;
	if (question.kind === "seriesEnd")
		return withPrefs(asked, tx, {
			perRun: command.yes
				? unique([...prefs.perRun, ...prefs.auto])
				: prefs.perRun,
			auto: [],
		});
	if (command.yes) return offerYes(asked, question.names, tx);
	return withPrefs(asked, tx, { offerAnswered: true, introduced: true });
};

/**
 * The series ended (its last file was taken, or the rest was removed): the dock asks once whether
 * the fields it made per run stay so.
 */
export function endSeries(state: FormSessionState): FormSessionState {
	const auto = state.memory.prefs.auto;
	const names = livePerRun(state).filter((name) => auto.includes(name));
	if (names.length === 0 || state.view.question !== null) return state;
	return withView(state, { question: { kind: "seriesEnd", names } });
}

/**
 * The one-time offer as a run starts (spec M1): with three or more runs on this device, nothing per
 * run and the offer never answered, the fields `offerNames` finds.
 */
export function maybeOffer(state: FormSessionState, tx: Tx): FormSessionState {
	const prefs = state.memory.prefs;
	if (prefs.offerAnswered || state.view.question !== null) return state;
	if (livePerRun(state).length > 0) return state;
	const runs = state.runs
		.filter((run) => run.status !== "notStarted")
		.map((run) => ({ values: run.copy.values, replaced: run.copy.replaced }));
	const names = offerNames(state.form.fields, runs, [], secretTest(state));
	if (names.length === 0) return state;
	const asked = withView(state, { question: { kind: "offer", names } });
	return prefs.introduced ? asked : withPrefs(asked, tx, { introduced: true });
}

export const PER_RUN_COMMANDS: CommandHandlers<
	"setPerRun" | "uncheckAllPerRun" | "perRunFilesAndDates" | "answerQuestion"
> = {
	setPerRun,
	uncheckAllPerRun,
	perRunFilesAndDates,
	answerQuestion,
};
