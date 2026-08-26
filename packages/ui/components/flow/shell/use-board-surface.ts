"use client";

import { useEffect, useMemo, useReducer, useRef } from "react";

/** A view the activity rail opens in the primary sidebar. One at a time. */
export type IBoardSidebarView =
	| "explorer"
	| "search"
	| "variables"
	| "events"
	| "comments";

/** A tab of the bottom panel. */
export type IBoardPanelTab = "problems" | "runs" | "traces" | "tests";

/** A view stacked in the secondary sidebar. */
export type IBoardSecondaryView = "inspector" | "flowpilot";

/** Every surface that takes over the screen below `md`, where there is no room to dock. */
export type IBoardMobileSurface =
	| IBoardSidebarView
	| IBoardPanelTab
	| IBoardSecondaryView
	| "script"
	| "pages";

export interface IBoardSurfaceState {
	sidebar: IBoardSidebarView | null;
	panel: IBoardPanelTab | null;
	secondary: IBoardSecondaryView | null;
	/** FlowScript is split beside the graph in the editor group. */
	script: boolean;
	mobile: IBoardMobileSurface | null;
}

type Action = (
	| { type: "sidebar"; view: IBoardSidebarView | null; toggle?: boolean }
	| { type: "panel"; tab: IBoardPanelTab | null; toggle?: boolean }
	| { type: "secondary"; view: IBoardSecondaryView | null; toggle?: boolean }
	| { type: "script"; open: boolean | null }
	| { type: "mobile"; surface: IBoardMobileSurface | null }
	| { type: "hydrate"; state: Partial<IBoardSurfaceState> }
) & {
	/**
	 * The viewport has no room to dock. The surface still opens in state — one
	 * body then renders in both places — and `mobile` names which of them the
	 * drawer shows.
	 */
	drawer?: boolean;
};

const INITIAL: IBoardSurfaceState = {
	sidebar: null,
	panel: null,
	secondary: null,
	script: false,
	mobile: null,
};

/** The drawer follows whatever the action just opened, and closes with it. */
function withDrawer(
	state: IBoardSurfaceState,
	action: Action,
	opened: IBoardMobileSurface | null,
): IBoardSurfaceState {
	if (!action.drawer) return state;
	return state.mobile === opened ? state : { ...state, mobile: opened };
}

function reducer(
	state: IBoardSurfaceState,
	action: Action,
): IBoardSurfaceState {
	switch (action.type) {
		case "sidebar": {
			const next =
				action.toggle && state.sidebar === action.view ? null : action.view;
			return withDrawer(
				next === state.sidebar ? state : { ...state, sidebar: next },
				action,
				next,
			);
		}
		case "panel": {
			const next =
				action.toggle && state.panel === action.tab ? null : action.tab;
			return withDrawer(
				next === state.panel ? state : { ...state, panel: next },
				action,
				next,
			);
		}
		case "secondary": {
			const next =
				action.toggle && state.secondary === action.view ? null : action.view;
			return withDrawer(
				next === state.secondary ? state : { ...state, secondary: next },
				action,
				next,
			);
		}
		case "script": {
			const next = action.open === null ? !state.script : action.open;
			return withDrawer(
				next === state.script ? state : { ...state, script: next },
				action,
				next ? "script" : null,
			);
		}
		case "mobile":
			return state.mobile === action.surface
				? state
				: { ...state, mobile: action.surface };
		case "hydrate":
			return { ...state, ...action.state, mobile: null };
		default:
			return state;
	}
}

const STORAGE_KEY = "flow-board-shell";

type Persisted = Pick<
	IBoardSurfaceState,
	"sidebar" | "panel" | "secondary" | "script"
>;

function readPersisted(): Partial<Persisted> {
	if (typeof window === "undefined") return {};
	try {
		const raw = window.localStorage.getItem(STORAGE_KEY);
		return raw ? (JSON.parse(raw) as Partial<Persisted>) : {};
	} catch {
		return {};
	}
}

function writePersisted(state: IBoardSurfaceState): void {
	if (typeof window === "undefined") return;
	try {
		window.localStorage.setItem(
			STORAGE_KEY,
			JSON.stringify({
				sidebar: state.sidebar,
				panel: state.panel,
				secondary: state.secondary,
				script: state.script,
			} satisfies Persisted),
		);
	} catch {
		/* private mode — the shell just opens at its defaults next time */
	}
}

export interface IBoardSurfaceActions {
	toggleSidebar: (view: IBoardSidebarView) => void;
	openSidebar: (view: IBoardSidebarView) => void;
	closeSidebar: () => void;
	togglePanel: (tab: IBoardPanelTab) => void;
	openPanel: (tab: IBoardPanelTab) => void;
	closePanel: () => void;
	toggleSecondary: (view: IBoardSecondaryView) => void;
	openSecondary: (view: IBoardSecondaryView) => void;
	closeSecondary: () => void;
	toggleScript: () => void;
	openScript: () => void;
	closeScript: () => void;
	openMobile: (surface: IBoardMobileSurface) => void;
	closeMobile: () => void;
}

/**
 * The single owner of which board surfaces are open.
 *
 * It replaces six independent booleans and four imperative panel handles. Two
 * behaviours matter and neither was reachable before: only one primary-sidebar
 * view can be open, so side panels stop competing for the same edge; and below
 * `md` every open request is routed to `mobile` instead, so one drawer host
 * renders what desktop docks.
 */
export function useBoardSurface(isMobile: boolean): {
	surface: IBoardSurfaceState;
	actions: IBoardSurfaceActions;
} {
	const [surface, dispatch] = useReducer(reducer, INITIAL);
	const mobileRef = useRef(isMobile);
	mobileRef.current = isMobile;

	useEffect(() => {
		const persisted = readPersisted();
		if (Object.keys(persisted).length > 0) {
			dispatch({ type: "hydrate", state: persisted });
		}
	}, []);

	useEffect(() => {
		writePersisted(surface);
	}, [surface]);

	const actions = useMemo<IBoardSurfaceActions>(() => {
		const drawer = () => mobileRef.current;
		return {
			toggleSidebar: (view) =>
				dispatch({ type: "sidebar", view, toggle: true, drawer: drawer() }),
			openSidebar: (view) =>
				dispatch({ type: "sidebar", view, drawer: drawer() }),
			closeSidebar: () => dispatch({ type: "sidebar", view: null }),
			togglePanel: (tab) =>
				dispatch({ type: "panel", tab, toggle: true, drawer: drawer() }),
			openPanel: (tab) => dispatch({ type: "panel", tab, drawer: drawer() }),
			closePanel: () => dispatch({ type: "panel", tab: null }),
			toggleSecondary: (view) =>
				dispatch({ type: "secondary", view, toggle: true, drawer: drawer() }),
			openSecondary: (view) =>
				dispatch({ type: "secondary", view, drawer: drawer() }),
			closeSecondary: () => dispatch({ type: "secondary", view: null }),
			toggleScript: () =>
				dispatch({ type: "script", open: null, drawer: drawer() }),
			openScript: () =>
				dispatch({ type: "script", open: true, drawer: drawer() }),
			closeScript: () => dispatch({ type: "script", open: false }),
			openMobile: (target) => dispatch({ type: "mobile", surface: target }),
			closeMobile: () => dispatch({ type: "mobile", surface: null }),
		};
	}, []);

	// Widening past `md` docks whatever the drawer was showing, rather than
	// leaving a sheet floating over a layout that now has room for it.
	useEffect(() => {
		if (!isMobile) dispatch({ type: "mobile", surface: null });
	}, [isMobile]);

	return { surface, actions };
}
