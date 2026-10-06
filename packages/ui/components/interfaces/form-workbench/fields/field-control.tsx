"use client";

import type { ComponentType } from "react";
import {
	FIELD_KEY_SEPARATOR,
	type FieldControlProps,
	type FieldKind,
} from "../contracts";
import type { ControlProps } from "./bind";
import { BoolField } from "./bool-field";
import { ChipsField } from "./chips-field";
import { ChoiceField } from "./choice-field";
import { DateField } from "./date-field";
import { FieldFrame, type FrameOptions, useFrameState } from "./field-frame";
import { FileField } from "./file-field";
import { GroupField } from "./group-field";
import { JsonField, JsonHint } from "./json-field";
import { MaskedField } from "./masked-field";
import { NumberField } from "./number-field";
import { PairsField } from "./pairs-field";
import { TextField } from "./text-field";
import { UnsupportedField } from "./unsupported-field";

/** A sensitive text is a masked input (spec M6). */
function TextOrMasked(props: ControlProps) {
	return props.field.sensitive ? (
		<MaskedField {...props} />
	) : (
		<TextField {...props} />
	);
}

/** A sensitive number is a masked input too. */
function NumberOrMasked(props: ControlProps) {
	return props.field.sensitive ? (
		<MaskedField {...props} />
	) : (
		<NumberField {...props} />
	);
}

const CONTROLS: Readonly<Record<FieldKind, ComponentType<ControlProps>>> = {
	text: TextOrMasked,
	number: NumberOrMasked,
	bool: BoolField,
	date: DateField,
	choice: ChoiceField,
	chips: ChipsField,
	file: FileField,
	files: FileField,
	group: GroupField,
	pairs: PairsField,
	json: JsonField,
	unsupported: UnsupportedField,
};

/** Labels that point at their control (`<label for>`); choices, files, groups and switches are named by `aria-labelledby`. */
const LABEL_FOR: Readonly<Record<FieldKind, boolean>> = {
	text: true,
	number: true,
	date: true,
	chips: true,
	json: true,
	bool: false,
	choice: false,
	file: false,
	files: false,
	group: false,
	pairs: false,
	unsupported: false,
};

/**
 * One control with its label line, help and message (spec M1, SURFACE §4). Groups render their properties with
 * nested `FieldControl`s and pass the same callbacks, so every property has its own markers, recent values and
 * date anchor. Every focusable element carries `data-fw-focus="field:<key>"`.
 */
export function FieldControl(props: Readonly<FieldControlProps>) {
	const { field, layout } = props;
	const state = useFrameState(props, {
		hint: field.kind === "date" && layout.finePointer,
	});
	const Control = CONTROLS[field.kind];
	const options: FrameOptions = {
		labelFor: LABEL_FOR[field.kind],
		hideLabelLine: field.kind === "bool" && layout.touch,
		nested: field.key.includes(FIELD_KEY_SEPARATOR),
		extra: field.kind === "json" ? <JsonHint /> : undefined,
	};
	return (
		<FieldFrame props={props} state={state} options={options}>
			<Control {...props} bind={state.bind} />
		</FieldFrame>
	);
}
