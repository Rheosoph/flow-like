/*
 * Compare (spec fix-report "Compare", canvas `cmp`): a pinned run beside the run on the stage, with
 * a table of the inputs that differ (per-run fields kept). From a 600 px stage (`layout.compare`) the
 * two panes sit side by side; a split box that is too narrow names the pinned run in a one-line note;
 * a phone shows nothing. Pure.
 */
import type {
	FieldChange,
	FormSessionState,
	RunEntry,
	ShortWords,
	WorkbenchLayout,
} from "../contracts";
import { diffValues } from "../model/values";
import type { IsSecret } from "../model/values";
import { withNumberText } from "../run/format";

export type CompareMode = "off" | "panes" | "note";

/** "Run 12 is pinned", "Same inputs", "3 inputs differ", or (a form without inputs) "Run 12 beside run 14". */
export type CompareTitle =
	| { readonly kind: "pinned" }
	| { readonly kind: "same" }
	| { readonly kind: "differ"; readonly count: number }
	| { readonly kind: "beside" };

export interface CompareView {
	readonly mode: CompareMode;
	readonly pinned: RunEntry | null;
	readonly selected: RunEntry | null;
	/** The pinned run is the one on the stage: there is nothing to put beside it yet. */
	readonly samePinned: boolean;
	readonly title: CompareTitle;
	readonly rows: readonly FieldChange[];
}

export interface CompareContext {
	readonly state: Pick<FormSessionState, "form" | "runs" | "view">;
	readonly layout: Pick<WorkbenchLayout, "split" | "compare">;
	readonly stageRun: RunEntry | null;
	readonly words: ShortWords;
	readonly isSecret: IsSecret;
	/** A typed number in the viewer's reading. */
	readonly number: (text: string) => string;
}

const OFF: CompareView = {
	mode: "off",
	pinned: null,
	selected: null,
	samePinned: false,
	title: { kind: "pinned" },
	rows: [],
};

function modeOf(layout: CompareContext["layout"]): CompareMode {
	if (layout.compare) return "panes";
	return layout.split ? "note" : "off";
}

function titleOf(
	mode: CompareMode,
	fieldCount: number,
	samePinned: boolean,
	rows: readonly FieldChange[],
): CompareTitle {
	if (samePinned || mode !== "panes") return { kind: "pinned" };
	if (fieldCount === 0) return { kind: "beside" };
	return rows.length === 0
		? { kind: "same" }
		: { kind: "differ", count: rows.length };
}

export function compareViewOf(context: CompareContext): CompareView {
	const { state, stageRun } = context;
	const pinnedId = state.view.pinnedRunId;
	const pinned =
		pinnedId === null
			? null
			: (state.runs.find((run) => run.id === pinnedId) ?? null);
	const mode = modeOf(context.layout);
	if (!pinned || !stageRun || mode === "off") return OFF;
	const samePinned = pinned.id === stageRun.id;
	const rows =
		mode === "panes" && !samePinned
			? withNumberText(
					state.form.fields,
					diffValues(
						state.form.fields,
						pinned.copy.values,
						stageRun.copy.values,
						{ words: context.words, isSecret: context.isSecret },
					),
					context.number,
				)
			: [];
	return {
		mode,
		pinned,
		selected: stageRun,
		samePinned,
		title: titleOf(mode, state.form.fields.length, samePinned, rows),
		rows,
	};
}
