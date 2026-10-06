/*
 * What every pane component needs besides its own run: the actions, the layout and the few
 * constants derived from it. Built once per layout in `Stage` and handed down as one stable object.
 */
import type {
	FormSessionActions,
	RouteNav,
	WorkbenchLayout,
} from "../contracts";
import type { ResultFormat } from "../run/result-view";

/** Side padding of the bar, the links and the body: 16 px narrow, 24 px beside another pane, 32 px alone. */
export type PanePad = "px-4" | "px-6" | "px-8";

export interface PaneEnv {
	readonly actions: FormSessionActions;
	readonly layout: WorkbenchLayout;
	readonly routes: RouteNav;
	/** Unique per mounted stage: the prefix of tab and pane ids. */
	readonly uid: string;
	readonly mac: boolean;
	/** `ViewerHabits.decimalSign`, for sizes ("1.2 MB" / "1,2 MB"). */
	readonly decimalSign: "." | ",";
	readonly pad: PanePad;
	/** `(pointer: coarse)`: 44 px controls. */
	readonly touch: boolean;
	readonly resultFormat: ResultFormat;
}

export function padOf(layout: WorkbenchLayout, comparing: boolean): PanePad {
	if (!layout.split) return "px-4";
	return comparing ? "px-6" : "px-8";
}

export const tabIdOf = (uid: string, runId: string) => `${uid}-tab-${runId}`;
export const paneIdOf = (uid: string, runId: string) => `${uid}-pane-${runId}`;
