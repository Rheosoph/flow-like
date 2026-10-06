/*
 * Which buttons a run's bar and its "…" menu carry (spec M1, M5, S4; canvas `paneVM` `act`). Pure.
 *
 * Stop acts on a live or sending run, "Remove from queue" on a queued one. An ended run offers one
 * repeat button when it can be repeated exactly (S4), else "Use these inputs" takes its place. A finished
 * run repeats only while the rail differs (otherwise the dock's Run does the same). The menu always lists
 * "Use these inputs" and "Save these inputs as a preset…" for a form with fields, and "Remove from this
 * device".
 */
import type { RunEntry, RunStatus, WorkbenchField } from "../contracts";
import { type RepeatBlocker, repeatBlocker } from "../run/run-view";

export type CopyKind = "answer" | "result";
export type RepeatKind = "runAgain" | "tryAgain";

export type MenuItemId =
	| "pin"
	| "unpin"
	| "repeat"
	| "use"
	| "savePreset"
	| "copy"
	| "remove";

export interface MenuItem {
	readonly id: MenuItemId;
	readonly separatorBefore: boolean;
	/** `repeat` only: a run whose files can no longer be sent says why. */
	readonly disabled: boolean;
	readonly blocker: RepeatBlocker | null;
	/** `repeat` only: "Try again" for a failed run, "Run again" otherwise. */
	readonly repeat: RepeatKind | null;
	/** `copy` only. */
	readonly copy: CopyKind | null;
}

export interface BarActions {
	readonly stop: boolean;
	readonly removeFromQueue: boolean;
	readonly repeat: RepeatKind | null;
	readonly use: boolean;
	/** The Copy button; null on a compact bar, where Copy moves into the menu. */
	readonly copy: CopyKind | null;
	/** Empty: no "…" button (live, sending and queued runs). */
	readonly menu: readonly MenuItem[];
}

export interface BarActionInput {
	readonly run: RunEntry;
	readonly now: number;
	readonly hasFields: boolean;
	/** Today's fields: a copy they would refuse is not repeated (S4). */
	readonly fields?: readonly WorkbenchField[];
	/** Phone, or two panes: Copy moves into the menu. */
	readonly compact: boolean;
	/** Two panes: the bar leaves the repeat button to the menu. */
	readonly comparing: boolean;
	readonly canPin: boolean;
	readonly pinned: boolean;
	/** The rail differs from this run's copy (per-run fields left out) or holds a file to pick again. */
	readonly railDiffers: boolean;
	readonly copy: CopyKind | null;
}

const NO_ACTIONS: BarActions = {
	stop: false,
	removeFromQueue: false,
	repeat: null,
	use: false,
	copy: null,
	menu: [],
};

const WORKING: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"starting",
	"asking",
	"running",
	"streaming",
	"sending",
]);

type Wanted = (run: RunEntry, railDiffers: boolean) => RepeatKind | null;

const fileNotSent = (run: RunEntry) =>
	run.outcome?.kind === "notStarted" && run.outcome.reason === "fileNotSent";

const RUN_AGAIN: Wanted = () => "runAgain";
const WHEN_RAIL_DIFFERS: Wanted = (_run, railDiffers) =>
	railDiffers ? "runAgain" : null;

/** The repeat a status wants before S4's check; none means the dock's Run covers it. */
const WANTED: Readonly<Partial<Record<RunStatus, Wanted>>> = {
	failed: () => "tryAgain",
	notStarted: (run) => (fileNotSent(run) ? "tryAgain" : "runAgain"),
	stopped: RUN_AGAIN,
	unknown: RUN_AGAIN,
	done: WHEN_RAIL_DIFFERS,
	empty: WHEN_RAIL_DIFFERS,
};

const item = (
	id: MenuItemId,
	extra: Partial<Omit<MenuItem, "id">> = {},
): MenuItem => ({
	id,
	separatorBefore: false,
	disabled: false,
	blocker: null,
	repeat: null,
	copy: null,
	...extra,
});

const pinItems = (input: BarActionInput) =>
	input.canPin ? [item(input.pinned ? "unpin" : "pin")] : [];

/** Shown disabled with its reason when S4 blocks it, and in place of the bar's button on two panes. */
function repeatItems(
	input: BarActionInput,
	blocker: RepeatBlocker | null,
	barRepeats: boolean,
) {
	const listed = input.hasFields && !barRepeats;
	if (!listed || !(input.comparing || blocker)) return [];
	return [
		item("repeat", {
			disabled: blocker !== null,
			blocker,
			repeat: input.run.status === "failed" ? "tryAgain" : "runAgain",
		}),
	];
}

const inputItems = (input: BarActionInput) =>
	input.hasFields ? [item("use"), item("savePreset")] : [];

const copyItems = (input: BarActionInput) =>
	input.compact && input.copy ? [item("copy", { copy: input.copy })] : [];

function menuOf(
	input: BarActionInput,
	blocker: RepeatBlocker | null,
	barRepeats: boolean,
) {
	const items = [
		...pinItems(input),
		...repeatItems(input, blocker, barRepeats),
		...inputItems(input),
		...copyItems(input),
	];
	return [...items, item("remove", { separatorBefore: items.length > 0 })];
}

/** "Use these inputs" in the bar: the rail differs, or S4 took the repeat button away. */
const barUses = (
	input: BarActionInput,
	wanted: RepeatKind | null,
	blocker: RepeatBlocker | null,
) =>
	input.hasFields &&
	(input.railDiffers || (wanted !== null && blocker !== null));

export function barActionsOf(input: BarActionInput): BarActions {
	const { run } = input;
	if (WORKING.has(run.status)) return { ...NO_ACTIONS, stop: true };
	if (run.status === "queued") return { ...NO_ACTIONS, removeFromQueue: true };
	const blocker = repeatBlocker(run, input.now, input.fields);
	const wanted = input.hasFields
		? (WANTED[run.status]?.(run, input.railDiffers) ?? null)
		: null;
	const repeat = input.comparing || blocker ? null : wanted;
	return {
		...NO_ACTIONS,
		repeat,
		use: barUses(input, wanted, blocker),
		copy: input.compact ? null : input.copy,
		menu: menuOf(input, blocker, repeat !== null),
	};
}
