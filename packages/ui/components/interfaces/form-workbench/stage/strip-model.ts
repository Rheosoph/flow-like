/*
 * The run strip's pure side: how many tabs fit, which runs hide behind "N more", what a tab says
 * after "Run 14", and which tabs carry the pin button. Spec M5 anatomy, canvas `renderVals`.
 */
import { LAYOUT, type RunEntry, type RunStatus } from "../contracts";
import { elapsedMs } from "../run/run-view";

/** What one tab takes in the fit: a 960 px stage shows six, as the canvas does (`LAYOUT.stripTabMinWidth` is a floor, not this). */
export const TAB_FIT_WIDTH = 140;
export const MIN_TABS = 2;
export const MAX_TABS = 6;
/** The coral Run at the left end of a zero-field strip. */
export const STRIP_RUN_RESERVE = 150;

/** Statuses whose clock ticks: started, and the flow is working or waiting for the person's answer. */
const TICKING: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"asking",
	"running",
	"streaming",
]);
const TOOK: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"done",
	"empty",
	"failed",
	"stopped",
]);
/** Waiting, sending and not-started tabs never get the pin button (nothing to compare yet). */
const NO_PIN: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"queued",
	"sending",
	"notStarted",
]);

export interface TabFitInput {
	readonly stageWidth: number;
	/** The zero-field strip carries the Run button. */
	readonly withRunButton: boolean;
	/** An unseen failure hides behind the overflow button: it needs 180 px, not 110. */
	readonly withFailure: boolean;
}

/** How many tabs the strip shows before the rest sit behind "N more" (never fewer than two, never more than six). */
export function tabFit(input: TabFitInput) {
	const reserve = input.withFailure
		? LAYOUT.stripOverflowReserveWithFailure
		: LAYOUT.stripOverflowReserve;
	const room =
		input.stageWidth - reserve - (input.withRunButton ? STRIP_RUN_RESERVE : 0);
	const fit = Math.floor(room / TAB_FIT_WIDTH);
	return Math.max(MIN_TABS, Math.min(MAX_TABS, fit));
}

/** A failed run of this session the person has not picked yet. */
export const isUnseenFailure = (
	run: Pick<RunEntry, "status" | "unseenFailure">,
) => run.status === "failed" && run.unseenFailure;

export interface StripTabs {
	/** Newest first, as `state.runs`. The selected and the pinned run are always among them. */
	readonly visible: readonly RunEntry[];
	readonly overflow: readonly RunEntry[];
	/** Unseen failures inside `overflow`, in list order. */
	readonly hiddenFailures: readonly RunEntry[];
}

export interface StripInput {
	readonly runs: readonly RunEntry[];
	readonly selectedId: string | null;
	readonly pinnedId: string | null;
	readonly stageWidth: number;
	readonly withRunButton: boolean;
}

function lastIndexWhere<T>(list: readonly T[], test: (item: T) => boolean) {
	for (let index = list.length - 1; index >= 0; index -= 1) {
		const item = list[index];
		if (item !== undefined && test(item)) return index;
	}
	return -1;
}

/** The phone's chip row shows this many runs (the Runs list holds the whole history). */
export const MAX_CHIPS = 30;

/** The first `fit` runs; the selected and the pinned run take the place of the last tabs that are neither. */
export function takeVisible(
	runs: readonly RunEntry[],
	selectedId: string | null,
	pinnedId: string | null,
	fit: number,
) {
	const visible = runs.slice(0, fit);
	const kept = (run: RunEntry) => run.id === selectedId || run.id === pinnedId;
	for (const id of [selectedId, pinnedId]) {
		if (id === null || visible.some((run) => run.id === id)) continue;
		const run = runs.find((candidate) => candidate.id === id);
		if (!run) continue;
		const slot = lastIndexWhere(visible, (item) => !kept(item));
		if (slot >= 0) visible[slot] = run;
		else visible.push(run);
	}
	return visible.sort((a, b) => runs.indexOf(a) - runs.indexOf(b));
}

function splitStrip(input: StripInput, withFailure: boolean): StripTabs {
	const fit = tabFit({
		stageWidth: input.stageWidth,
		withRunButton: input.withRunButton,
		withFailure,
	});
	const visible = takeVisible(
		input.runs,
		input.selectedId,
		input.pinnedId,
		fit,
	);
	const shown = new Set(visible.map((run) => run.id));
	const overflow = input.runs.filter((run) => !shown.has(run.id));
	return {
		visible,
		overflow,
		hiddenFailures: overflow.filter(isUnseenFailure),
	};
}

/**
 * Which runs get a tab. The strip is first laid out with the wider reserve (180 px) that an unseen failure
 * behind the overflow button needs; when no failure hides in that layout, the plain one (110 px) is used, so a
 * failure on the sixth tab still surfaces on the button instead of sitting unmarked in the row's last place.
 */
export function stripTabsOf(input: StripInput): StripTabs {
	const wide = splitStrip(input, true);
	return wide.hiddenFailures.length > 0 ? wide : splitStrip(input, false);
}

/** The overflow menu's rows: the unseen failures first, then the rest (the menu draws a hairline between). */
export function overflowMenuOf(tabs: StripTabs) {
	const failed = new Set(tabs.hiddenFailures.map((run) => run.id));
	return {
		failures: tabs.hiddenFailures,
		others: tabs.overflow.filter((run) => !failed.has(run.id)),
	};
}

/** What follows "Run 14" in a tab: the live clock, how long a finished run took, or the status word. */
export type TabText =
	| { readonly kind: "clock"; readonly ms: number }
	| { readonly kind: "took"; readonly ms: number }
	| { readonly kind: "word" };

export const ticksClock = (run: Pick<RunEntry, "status" | "startedAt">) =>
	TICKING.has(run.status) && run.startedAt !== null;

export function tabTextOf(run: RunEntry, now: number): TabText {
	if (ticksClock(run)) return { kind: "clock", ms: elapsedMs(run, now) };
	if (TOOK.has(run.status) && run.startedAt !== null)
		return { kind: "took", ms: elapsedMs(run, now) };
	return { kind: "word" };
}

export interface PinInput {
	readonly canPin: boolean;
	readonly selectedId: string | null;
	readonly pinnedId: string | null;
}

/** The pin button sits on the selected tab and the pinned one, except on queued, sending and not-started runs. */
export function showsPin(run: RunEntry, input: PinInput) {
	if (!input.canPin || NO_PIN.has(run.status)) return false;
	return run.id === input.selectedId || run.id === input.pinnedId;
}

/** The tab ←/→/Home/End moves to from `from` (an index into `tabs`), wrapping neither way. */
export function rovingTarget(key: string, from: number, count: number) {
	if (count === 0) return null;
	const last = count - 1;
	const moves: Readonly<Record<string, number>> = {
		ArrowLeft: Math.max(0, from - 1),
		ArrowRight: Math.min(last, from + 1),
		Home: 0,
		End: last,
	};
	return moves[key] ?? null;
}
