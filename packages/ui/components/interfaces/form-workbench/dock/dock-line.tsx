"use client";

import { type RefObject, useMemo } from "react";
import type { FormSessionActions, FormSessionState } from "../contracts";
import { stepKeysOf } from "./dock-view";
import { MessageLiveRegion, StatusLine } from "./status-line";
import {
	type DockWords,
	useReveal,
	useRunFocus,
	useStatusLine,
} from "./use-dock";
import { useFieldStepper } from "./use-field-stepper";

/**
 * The dock's status line with everything around it: the ▲▼ stepping to fields, the focus that goes back to Run when a
 * line button disappears, and the hidden mirror that reads a message out. `phone` trims the line to what the narrow
 * layout shows and draws it as its own bar above the dock.
 */
export function DockLine({
	state,
	actions,
	kit,
	dockRef,
	runRef,
	phone,
	big,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	kit: DockWords;
	dockRef: RefObject<HTMLElement | null>;
	runRef: RefObject<HTMLButtonElement | null>;
	phone: boolean;
	big: boolean;
}>) {
	const { line, view } = useStatusLine(state, actions, kit, phone);
	const { afterAction } = useRunFocus(runRef);
	const keys = useMemo(() => stepKeysOf(state, line), [state, line]);
	const reveal = useReveal(actions);
	const step = useFieldStepper({ dockRef, keys, reveal });
	return (
		<>
			{view ? (
				<StatusLine
					view={view}
					big={big}
					bar={phone}
					onStep={step}
					onActed={afterAction}
				/>
			) : null}
			<MessageLiveRegion text={view?.kind === "message" ? view.text : ""} />
		</>
	);
}
