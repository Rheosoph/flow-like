/*
 * The press (spec M1 behaviour, M2, M5): validate; the 700 ms rule; the run takes its copy; it sends
 * its files, joins the queue or starts; per-run fields go back and the next file moves in (unless
 * ⇧⌘↵); the cursor lands where the next entry begins; the start message names the pairing; the
 * series may end and the offer may come. ↵ walks to the next empty required stop and runs only when
 * none is left and the inputs differ from the newest run. "Run again" runs a run's own copy and never
 * touches the rail. A form without fields never queues: at its cap a press is refused.
 */
import type {
	FieldKey,
	FieldProblem,
	FormSessionState,
	RunCopy,
	RunEntry,
	ShortWords,
	StartMessage,
} from "../contracts";
import { formatDate } from "../model/date-text";
import { enterMayRun, enterTarget, focusAfterRun } from "../model/keyboard";
import { requestCheck } from "../model/next-files";
import {
	type PerRunApplied,
	applyPerRun,
	pairing,
	startMessage,
} from "../model/per-run";
import {
	firstProblemKey,
	problemOf,
	railProblems,
	railSlots,
} from "../model/validate";
import { baselineOf, slotsOf, valueAt } from "../model/values";
import { repeatable } from "../run/run-view";
import { endSeries, maybeOffer } from "./reduce-per-run";
import { commitTyped } from "./reduce-rail";
import {
	addRun,
	atCap,
	isDoublePress,
	refuseAtCap,
	showOnStage,
	startQueued,
	startRun,
} from "./reduce-runs";
import {
	type CommandHandlers,
	type Tx,
	clearEdits,
	fieldFocus,
	withFocus,
	withMessage,
	withProblem,
	withRail,
	withView,
} from "./reduce-tx";
import { updateSlot } from "./slots";
import {
	activePreset,
	blockedOf,
	isSplit,
	livePerRun,
	newestEnteredRun,
	runById,
	secretTest,
	targetOf,
} from "./state";

// ─── Checking the rail ──────────────────────────────────────────────────────

interface Checked {
	readonly state: FormSessionState;
	readonly problems: Readonly<Record<FieldKey, FieldProblem>>;
}

/** Commits every typed date and number, then validates the whole rail. */
function checkRail(state: FormSessionState, tx: Tx) {
	let next = state;
	const unread: Record<FieldKey, FieldProblem> = {};
	for (const key of Object.keys(state.rail.texts)) {
		const committed = commitTyped(next, key, tx);
		next = committed.state;
		if (committed.unread) unread[key] = { code: "date" };
	}
	const problems = {
		...railProblems(next.form.fields, next.rail, next.form),
		...unread,
	};
	const checked: Checked = { state: next, problems };
	return checked;
}

/** Run with problems: the messages show, the dock counts them, focus goes to the first one. */
function refusePress(checked: Checked, tx: Tx) {
	const { fields } = checked.state.form;
	const marked = withRail(checked.state, {
		problems: checked.problems,
		pressed: true,
	});
	const shown = isSplit(marked) ? marked : withView(marked, { pane: "inputs" });
	const first = firstProblemKey(fields, checked.problems);
	return first ? withFocus(shown, tx, fieldFocus(first)) : shown;
}

// ─── After the run took its copy ────────────────────────────────────────────

/** A new entry begins: per-run fields went back, messages and Undo go, the next-files list closes. */
function nextEntry(
	state: FormSessionState,
	applied: PerRunApplied | null,
	tx: Tx,
) {
	const cleared = clearEdits(state);
	const list =
		cleared.view.list?.kind === "nextFiles" ? null : cleared.view.list;
	return withView(
		withRail(cleared, {
			values: applied?.values ?? cleared.rail.values,
			nextFiles: applied?.nextFiles ?? cleared.rail.nextFiles,
			texts: {},
			problems: {},
			pressed: false,
			entryBegan: {},
			lastPressAt: tx.clock.now,
		}),
		{ list },
	);
}

/**
 * Desktop: the cursor goes to `focusAfterRun` (it stays when nothing is per run or ⇧⌘↵ left the
 * inputs). Phone: while next files wait the Inputs pane stays and the first per-run field scrolls
 * into view without focus; otherwise the Output pane shows.
 */
function placeCursor(
	state: FormSessionState,
	applied: PerRunApplied | null,
	perRun: readonly string[],
	tx: Tx,
) {
	const { fields } = state.form;
	const key = applied
		? focusAfterRun(fields, state.rail.values, perRun, blockedOf(state))
		: null;
	if (isSplit(state))
		return key ? withFocus(state, tx, fieldFocus(key, true)) : state;
	if (!applied?.nextFile)
		return withView(state, { pane: "output", outputUnseen: false });
	const kept = withView(state, { pane: "inputs" });
	return key ? withFocus(kept, tx, fieldFocus(key, false, true)) : kept;
}

/**
 * Words of the start message's stored pairing. Dates are the viewer's (Intl); the few words a per-run
 * switch, list or map would need are English here, so the dock re-pairs from the run's copy
 * (`StartMessage.runId`) with the viewer's words and shows these only when the run is gone.
 */
export const pairWords = (locale: string): ShortWords => ({
	none: "",
	empty: "",
	on: "On",
	off: "Off",
	files: (count) => (count === 1 ? "1 file" : `${count} files`),
	entries: (count) => (count === 1 ? "1 entry" : `${count} entries`),
	date: (iso) => formatDate(iso, locale),
});

const START_STATE: Partial<Record<RunEntry["status"], StartMessage["state"]>> =
	{ sending: "sending", queued: "queued" };

/** What the start message says of its run now; null once the run was taken out before it began. */
const startStateOf = (status: RunEntry["status"]) =>
	status === "notStarted" ? null : (START_STATE[status] ?? "started");

/**
 * The start message follows its run while it shows: "starts when its file is sent" becomes "is queued" or "started"
 * once the file is sent. A run taken out before it began, or one that started with nothing to name, has none.
 */
export function withLiveStart(state: FormSessionState): FormSessionState {
	const entry = state.view.message;
	if (!entry || entry.message.kind !== "start") return state;
	const { start } = entry.message;
	const run = runById(state, start.runId ?? null);
	if (!run) return state;
	const live = startStateOf(run.status);
	if (live === start.state) return state;
	const next = live === null ? null : startMessage({ ...start, state: live });
	return withView(state, {
		message: next
			? { ...entry, message: { kind: "start", start: next } }
			: null,
	});
}

/** "Run 15 started: invoice-RE-2026-0918.pdf, 18 Sep 2026." for 4 s or until the next key in the rail. */
function announceStart(
	state: FormSessionState,
	run: RunEntry,
	applied: PerRunApplied | null,
	tx: Tx,
) {
	const { form } = state;
	const start = startMessage({
		n: run.n,
		runId: run.id,
		state: startStateOf(run.status) ?? "started",
		files: run.pendingSlotIds.length,
		pairs: pairing(
			form.fields,
			run.copy.values,
			run.copy.perRun,
			secretTest(state),
			pairWords(form.viewer.locale),
		),
		lastFile: applied?.lastFile ?? false,
		leftAsIs: run.copy.leftAsIs,
	});
	return start ? withMessage(state, tx, { kind: "start", start }) : state;
}

/** Text fields whose entry began over an empty field or a fully selected value. */
const replacedNames = (state: FormSessionState) =>
	Object.entries(state.rail.entryBegan)
		.filter(([, how]) => how === "replaced")
		.map(([key]) => key);

/** The run takes its copy of the rail; everything after M1 step 3 follows. */
function runFromRail(state: FormSessionState, leaveAsIs: boolean, tx: Tx) {
	const { fields } = state.form;
	const perRun = livePerRun(state);
	const preset = activePreset(state);
	const copy: RunCopy = {
		values: state.rail.values,
		presetName: preset?.name ?? null,
		leftAsIs: leaveAsIs,
		perRun,
		replaced: replacedNames(state),
	};
	const added = addRun(state, copy, tx);
	const queued = startQueued(added.state, tx);
	const run = runById(queued, added.run.id) ?? added.run;
	const applied = leaveAsIs
		? null
		: applyPerRun(
				fields,
				queued.rail.values,
				queued.rail.nextFiles,
				perRun,
				baselineOf(fields, preset),
			);
	let next = nextEntry(queued, applied, tx);
	next = placeCursor(next, applied, perRun, tx);
	next = announceStart(next, run, applied, tx);
	if (applied?.lastFile) next = endSeries(next);
	return maybeOffer(next, tx);
}

/** A form without fields: never queued; at its cap the press is refused and announced. */
function runWithoutFields(state: FormSessionState, tx: Tx) {
	if (atCap(state)) return refuseAtCap(state, tx);
	if (isDoublePress(state, {}, tx.clock.now)) return state;
	const copy: RunCopy = {
		values: {},
		presetName: null,
		leftAsIs: false,
		perRun: [],
		replaced: [],
	};
	const added = addRun(clearEdits(state), copy, tx);
	const started = withRail(startRun(added.state, added.run.id, tx), {
		lastPressAt: tx.clock.now,
	});
	if (!isSplit(started))
		return withView(started, { pane: "output", outputUnseen: false });
	return withFocus(started, tx, { kind: "runAgain" });
}

/** Run, ⌘↵, the hero's and the strip's Run, the phone's Run (and ↵ when nothing is missing). */
export function press(state: FormSessionState, leaveAsIs: boolean, tx: Tx) {
	if (state.form.fields.length === 0) return runWithoutFields(state, tx);
	return pressWithFields(state, leaveAsIs, tx);
}

/** Validate, check what an inline host can carry, the 700 ms rule, then the run. */
function pressWithFields(state: FormSessionState, leaveAsIs: boolean, tx: Tx) {
	const checked = checkRail(state, tx);
	if (Object.keys(checked.problems).length > 0) return refusePress(checked, tx);
	const { fields, host } = checked.state.form;
	const values = checked.state.rail.values;
	const tooLarge = requestCheck(host, railSlots(fields, values));
	if (tooLarge)
		return withMessage(checked.state, tx, {
			kind: "requestTooLarge",
			...tooLarge,
		});
	if (isDoublePress(checked.state, values, tx.clock.now)) return checked.state;
	return runFromRail(checked.state, leaveAsIs, tx);
}

const run: CommandHandlers<"run">["run"] = (state, command, tx) =>
	press(state, command.leaveAsIs, tx);

// ─── ↵ ──────────────────────────────────────────────────────────────────────

/**
 * ↵ in a field (spec M2): a typed date commits first (a far-back day only commits); an empty
 * required field or an unreadable value shows its message and stays; else the cursor goes to the
 * next empty required stop; with none left the form runs, unless the inputs equal the newest run of
 * this session that ran or still waits to ("Same inputs as run {n}. ⌘↵ runs it again.").
 */
const enter: CommandHandlers<"enter">["enter"] = (state, command, tx) => {
	const target = targetOf(state, command.fromKey);
	if (!target) return state;
	const committed = commitTyped(state, command.fromKey, tx);
	if (committed.unread || committed.past > 0) return committed.state;
	const next = committed.state;
	const problem = problemOf(
		target.field,
		valueAt(next.rail.values, command.fromKey),
		next.rail.texts[command.fromKey],
		next.form,
	);
	const checked = withProblem(next, command.fromKey, problem);
	if (problem) return checked;
	const { fields } = checked.form;
	const values = checked.rail.values;
	const to = enterTarget(fields, values, command.fromKey, blockedOf(checked));
	if (to) return withFocus(checked, tx, fieldFocus(to, true));
	const newest = newestEnteredRun(checked);
	if (newest && !enterMayRun(fields, values, newest.copy.values))
		return withMessage(checked, tx, { kind: "sameInputs", n: newest.n });
	return press(checked, false, tx);
};

// ─── Run again / Try again ──────────────────────────────────────────────────

/** Files of the copy that were not sent are tried again (the run holds their File objects). */
function retryCopyFiles(state: FormSessionState, source: RunEntry) {
	const failed = state.form.fields
		.flatMap((field) => slotsOf(source.copy.values[field.name]))
		.filter((slot) => slot.state === "failed");
	return failed.reduce(
		(current, slot) =>
			updateSlot(current, slot.id, (item) => ({
				...item,
				state: "waiting",
				error: null,
				progress: null,
			})),
		state,
	);
}

/**
 * "Run again" / "Try again": a new run with the run's own copy, only when it can be repeated exactly
 * (S4) and today's form still takes it (as the run bar decides). The new run takes the stage and the
 * cursor goes to its Stop.
 */
const runAgain: CommandHandlers<"runAgain">["runAgain"] = (
	state,
	command,
	tx,
) => {
	const source = runById(state, command.runId);
	if (!source || !repeatable(source, tx.clock.now, state.form.fields))
		return state;
	const withoutFields = state.form.fields.length === 0;
	if (withoutFields && atCap(state)) return refuseAtCap(state, tx);
	if (isDoublePress(state, source.copy.values, tx.clock.now)) return state;
	const retried = retryCopyFiles(clearEdits(state), source);
	const values = runById(retried, source.id)?.copy.values ?? source.copy.values;
	const added = addRun(
		retried,
		{ ...source.copy, values, leftAsIs: false },
		tx,
	);
	const started = withoutFields
		? startRun(added.state, added.run.id, tx)
		: startQueued(added.state, tx);
	const shown = withView(showOnStage(started, added.run.id), {
		stageFollowsNewest: true,
		...(isSplit(started) ? {} : { pane: "output", outputUnseen: false }),
	});
	return withFocus(shown, tx, { kind: "stop" });
};

export const PRESS_COMMANDS: CommandHandlers<"run" | "enter" | "runAgain"> = {
	run,
	enter,
	runAgain,
};
