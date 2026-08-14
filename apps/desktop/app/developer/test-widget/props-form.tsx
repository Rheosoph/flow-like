"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Badge,
	Button,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
	Textarea,
	cn,
} from "@flow-like/flow-like-ui";
import type { ContractInput, WidgetContract } from "@flow-like/widget-sdk";
import { useId } from "react";
import type {
	WidgetPropDraftValue,
	WidgetPropsDraft,
} from "../../../lib/widget-props-form";
import { emptyWidgetPropDraft } from "../../../lib/widget-props-form";
import { homogeneousArrayItemSchema } from "../../../lib/widget-schema-form";
import { SchemaListField } from "./schema-list-field";

interface WidgetPropsFormProps {
	contract: WidgetContract;
	draft: WidgetPropsDraft;
	errors: Record<string, string[]>;
	onChange: (key: string, value: WidgetPropDraftValue) => void;
}

interface WidgetPropFieldProps {
	id: string;
	name: string;
	input: ContractInput;
	value: WidgetPropDraftValue;
	errors: string[];
	onChange: (value: WidgetPropDraftValue) => void;
}

function errorText(error: string): string {
	return error.replace(/^\$:\s*/, "");
}

function enumOptionValue(choice: string): string {
	return JSON.stringify(choice);
}

function WidgetPropField({
	id,
	name,
	input,
	value,
	errors,
	onChange,
}: WidgetPropFieldProps) {
	const { t } = useTranslation("common");
	const errorId = `${id}-error`;
	const labelId = `${id}-label`;
	const descriptionId = input.description ? `${id}-description` : undefined;
	const hasBounds = input.min !== undefined || input.max !== undefined;
	const boundsId = hasBounds ? `${id}-bounds` : undefined;
	const invalid = errors.length > 0;
	const uniqueErrors = [...new Set(errors)];
	const unset = value === undefined;
	const controlDisabled = input.optional === true && unset;
	const requiredUnset = input.optional !== true && unset;
	const isList =
		input.type === "json" &&
		input.schema !== undefined &&
		homogeneousArrayItemSchema(input.schema) !== null;
	const describedBy = [descriptionId, boundsId, invalid ? errorId : undefined]
		.filter(Boolean)
		.join(" ");

	const control = (() => {
		switch (input.type) {
			case "boolean":
				return (
					<div className="flex h-9 items-center gap-2">
						<Switch
							id={id}
							checked={value === true}
							onCheckedChange={onChange}
							disabled={controlDisabled}
							aria-invalid={invalid}
							aria-describedby={describedBy || undefined}
						/>
						<span className="text-xs text-muted-foreground">
							{value === true ? "True" : "False"}
						</span>
					</div>
				);
			case "number":
			case "integer":
				return (
					<Input
						id={id}
						type="number"
						value={typeof value === "string" ? value : ""}
						min={
							input.min === undefined
								? undefined
								: input.type === "integer"
									? Math.ceil(input.min)
									: input.min
						}
						max={
							input.max === undefined
								? undefined
								: input.type === "integer"
									? Math.floor(input.max)
									: input.max
						}
						step={input.type === "integer" ? 1 : "any"}
						disabled={controlDisabled}
						onChange={(event) => onChange(event.target.value)}
						aria-invalid={invalid}
						aria-describedby={describedBy || undefined}
						className="h-9"
					/>
				);
			case "enum": {
				const choices = [...new Set(input.choices ?? [])];
				return (
					<Select
						value={
							typeof value === "string" ? enumOptionValue(value) : undefined
						}
						onValueChange={(encoded) => {
							const choice: unknown = JSON.parse(encoded);
							if (typeof choice === "string") onChange(choice);
						}}
						disabled={controlDisabled}
					>
						<SelectTrigger
							id={id}
							className="h-9"
							aria-invalid={invalid}
							aria-describedby={describedBy || undefined}
						>
							<SelectValue placeholder={t('selectAValue', 'Select a value…')} />
						</SelectTrigger>
						<SelectContent>
							{choices.map((choice) => (
								<SelectItem
									key={enumOptionValue(choice)}
									value={enumOptionValue(choice)}
								>
									{choice || "Empty string"}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				);
			}
			case "json": {
				const schema = input.schema;
				if (schema && isList) {
					return (
						<SchemaListField
							fieldName={name}
							id={id}
							labelledBy={labelId}
							schema={schema}
							value={value}
							disabled={controlDisabled}
							describedBy={describedBy || undefined}
							onChange={onChange}
						/>
					);
				}
				return (
					<Textarea
						id={id}
						value={typeof value === "string" ? value : ""}
						onChange={(event) => onChange(event.target.value)}
						rows={6}
						spellCheck={false}
						disabled={controlDisabled}
						aria-invalid={invalid}
						aria-describedby={describedBy || undefined}
						className="font-mono text-xs"
					/>
				);
			}
			default:
				return (
					<Input
						id={id}
						value={typeof value === "string" ? value : ""}
						onChange={(event) => onChange(event.target.value)}
						disabled={controlDisabled}
						aria-invalid={invalid}
						aria-describedby={describedBy || undefined}
						className="h-9"
					/>
				);
		}
	})();

	return (
		<div
			className={cn(
				"space-y-2 rounded-lg border border-border/30 bg-card/30 p-3",
				input.type === "json" && "lg:col-span-2",
			)}
		>
			<div className="flex items-center justify-between gap-3">
				{isList ? (
					<span id={labelId} className="font-medium">
						{name}
					</span>
				) : (
					<Label id={labelId} htmlFor={id} className="font-medium">
						{name}
					</Label>
				)}
				<div className="flex items-center gap-1.5">
					<Badge variant="outline" className="text-[10px] font-normal">
						{isList ? "list" : input.type}
					</Badge>
					{input.optional && (
						<Badge variant="secondary" className="text-[10px] font-normal">
							{t('optional', 'Optional')}
						</Badge>
					)}
				</div>
			</div>

			{input.description && (
				<p
					id={descriptionId}
					className="text-xs leading-5 text-muted-foreground"
				>
					{input.description}
				</p>
			)}

			{input.optional && (
				<div className="flex items-center gap-2">
					<Switch
						id={`${id}-included`}
						checked={!unset}
						onCheckedChange={(included) =>
							onChange(included ? emptyWidgetPropDraft(input) : undefined)
						}
						aria-describedby={describedBy || undefined}
					/>
					<Label
						htmlFor={`${id}-included`}
						className="text-xs font-normal text-muted-foreground"
					>
						{t('includeValue', 'Include value')}
					</Label>
				</div>
			)}

			{requiredUnset ? (
				<Button
					id={id}
					type="button"
					variant="outline"
					size="sm"
					onClick={() => onChange(emptyWidgetPropDraft(input))}
					aria-label={t('setAValueForName', 'Set a value for {{name}}', { name })}
					aria-describedby={describedBy || undefined}
				>
					{t('setValue', 'Set value')}
				</Button>
			) : (
				control
			)}

			{hasBounds && (
				<p id={boundsId} className="text-[10px] text-muted-foreground">
					{input.min !== undefined ? t('minimumMin', 'Minimum {{min}}', { min: input.min }) : ""}
					{input.min !== undefined && input.max !== undefined ? ` · ` : ""}
					{input.max !== undefined ? t('maximumMax', 'Maximum {{max}}', { max: input.max }) : ""}
				</p>
			)}

			{invalid && (
				<div id={errorId} className="space-y-0.5" role="alert">
					{uniqueErrors.map((error) => (
						<p key={error} className="text-xs text-destructive">
							{errorText(error)}
						</p>
					))}
				</div>
			)}
		</div>
	);
}

export function WidgetPropsForm({
	contract,
	draft,
	errors,
	onChange,
}: WidgetPropsFormProps) {
	const { t } = useTranslation("common");
	const inputs = Object.entries(contract.inputs ?? {});
	const idPrefix = useId();

	if (inputs.length === 0) {
		return (
			<div className="rounded-lg border border-dashed border-border/40 p-6 text-center text-sm text-muted-foreground">
				{t('thisWidgetDoesNotDeclareAnyProps', 'This widget does not declare any props.')}
			</div>
		);
	}

	return (
		<div className="grid gap-3 lg:grid-cols-2">
			{inputs.map(([name, input]) => (
				<WidgetPropField
					key={`${contract.id}-${name}`}
					id={`${idPrefix}-${name}`}
					name={name}
					input={input}
					value={draft[name]}
					errors={errors[name] ?? []}
					onChange={(value) => onChange(name, value)}
				/>
			))}
		</div>
	);
}
