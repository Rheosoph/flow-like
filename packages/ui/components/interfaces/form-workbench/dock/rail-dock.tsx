"use client";

import { type RefObject, useCallback, useMemo, useRef } from "react";
import {
	type DockProps,
	FORM_LIMITS,
	type FormSessionActions,
	type FormSessionState,
	type WorkbenchLayout,
} from "../contracts";
import { AfterRunLine } from "./after-run";
import { DockLine } from "./dock-line";
import { DockStop } from "./dock-parts";
import {
	type AfterRunView,
	afterRunOf,
	makeOffered,
	perRunRowsOf,
	stopTargetOf,
} from "./dock-view";
import { focusField, interfaceRootOf } from "./focus";
import { PerRunPopoverContent } from "./per-run-popover";
import { RunButton } from "./run-button";
import {
	type DockWords,
	useDockWords,
	useRunControl,
	useRunFocus,
} from "./use-dock";

/** The popover's room: min(440 px, the interface's height − 140 px), never smaller than a few rows. */
const popoverHeight = (layout: WorkbenchLayout) =>
	Math.max(200, Math.min(440, layout.height - 140));

function AfterRunHost({
	state,
	actions,
	layout,
	kit,
	view,
	dockRef,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	layout: WorkbenchLayout;
	kit: DockWords;
	view: AfterRunView;
	dockRef: RefObject<HTMLElement | null>;
}>) {
	const { overlay } = state.view;
	const open = overlay?.id === "afterRun";
	const rows = useMemo(
		() => perRunRowsOf(state, view.names, kit.words),
		[state, view.names, kit.words],
	);
	const returnFocus = useCallback(
		(name: string) => {
			const root = interfaceRootOf(dockRef.current);
			return root !== null && focusField(root, name, false);
		},
		[dockRef],
	);
	const setOpen = (next: boolean) => {
		if (next) actions.openOverlay({ id: "afterRun", focusName: null });
		else if (open) actions.closeOverlay();
	};
	return (
		<AfterRunLine
			count={view.names.length}
			keyboardIcon={view.keyboardIcon}
			mac={kit.copy.mac}
			open={open}
			onOpenChange={setOpen}
			onShortcuts={() => actions.openOverlay({ id: "shortcuts" })}
		>
			<PerRunPopoverContent
				panel={{
					rows,
					words: kit.words,
					showMake: makeOffered(state, view.names),
					showFilter: state.form.fields.length >= FORM_LIMITS.filterFromFields,
					large: layout.touch,
					actions,
				}}
				focusName={overlay?.id === "afterRun" ? overlay.focusName : null}
				maxHeight={popoverHeight(layout)}
				returnFocus={returnFocus}
			/>
		</AfterRunLine>
	);
}

function RunRow({
	state,
	actions,
	kit,
	runRef,
	large,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	kit: DockWords;
	runRef: RefObject<HTMLButtonElement | null>;
	large: boolean;
}>) {
	const run = useRunControl(state, actions, "rail", kit);
	const { focusRun } = useRunFocus(runRef);
	const stop = stopTargetOf(state);
	return (
		<div className="flex gap-2">
			<RunButton
				ref={runRef}
				variant="rail"
				label={run.label}
				title={run.title}
				capped={run.capped}
				large={large}
				mac={kit.copy.mac}
				onRun={run.onRun}
			/>
			{stop ? (
				<DockStop
					run={stop}
					actions={actions}
					kit={kit}
					large={large}
					iconOnly={false}
					onStopped={focusRun}
				/>
			) : null}
		</div>
	);
}

/**
 * The dock's fill: the rail shows through while the field list fits above it; the card fill once the list
 * scrolls under the head (`data-fw-over`) or the Runs list shows (`data-fw-tab=runs`), read from the rail
 * column (`group/rail`).
 */
const DOCK_FILL =
	"bg-transparent group-has-[[data-fw-over]]/rail:bg-card group-has-[[data-fw-tab=runs]]/rail:bg-card";

/**
 * The desktop rail's dock (spec M1, M5): the status line above Run (and Stop for the run on the stage), then the
 * after-run line once the setting has been introduced. 92 px, 116 px with the after-run line. Run comes first in the
 * page order and the line is moved up by CSS, so Tab reaches Run, Stop, the line's buttons and "Change" in that order
 * (spec M1 keyboard: "Yes" and "No" are Tab stops after Run).
 */
export function RailDock({ state, actions, layout }: Readonly<DockProps>) {
	const kit = useDockWords(state);
	const rootRef = useRef<HTMLDivElement>(null);
	const runRef = useRef<HTMLButtonElement>(null);
	const after = afterRunOf(state, layout);
	return (
		<div
			ref={rootRef}
			data-fw-dock="rail"
			className={`relative flex flex-none flex-col gap-2 border-hairline border-t px-5 pt-2.5 pb-4 ${DOCK_FILL}`}
		>
			<RunRow
				state={state}
				actions={actions}
				kit={kit}
				runRef={runRef}
				large={layout.touch}
			/>
			<DockLine
				state={state}
				actions={actions}
				kit={kit}
				dockRef={rootRef}
				runRef={runRef}
				phone={false}
				big={layout.touch}
			/>
			{after.show ? (
				<AfterRunHost
					state={state}
					actions={actions}
					layout={layout}
					kit={kit}
					view={after}
					dockRef={rootRef}
				/>
			) : null}
		</div>
	);
}
