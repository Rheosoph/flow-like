/*
 * What the view shows (spec 2.0, M5, M6): the run on the stage and whether the stage follows the newest
 * started run, the compared run, failures seen, panes, overlays, the open list under a field, focus
 * requests the shell carries out, and the dock message's life.
 */
import {
	FORM_LIMITS,
	type FormSessionState,
	type OpenList,
	type RunEntry,
	type SelectHow,
	type WorkbenchLayout,
} from "../contracts";
import { recallFor, recentValues } from "../model/recent";
import { listOf, textOf, valueAt } from "../model/values";
import { stageRunOf } from "../run/run-view";
import { focusKeyOf } from "./reduce-per-run";
import {
	type CommandHandlers,
	type InputHandlers,
	emit,
	fieldFocus,
	mapRun,
	withFocus,
	withMessage,
	withRail,
	withView,
} from "./reduce-tx";
import { isBusy, isSplit, newestStartedRun, runById, targetOf } from "./state";

/** The stage follows new runs while it shows the newest started run (or nothing picked). */
const followsNewest = (state: FormSessionState, runId: string | null) =>
	runId === null || runId === newestStartedRun(state.runs)?.id;

/** The rail is compared with the run on the stage when it is of this session or the person picked it. */
function comparedOnStage(state: FormSessionState, pickedRun: boolean) {
	const stage = stageRunOf(state);
	if (!stage) return null;
	return stage.origin === "session" || pickedRun ? stage.id : null;
}

const seen = (run: RunEntry) =>
	run.unseenFailure ? { ...run, unseenFailure: false } : run;

/** A Runs row picked on a phone shows the Output pane. */
const showOutputFor = (state: FormSessionState, how: SelectHow) =>
	how === "list" && !isSplit(state)
		? withView(state, { pane: "output", outputUnseen: false })
		: state;

/**
 * The person picks a run (tab, Runs row, overflow menu, "Show", arrow keys): it goes on the stage, the
 * rail is compared with it, its failure counts as seen. Picking an older run keeps the stage there;
 * picking the newest started run (or none) lets the stage follow new runs again.
 */
const selectRun: CommandHandlers<"selectRun">["selectRun"] = (
	state,
	command,
) => {
	const run = runById(state, command.runId);
	if (command.runId !== null && !run) return state;
	const picked = command.how !== "auto" && run !== null;
	const selected = withView(state, {
		selectedRunId: command.runId,
		stageFollowsNewest: followsNewest(state, command.runId),
	});
	const compared = withRail(selected, {
		comparedRunId: comparedOnStage(selected, picked),
	});
	const marked = picked && run ? mapRun(compared, run.id, seen) : compared;
	return showOutputFor(marked, command.how);
};

const pinRun: CommandHandlers<"pinRun">["pinRun"] = (state, command) => {
	if (command.runId !== null && !runById(state, command.runId)) return state;
	return withView(state, { pinnedRunId: command.runId });
};

const setPane: CommandHandlers<"setPane">["setPane"] = (state, command) =>
	withView(state, {
		pane: command.pane,
		outputUnseen: command.pane === "output" ? false : state.view.outputUnseen,
	});

const openOverlay: CommandHandlers<"openOverlay">["openOverlay"] = (
	state,
	command,
) => withView(state, { overlay: command.overlay });

/** Closing the "Per run" popover returns the cursor to the marker's field, else to "Change". */
const closeOverlay: CommandHandlers<"closeOverlay">["closeOverlay"] = (
	state,
	_command,
	tx,
) => {
	const overlay = state.view.overlay;
	if (!overlay) return state;
	const closed = withView(state, { overlay: null });
	if (overlay.id !== "afterRun") return closed;
	const key = overlay.focusName ? focusKeyOf(closed, overlay.focusName) : null;
	return withFocus(closed, tx, key ? fieldFocus(key) : { kind: "change" });
};

/** Recent values open with the first row active (spec M6); next files with none. */
const openList: CommandHandlers<"openList">["openList"] = (state, command) =>
	withView(state, {
		list: {
			kind: command.kind,
			key: command.key,
			active: command.kind === "recent" ? 0 : -1,
		},
	});

/** Rows of the open list: next files of the field, or the recent values the field would offer now. */
function listRows(state: FormSessionState, list: OpenList): number {
	if (list.kind === "nextFiles")
		return state.rail.nextFiles[list.key]?.length ?? 0;
	const target = targetOf(state, list.key);
	const recall = target ? recallFor(state, target.field, true) : null;
	if (!target || !recall) return 0;
	const value = valueAt(state.rail.values, list.key);
	const chips = target.field.kind === "chips";
	return recentValues(recall.source, target.field, {
		exclude: chips ? listOf(value) : [],
		query: chips ? "" : textOf(value),
		forgotten: recall.forgotten,
		max: FORM_LIMITS.recentShown,
	}).length;
}

/** After its rows changed, the open list keeps its active row inside them; with none left it closes. */
export function withListInRange(state: FormSessionState) {
	const list = state.view.list;
	if (!list) return state;
	const rows = listRows(state, list);
	if (rows === 0) return withView(state, { list: null });
	const active = Math.min(list.active, rows - 1);
	return active === list.active
		? state
		: withView(state, { list: { ...list, active } });
}

/** ↑ / ↓ in the open list, kept inside its rows. */
const moveList: CommandHandlers<"moveList">["moveList"] = (state, command) => {
	const list = state.view.list;
	if (!list) return state;
	const rows = listRows(state, list);
	const active =
		rows === 0
			? -1
			: Math.min(rows - 1, Math.max(0, list.active + command.delta));
	return active === list.active
		? state
		: withView(state, { list: { ...list, active } });
};

const closeList: CommandHandlers<"closeList">["closeList"] = (state) =>
	state.view.list ? withView(state, { list: null }) : state;

const focusHandled: CommandHandlers<"focusHandled">["focusHandled"] = (
	state,
	command,
) =>
	state.view.focus?.seq === command.seq
		? withView(state, { focus: null })
		: state;

const markSeen: CommandHandlers<"markSeen">["markSeen"] = (state, command) =>
	mapRun(state, command.runId, seen);

const dismissMessage: CommandHandlers<"dismissMessage">["dismissMessage"] = (
	state,
) => (state.view.message ? withView(state, { message: null }) : state);

/** A key press in the rail ends a start message early (spec M1 step 6). */
const railKey: CommandHandlers<"railKey">["railKey"] = (state) =>
	state.view.message?.message.kind === "start"
		? withView(state, { message: null })
		: state;

const sameLayout = (a: WorkbenchLayout | null, b: WorkbenchLayout) =>
	a !== null &&
	a.width === b.width &&
	a.height === b.height &&
	a.split === b.split &&
	a.touch === b.touch &&
	a.finePointer === b.finePointer &&
	a.compare === b.compare;

const setLayout: CommandHandlers<"setLayout">["setLayout"] = (
	state,
	command,
) =>
	sameLayout(state.layout, command.layout)
		? state
		: { ...state, layout: command.layout };

/**
 * An answer to an in-run question goes to the runtime; the run leaves `asking` when its output says so. A message that
 * an earlier answer failed goes with the new try.
 */
const respondInteraction: CommandHandlers<"respondInteraction">["respondInteraction"] =
	(state, command, tx) => {
		emit(tx, {
			type: "respondInteraction",
			runId: command.runId,
			interactionId: command.interactionId,
			value: command.value,
		});
		return state.view.message?.message.kind === "answerFailed"
			? withView(state, { message: null })
			: state;
	};

export const VIEW_COMMANDS: CommandHandlers<
	| "selectRun"
	| "pinRun"
	| "setPane"
	| "openOverlay"
	| "closeOverlay"
	| "openList"
	| "moveList"
	| "closeList"
	| "focusHandled"
	| "markSeen"
	| "dismissMessage"
	| "railKey"
	| "setLayout"
	| "respondInteraction"
> = {
	selectRun,
	pinRun,
	setPane,
	openOverlay,
	closeOverlay,
	openList,
	moveList,
	closeList,
	focusHandled,
	markSeen,
	dismissMessage,
	railKey,
	setLayout,
	respondInteraction,
};

const messageExpired: InputHandlers<"messageExpired">["messageExpired"] = (
	state,
	input,
) =>
	state.view.message?.seq === input.seq
		? withView(state, { message: null })
		: state;

/** The answer did not reach a run that still goes: the dock says so until the next edit, run or answer. */
const interactionFailed: InputHandlers<"interactionFailed">["interactionFailed"] =
	(state, input, tx) => {
		const run = runById(state, input.runId);
		if (!run || !isBusy(run)) return state;
		return withMessage(
			state,
			tx,
			{ kind: "answerFailed", n: run.n, runId: run.id },
			"action",
		);
	};

export const VIEW_INPUTS: InputHandlers<
	"messageExpired" | "interactionFailed"
> = { messageExpired, interactionFailed };

/** "Your answer … could not be sent" goes once its run no longer goes: nothing is left to answer. */
export function withoutStaleAnswerFailure(
	state: FormSessionState,
): FormSessionState {
	const message = state.view.message?.message;
	if (message?.kind !== "answerFailed") return state;
	const run = runById(state, message.runId);
	return run && isBusy(run) ? state : withView(state, { message: null });
}
