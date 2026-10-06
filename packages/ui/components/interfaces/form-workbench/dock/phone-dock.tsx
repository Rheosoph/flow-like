"use client";

import { type RefObject, useMemo, useRef, useState } from "react";
import {
	type DockProps,
	FORM_LIMITS,
	type FormSessionActions,
	type FormSessionState,
} from "../contracts";
import { DockLine } from "./dock-line";
import { DockStop, PaneSwitch } from "./dock-parts";
import {
	knownPerRunNames,
	makeOffered,
	perRunRowsOf,
	stopTargetOf,
} from "./dock-view";
import { ROOT_SELECTOR } from "./focus";
import { PerRunSheet } from "./per-run-popover";
import { RunButton } from "./run-button";
import {
	type DockWords,
	useDockWords,
	useRunControl,
	useRunFocus,
} from "./use-dock";

/** The interface root the sheet and its scrim live in (the document body where there is none: the visual harness). */
function containerOf(anchor: HTMLElement | null) {
	if (!anchor) return null;
	return (
		anchor.closest<HTMLElement>(ROOT_SELECTOR) ?? anchor.ownerDocument.body
	);
}

function SheetHost({
	state,
	actions,
	kit,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	kit: DockWords;
}>) {
	const [anchor, setAnchor] = useState<HTMLElement | null>(null);
	const { overlay } = state.view;
	const names = useMemo(() => knownPerRunNames(state), [state]);
	const rows = useMemo(
		() => perRunRowsOf(state, names, kit.words),
		[state, names, kit.words],
	);
	return (
		<>
			<span ref={setAnchor} hidden />
			<PerRunSheet
				open={overlay?.id === "afterRun"}
				container={containerOf(anchor)}
				focusName={overlay?.id === "afterRun" ? overlay.focusName : null}
				onOpenChange={(next) => {
					if (!next) actions.closeOverlay();
				}}
				panel={{
					rows,
					words: kit.words,
					showMake: makeOffered(state, names),
					showFilter: state.form.fields.length >= FORM_LIMITS.filterFromFields,
					large: true,
					actions,
				}}
			/>
		</>
	);
}

function PhoneBar({
	state,
	actions,
	kit,
	runRef,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	kit: DockWords;
	runRef: RefObject<HTMLButtonElement | null>;
}>) {
	const run = useRunControl(state, actions, "phone", kit);
	const { focusRun } = useRunFocus(runRef);
	const stop = stopTargetOf(state);
	return (
		<div className="flex h-14 items-center gap-2 border-border border-t bg-card px-3">
			{state.form.fields.length > 0 ? (
				<PaneSwitch state={state} actions={actions} kit={kit} />
			) : null}
			{stop ? (
				<DockStop
					run={stop}
					actions={actions}
					kit={kit}
					large
					iconOnly
					onStopped={focusRun}
				/>
			) : null}
			<RunButton
				ref={runRef}
				variant="phone"
				label={run.label}
				title={run.title}
				capped={run.capped}
				large
				mac={kit.copy.mac}
				onRun={run.onRun}
			/>
		</div>
	);
}

/**
 * The narrow layout's bottom bar (spec M1 phone, M5): the status line above it, then Inputs / Output, Stop for the run
 * on the stage and Run. 56 px, 46 px controls. It also hosts the "Per run" bottom sheet its Inputs pane opens.
 */
export function PhoneDock({ state, actions }: Readonly<DockProps>) {
	const kit = useDockWords(state);
	const rootRef = useRef<HTMLDivElement>(null);
	const runRef = useRef<HTMLButtonElement>(null);
	return (
		<div
			ref={rootRef}
			data-fw-dock="phone"
			className="relative flex flex-none flex-col"
		>
			<PhoneBar state={state} actions={actions} kit={kit} runRef={runRef} />
			<DockLine
				state={state}
				actions={actions}
				kit={kit}
				dockRef={rootRef}
				runRef={runRef}
				phone
				big
			/>
			<SheetHost state={state} actions={actions} kit={kit} />
		</div>
	);
}
