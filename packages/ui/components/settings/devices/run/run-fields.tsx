"use client";

import { useTranslation } from "@flow-like/locales";
import { OctagonX } from "lucide-react";
import { useId } from "react";
import type { EventFormField } from "../../../../lib/device-management/model/types";
import {
	type FieldProblem,
	type FieldShape,
	type FieldValue,
	fieldShape,
} from "../../../../lib/event-form";
import type { DevicesT } from "../primitives/area-context";
import {
	DvInput,
	DvSelect,
	DvTextarea,
	Field,
	SecretInput,
	SwitchField,
} from "../primitives/form-fields";
import { cap, fieldProblemText } from "./run-copy";

/** An optional choice left empty: Radix Select can't hold an empty value. */
const NO_CHOICE = "⁣none";

/** What `Field` hands its control. */
interface Wiring {
	id?: string;
	"aria-describedby"?: string;
	"aria-invalid"?: boolean;
}

interface ControlProps extends Wiring {
	field: EventFormField;
	shape: FieldShape;
	value: FieldValue;
	onValueChange(value: FieldValue): void;
	disabled?: boolean;
}

const textOf = (value: FieldValue) => (typeof value === "string" ? value : "");

function ChoiceControl({
	field,
	shape,
	value,
	onValueChange,
	...wiring
}: Readonly<ControlProps>) {
	const { t } = useTranslation("devices");
	const options = (field.options ?? []).map((option) => ({
		value: option,
		label: cap(option, 64),
	}));
	const choices = shape.required
		? options
		: [
				{
					value: NO_CHOICE,
					label: t("runNow.field.noChoice", "No value · the flow's default"),
				},
				...options,
			];
	const current = textOf(value);
	const empty = shape.required ? undefined : NO_CHOICE;
	return (
		<DvSelect
			{...wiring}
			value={current === "" ? empty : current}
			onValueChange={(next) => onValueChange(next === NO_CHOICE ? "" : next)}
			options={choices}
			placeholder={t("runNow.field.choose", "Choose a value")}
		/>
	);
}

/** One control per field kind; `Field` adds the label, hint and problem. */
function FieldControl(props: Readonly<ControlProps>) {
	const { field, shape, value, onValueChange, ...wiring } = props;
	const text = textOf(value);
	const typed = {
		...wiring,
		value: text,
		onChange: (event: { target: { value: string } }) =>
			onValueChange(event.target.value),
	};
	switch (shape.control) {
		case "select":
			return <ChoiceControl {...props} />;
		case "password":
			return (
				<SecretInput
					{...wiring}
					value={text}
					onValueChange={onValueChange}
					autoComplete="off"
				/>
			);
		case "json":
			return (
				<DvTextarea
					{...typed}
					rows={4}
					spellCheck={false}
					className="font-mono text-xs"
				/>
			);
		case "integer":
			return <DvInput {...typed} numeric inputMode="numeric" />;
		case "number":
			return <DvInput {...typed} numeric inputMode="decimal" />;
		case "date":
			return <DvInput {...typed} type="date" />;
		default:
			return <DvInput {...typed} />;
	}
}

function FieldLabel({
	field,
	shape,
}: Readonly<{ field: EventFormField; shape: FieldShape }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			{cap(field.label || field.name, 120)}
			{shape.required ? (
				<span className="ml-1.5 font-normal text-muted-foreground">
					{t("runNow.field.requiredMark", "required")}
				</span>
			) : null}
		</>
	);
}

function fieldHint(
	t: DevicesT,
	field: EventFormField,
	shape: FieldShape,
): string | undefined {
	const parts = [
		field.description ? cap(field.description) : null,
		field.default_omitted && !shape.required
			? t(
					"devices:runNow.field.defaultOmitted",
					"Its default is too large to show here. Left empty, the flow uses it.",
				)
			: null,
		shape.control === "password"
			? t(
					"devices:runNow.field.sensitive",
					"Hidden while you type. It is part of the run's input all the same.",
				)
			: null,
	].filter((part): part is string => Boolean(part));
	return parts.length ? parts.join(" ") : undefined;
}

export interface RunFieldsProps {
	fields: readonly EventFormField[];
	values: Readonly<Record<string, FieldValue>>;
	problems: Readonly<Record<string, FieldProblem>>;
	/** Fields the device refused in the last run: names only. */
	refused?: readonly string[];
	device: string;
	onChange(name: string, value: FieldValue): void;
	disabled?: boolean;
}

/** The form's fields with the area's controls; every text from the device is plain text. */
export function RunFields({
	fields,
	values,
	problems,
	refused = [],
	device,
	onChange,
	disabled,
}: Readonly<RunFieldsProps>) {
	const { t } = useTranslation("devices");
	const base = useId();
	return (
		<div data-run-fields="" className="flex flex-col gap-3.5">
			{fields.map((field, index) => {
				const shape = fieldShape(field);
				if (!shape) return null;
				const id = `${base}-${index}`;
				const problem = problems[field.name];
				const error = problem
					? fieldProblemText(t, problem)
					: refused.includes(field.name)
						? t("runNow.field.refused", "{{device}} refused this value.", {
								device,
							})
						: undefined;
				const value = values[field.name] ?? "";
				const hint = fieldHint(t, field, shape);
				const change = (next: FieldValue) => onChange(field.name, next);
				return (
					<div
						key={field.name}
						data-run-field={field.name}
						data-control={shape.control}
						className="flex min-w-0 flex-col gap-1"
					>
						{shape.control === "switch" ? (
							<>
								<SwitchField
									id={id}
									checked={value === true}
									onCheckedChange={change}
									disabled={disabled}
								>
									<FieldLabel field={field} shape={shape} />
								</SwitchField>
								{error ? (
									<p className="flex items-start gap-1 text-xs text-critical">
										<OctagonX
											aria-hidden
											className="mt-px size-3.25 shrink-0"
										/>
										<span>{error}</span>
									</p>
								) : null}
								{hint ? (
									<p className="text-xs text-muted-foreground">{hint}</p>
								) : null}
							</>
						) : (
							<Field
								id={id}
								label={<FieldLabel field={field} shape={shape} />}
								hint={hint}
								error={error}
							>
								<FieldControl
									field={field}
									shape={shape}
									value={value}
									onValueChange={change}
									disabled={disabled}
								/>
							</Field>
						)}
					</div>
				);
			})}
		</div>
	);
}
