import { useId } from "react";
import {
	FOCUS_ATTR,
	FOCUS_VALUE,
	type FieldControlProps,
	type FieldKey,
	type FieldMarkers,
	type FieldProblem,
} from "../contracts";

/** What every control shares with its frame: ids for labels and descriptions, validity, markers, the focus hook. */
export interface FieldIds {
	readonly control: string;
	readonly label: string;
	readonly error: string;
	readonly help: string;
	readonly hint: string;
}

export interface Bind {
	readonly ids: FieldIds;
	readonly markers: FieldMarkers;
	readonly problem: FieldProblem | undefined;
	readonly invalid: boolean;
	/** `aria-describedby`: the message, the help, the control's own hint. */
	readonly describedBy: (hint?: boolean) => string | undefined;
	readonly touch: boolean;
	/** Spread on every focusable element of the field (`data-fw-focus="field:<key>"`). */
	readonly focus: { readonly [FOCUS_ATTR]: string };
}

export type ControlProps = Readonly<FieldControlProps> & {
	readonly bind: Bind;
};

export const focusValueOf = (key: FieldKey) =>
	`${FOCUS_VALUE.fieldPrefix}${key}`;

export function useFieldIds(): FieldIds {
	const id = useId();
	return {
		control: `${id}-c`,
		label: `${id}-l`,
		error: `${id}-e`,
		help: `${id}-h`,
		hint: `${id}-k`,
	};
}

export function describedByOf(
	ids: FieldIds,
	state: { problem: boolean; help: boolean },
	hint: boolean,
) {
	const parts = [
		state.problem ? ids.error : null,
		state.help ? ids.help : null,
		hint ? ids.hint : null,
	].filter(Boolean);
	return parts.length > 0 ? parts.join(" ") : undefined;
}
