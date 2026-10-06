"use client";

import {
	type KeyboardEvent as ReactKeyboardEvent,
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useRef,
} from "react";
import type {
	FormSessionActions,
	FormSessionState,
	NavigateIntent,
	RouteNav,
	WorkbenchLayout,
	WorkbenchViewProps,
} from "../contracts";
import {
	findFocusable,
	selectContents,
	showFocusRings,
	useIsoLayoutEffect,
} from "./focus";
import { handleRootKeyDown } from "./keyboard";
import { hasWaiting, leaveCountsOf } from "./leave";
import { type BoxSize, sameLayout, useBoxLayout } from "./use-box-layout";
import { useLatest } from "./use-latest";

/** The layout measured on the root, reported to the session; the measured one wins in the frame before the session has stored it. */
export function useShellLayout(
	rootRef: RefObject<HTMLElement | null>,
	state: FormSessionState,
	actions: FormSessionActions,
	initialSize: BoxSize | undefined,
) {
	const measured = useBoxLayout(rootRef, initialSize);
	const reported = useRef<WorkbenchLayout | null>(null);
	useEffect(() => {
		if (!measured || sameLayout(measured, state.layout)) return;
		if (reported.current === measured && state.layout !== null) return;
		reported.current = measured;
		actions.setLayout(measured);
	}, [measured, state.layout, actions]);
	return measured ?? state.layout;
}

export interface RouteNavInput {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
	readonly navigate: (intent: NavigateIntent) => void;
	readonly labels: Readonly<Record<string, string>>;
}

/** Every route button goes through one guard: it asks first while runs or next files wait (spec M5). */
export function useRouteNav(input: RouteNavInput) {
	const { state, actions, navigate, labels } = input;
	const waiting = hasWaiting(leaveCountsOf(state));
	const latest = useLatest({ waiting, navigate });
	const go = useCallback(
		(route: string) => {
			const { waiting: mustAsk, navigate: leave } = latest.current;
			if (mustAsk) actions.openOverlay({ id: "leave", route, replace: false });
			else leave({ route, replace: false });
		},
		[actions, latest],
	);
	return useMemo<RouteNav>(() => ({ labels, go }), [labels, go]);
}

/** Focus an element once the next render has put it in the DOM (a filter that is not on screen yet). */
function useDeferredFocus(rootRef: RefObject<HTMLElement | null>) {
	const pending = useRef<string | null>(null);
	useIsoLayoutEffect(() => {
		const value = pending.current;
		if (value === null) return;
		pending.current = null;
		const element = rootRef.current
			? findFocusable(rootRef.current, value)
			: null;
		if (!element) return;
		element.focus();
		selectContents(element);
	});
	return useCallback((value: string) => {
		pending.current = value;
		setTimeout(() => {
			if (pending.current === value) pending.current = null;
		}, 100);
	}, []);
}

/** The root's key handlers: chords on `keydown`, and the first key or click gives the focus ring back (S6). */
export function useShellKeys(
	rootRef: RefObject<HTMLElement | null>,
	state: FormSessionState,
	actions: FormSessionActions,
) {
	const focusSoon = useDeferredFocus(rootRef);
	const keyContext = useLatest({ state, actions, focusSoon });
	const onKeyDown = useCallback(
		(event: ReactKeyboardEvent<HTMLElement>) =>
			handleRootKeyDown(event, {
				...keyContext.current,
				root: rootRef.current,
			}),
		[keyContext, rootRef],
	);
	const showRings = useCallback(
		() => showFocusRings(rootRef.current),
		[rootRef],
	);
	return { onKeyDown, showRings };
}

/** The views get the layout the shell measured, also in the one frame before the session has stored it. */
export function useViewProps(
	state: FormSessionState,
	actions: FormSessionActions,
	layout: WorkbenchLayout | null,
	routes: RouteNav,
) {
	return useMemo<WorkbenchViewProps | null>(
		() =>
			layout
				? {
						state: state.layout === layout ? state : { ...state, layout },
						actions,
						layout,
						routes,
					}
				: null,
		[layout, state, actions, routes],
	);
}
