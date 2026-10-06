"use client";

import { useTranslation } from "@flow-like/locales";
import { OctagonX } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useCallback, useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	FOCUS_ATTR,
	type FieldControlProps,
	type FieldProblem,
	type ViewerHabits,
	type WorkbenchField,
} from "../contracts";
import { type Bind, describedByOf, focusValueOf, useFieldIds } from "./bind";
import { isComposing, isResetChord } from "./keys";
import { LabelLine } from "./label-line";
import { problemText } from "./problem-text";

/*
 * The frame around every control: label line, the control, the message of its problem and its help
 * (spec M1, SURFACE §6). One frame per field and per object property; groups nest frames.
 */

interface MessageProps {
	readonly id: string;
	readonly children: ReactNode;
}

export function Message({ id, children }: Readonly<MessageProps>) {
	return (
		<p
			id={id}
			role="alert"
			className="m-0 flex items-start gap-1.25 text-xs/4 text-critical"
		>
			<OctagonX aria-hidden className="mt-[1.5px] size-3.25 shrink-0" />
			<span>{children}</span>
		</p>
	);
}

interface HelpProps {
	readonly id: string;
	readonly text: string;
}

function Help({ id, text }: Readonly<HelpProps>) {
	return (
		<p id={id} className="m-0 text-xs/4 text-pretty text-muted-foreground">
			{text}
		</p>
	);
}

/** The text of the field's problem, or null. */
function useProblemText(
	field: WorkbenchField,
	problem: FieldProblem | undefined,
	viewer: Pick<ViewerHabits, "dateLocale" | "decimalSign">,
) {
	const { t } = useTranslation("interfaces");
	if (!problem) return null;
	return problemText({ t, field, viewer, problem });
}

export interface FrameOptions {
	/** The label line is hidden: the control carries its own label (a switch row on a phone). */
	readonly hideLabelLine?: boolean;
	readonly labelFor: boolean;
	/** The control's own hint is part of `aria-describedby` (the date's "Alt and Down open the calendar."). */
	readonly hint?: boolean;
	readonly nested: boolean;
	readonly extra?: ReactNode;
}

/** The first element of the frame the shell may focus (a group has none of its own: its first property). */
function focusTargetOf(frame: HTMLElement | null) {
	return frame?.querySelector<HTMLElement>(`[${FOCUS_ATTR}]`) ?? null;
}

function useResetChord(props: FieldControlProps, reset: (() => void) | null) {
	return useCallback(
		(event: KeyboardEvent) => {
			if (!reset || event.repeat || isComposing(event)) return;
			if (!isResetChord(event, props.viewer.mac)) return;
			event.preventDefault();
			event.stopPropagation();
			reset();
		},
		[props.viewer.mac, reset],
	);
}

export interface FrameState {
	readonly bind: Bind;
	readonly message: string | null;
}

/** Ids, markers, problem and the focus attribute of one field. */
export function useFrameState(
	props: FieldControlProps,
	options: Pick<FrameOptions, "hint">,
): FrameState {
	const ids = useFieldIds();
	const { field, rail } = props;
	const markers = props.markersFor(field.key);
	const problem = rail.problems[field.key];
	const message = useProblemText(field, problem, props.viewer);
	const hasHelp = field.help !== null;
	const hint = options.hint === true;
	const describedBy = (withHint = hint) =>
		describedByOf(
			ids,
			{ problem: problem !== undefined, help: hasHelp },
			withHint,
		);
	const bind: Bind = {
		ids,
		markers,
		problem,
		invalid: problem !== undefined,
		describedBy,
		touch: props.layout.touch,
		focus: { [FOCUS_ATTR]: focusValueOf(field.key) },
	};
	return { bind, message };
}

export function FieldFrame({
	props,
	state,
	options,
	children,
}: Readonly<{
	props: FieldControlProps;
	state: FrameState;
	options: FrameOptions;
	children: ReactNode;
}>) {
	const { field, actions } = props;
	const { bind, message } = state;
	const frame = useRef<HTMLDivElement>(null);
	const reset =
		bind.markers.reset === null
			? null
			: () => {
					actions.resetField(field.key);
					focusTargetOf(frame.current)?.focus();
				};
	const onKeyDown = useResetChord(props, reset);
	return (
		<div
			ref={frame}
			onKeyDown={onKeyDown}
			data-fw-field={field.key}
			className={cx(
				"flex min-w-0 flex-col gap-1.5 @container/field",
				options.nested ? "group/prop" : "group/field",
			)}
		>
			{options.hideLabelLine ? null : (
				<LabelLine
					field={field}
					bind={bind}
					labelFor={options.labelFor}
					nested={options.nested}
					extra={options.extra}
					onMarker={() =>
						actions.openOverlay({ id: "afterRun", focusName: field.name })
					}
					onReset={reset ?? (() => undefined)}
				/>
			)}
			{children}
			{message === null ? null : (
				<Message id={bind.ids.error}>{message}</Message>
			)}
			{field.help === null ? null : (
				<Help id={bind.ids.help} text={field.help} />
			)}
		</div>
	);
}
