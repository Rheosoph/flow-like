"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	CheckCircle2,
	ChevronRight,
	ChevronUp,
	Clock,
	Plus,
	Trash2,
	XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib";
import type {
	IFormField,
	IFormInteraction,
	IInteractionRequest,
	IInteractionStatus,
	IInteractionType,
	IMultipleChoiceInteraction,
	ISingleChoiceInteraction,
} from "../../../lib/schema/interaction";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../ui/radio-group";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";
import { TextEditor } from "../../ui/text-editor";
import { Textarea } from "../../ui/textarea";

interface InteractionProps {
	interaction: IInteractionRequest;
	onRespond: (interactionId: string, value: any) => void;
	disabled?: boolean;
}

function useCountdown(expiresAt: number) {
	const [remaining, setRemaining] = useState(() => {
		const now = Date.now();
		const expiresMs = expiresAt * 1000;
		return Math.max(0, Math.floor((expiresMs - now) / 1000));
	});

	useEffect(() => {
		if (remaining <= 0) return;

		const interval = setInterval(() => {
			const now = Date.now();
			const expiresMs = expiresAt * 1000;
			const left = Math.max(0, Math.floor((expiresMs - now) / 1000));
			setRemaining(left);
			if (left <= 0) clearInterval(interval);
		}, 1000);

		return () => clearInterval(interval);
	}, [expiresAt, remaining]);

	return remaining;
}

function formatCountdown(seconds: number): string {
	if (seconds <= 0) return "0s";
	const m = Math.floor(seconds / 60);
	const s = seconds % 60;
	return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

function StatusBadge({ status }: { status: IInteractionStatus }) {
	const { t } = useTranslation("chat");
	switch (status) {
		case "pending":
			return (
				<Badge
					variant="outline"
					className="gap-1 text-amber-600 border-amber-600/30"
				>
					<Clock className="h-3 w-3" />
					{t('pending', 'Pending')}
				</Badge>
			);
		case "responded":
			return (
				<Badge
					variant="outline"
					className="gap-1 text-emerald-600 border-emerald-600/30"
				>
					<CheckCircle2 className="h-3 w-3" />
					{t('responded', 'Responded')}
				</Badge>
			);
		case "expired":
			return (
				<Badge
					variant="outline"
					className="gap-1 text-red-600 border-red-600/30"
				>
					<XCircle className="h-3 w-3" />
					{t('expired', 'Expired')}
				</Badge>
			);
		case "cancelled":
			return (
				<Badge
					variant="outline"
					className="gap-1 text-muted-foreground border-muted-foreground/30"
				>
					<AlertCircle className="h-3 w-3" />
					{t('cancelled', 'Cancelled')}
				</Badge>
			);
	}
}

function ResponseDisplay({
	interactionType,
	responseValue,
}: {
	interactionType: IInteractionType;
	responseValue: any;
}) {
	const { t } = useTranslation("chat");
	if (!responseValue) {
		return (
			<p className="text-sm text-muted-foreground flex items-center gap-1.5">
				<CheckCircle2 className="h-4 w-4 text-emerald-500" />
				{t('responseSubmitted', 'Response submitted.')}
			</p>
		);
	}

	switch (interactionType.type) {
		case "single_choice": {
			const selectedId = responseValue.selected_id;
			const selectedOption = interactionType.options.find(
				(o) => o.id === selectedId,
			);
			const freeformValue = responseValue.freeform_value;

			return (
				<div className="text-sm space-y-1">
					<p className="text-muted-foreground flex items-center gap-1.5">
						<CheckCircle2 className="h-4 w-4 text-emerald-500" />
						{t('responseSubmitted2', 'Response submitted:')}
					</p>
					<p className="font-medium pl-5">
						{selectedOption?.label ?? selectedId}
						{freeformValue && (
							<span className="font-normal text-muted-foreground">{`: ${freeformValue}`}</span>
						)}
					</p>
				</div>
			);
		}
		case "multiple_choice": {
			const selectedIds: string[] = responseValue.selected_ids ?? [];
			const selectedLabels = selectedIds
				.map(
					(id) => interactionType.options.find((o) => o.id === id)?.label ?? id,
				)
				.join(", ");

			return (
				<div className="text-sm space-y-1">
					<p className="text-muted-foreground flex items-center gap-1.5">
						<CheckCircle2 className="h-4 w-4 text-emerald-500" />
						{t('responseSubmitted2', 'Response submitted:')}
					</p>
					<p className="font-medium pl-5">
						{selectedLabels || t('noneSelected', 'None selected')}
					</p>
				</div>
			);
		}
		case "form": {
			const fields = interactionType.fields ?? [];
			const entries = Object.entries(responseValue).filter(
				([, v]) => v !== undefined && v !== "",
			);

			if (entries.length === 0) {
				return (
					<p className="text-sm text-muted-foreground flex items-center gap-1.5">
						<CheckCircle2 className="h-4 w-4 text-emerald-500" />
						{t('responseSubmitted', 'Response submitted.')}
					</p>
				);
			}

			return (
				<div className="text-sm space-y-1">
					<p className="text-muted-foreground flex items-center gap-1.5">
						<CheckCircle2 className="h-4 w-4 text-emerald-500" />
						{t('responseSubmitted2', 'Response submitted:')}
					</p>
					<div className="pl-5 space-y-0.5">
						{entries.map(([fieldId, value]) => {
							const field = fields.find((f) => f.id === fieldId);
							const label = field?.label ?? fieldId;
							const displayValue =
								typeof value === "boolean"
									? value
										? "Yes"
										: "No"
									: String(value);
							return (
								<p key={fieldId}>
									<span className="text-muted-foreground">{`${label}:`}</span>{" "}
									<span className="font-medium">{displayValue}</span>
								</p>
							);
						})}
					</div>
				</div>
			);
		}
	}
}

function SingleChoiceForm({
	config,
	onSubmit,
	isDisabled,
}: {
	config: ISingleChoiceInteraction;
	onSubmit: (value: any) => void;
	isDisabled: boolean;
}) {
	const { t } = useTranslation("chat");
	const [selected, setSelected] = useState<string>("");
	const [freeformValue, setFreeformValue] = useState("");

	const handleSubmit = useCallback(() => {
		if (!selected) return;
		const freeformOption = config.options.find((o) => o.freeform);
		if (selected === freeformOption?.id && freeformValue.trim()) {
			onSubmit({ selected_id: selected, freeform_value: freeformValue.trim() });
		} else {
			onSubmit({ selected_id: selected });
		}
	}, [selected, freeformValue, config.options, onSubmit]);

	const activeFreeform = useMemo(() => {
		if (!config.allow_freeform) return null;
		return config.options.find((o) => o.freeform) ?? null;
	}, [config]);

	return (
		<div className="space-y-3">
			<RadioGroup
				value={selected}
				onValueChange={setSelected}
				disabled={isDisabled}
			>
				{config.options.map((option) => (
					<div key={option.id} className="flex items-start gap-2">
						<RadioGroupItem
							value={option.id}
							id={`opt-${option.id}`}
							className="mt-0.5"
						/>
						<div className="grid gap-0.5">
							<Label
								htmlFor={`opt-${option.id}`}
								className="font-normal cursor-pointer"
							>
								{option.label}
							</Label>
							{option.description && (
								<span className="text-xs text-muted-foreground">
									{option.description}
								</span>
							)}
						</div>
					</div>
				))}
			</RadioGroup>

			{activeFreeform && selected === activeFreeform.id && (
				<Input
					placeholder={t('enterYourAnswer', 'Enter your answer...')}
					value={freeformValue}
					onChange={(e) => setFreeformValue(e.target.value)}
					disabled={isDisabled}
					className="mt-2"
				/>
			)}

			<Button
				size="sm"
				onClick={handleSubmit}
				disabled={isDisabled || !selected}
			>
				{t('submit', 'Submit')}
			</Button>
		</div>
	);
}

function MultipleChoiceForm({
	config,
	onSubmit,
	isDisabled,
}: {
	config: IMultipleChoiceInteraction;
	onSubmit: (value: any) => void;
	isDisabled: boolean;
}) {
	const { t } = useTranslation("chat");
	const [selected, setSelected] = useState<Set<string>>(new Set());

	const toggle = useCallback((id: string) => {
		setSelected((prev) => {
			const next = new Set(prev);
			if (next.has(id)) {
				next.delete(id);
			} else {
				next.add(id);
			}
			return next;
		});
	}, []);

	const validationError = useMemo(() => {
		const count = selected.size;
		if (config.min_selections && count < config.min_selections) {
			return t('selectAtLeastMin_selections', 'Select at least {{min_selections}}', { min_selections: config.min_selections });
		}
		if (config.max_selections && count > config.max_selections) {
			return t('selectAtMostMax_selections', 'Select at most {{max_selections}}', { max_selections: config.max_selections });
		}
		return null;
	}, [selected, config.min_selections, config.max_selections]);

	const handleSubmit = useCallback(() => {
		if (selected.size === 0 || validationError) return;
		onSubmit({ selected_ids: Array.from(selected) });
	}, [selected, validationError, onSubmit]);

	return (
		<div className="space-y-3">
			{config.options.map((option) => (
				<div key={option.id} className="flex items-start gap-2">
					<Checkbox
						id={`chk-${option.id}`}
						checked={selected.has(option.id)}
						onCheckedChange={() => toggle(option.id)}
						disabled={isDisabled}
						className="mt-0.5"
					/>
					<div className="grid gap-0.5">
						<Label
							htmlFor={`chk-${option.id}`}
							className="font-normal cursor-pointer"
						>
							{option.label}
						</Label>
						{option.description && (
							<span className="text-xs text-muted-foreground">
								{option.description}
							</span>
						)}
					</div>
				</div>
			))}

			{(config.min_selections || config.max_selections) && (
				<p className="text-xs text-muted-foreground">
					{config.min_selections && config.max_selections
						? `Select ${config.min_selections}–${config.max_selections} options`
						: config.min_selections
							? t('selectAtLeastMin_selections', 'Select at least {{min_selections}}', { min_selections: config.min_selections })
							: t('selectAtMostMax_selections', 'Select at most {{max_selections}}', { max_selections: config.max_selections })}
				</p>
			)}

			{validationError && selected.size > 0 && (
				<p className="text-xs text-destructive">{validationError}</p>
			)}

			<Button
				size="sm"
				onClick={handleSubmit}
				disabled={isDisabled || selected.size === 0 || !!validationError}
			>
				{t('submit', 'Submit')}
			</Button>
		</div>
	);
}

function FormFieldInput({
	field,
	value,
	onChange,
	isDisabled,
}: {
	field: IFormField;
	value: any;
	onChange: (value: any) => void;
	isDisabled: boolean;
}) {
	const { t } = useTranslation("chat");
	switch (field.field_type) {
		case "text":
			return (
				<Input
					id={`field-${field.id}`}
					value={value ?? ""}
					onChange={(e) => onChange(e.target.value)}
					disabled={isDisabled}
					placeholder={field.description}
				/>
			);
		case "number":
			return (
				<Input
					id={`field-${field.id}`}
					type="number"
					value={value ?? ""}
					onChange={(e) => onChange(e.target.valueAsNumber || e.target.value)}
					disabled={isDisabled}
					placeholder={field.description}
				/>
			);
		case "boolean":
			return (
				<Switch
					id={`field-${field.id}`}
					checked={!!value}
					onCheckedChange={onChange}
					disabled={isDisabled}
				/>
			);
		case "select":
			return (
				<Select
					value={value ?? ""}
					onValueChange={onChange}
					disabled={isDisabled}
				>
					<SelectTrigger id={`field-${field.id}`}>
						<SelectValue placeholder={t('selectAnOption', 'Select an option...')} />
					</SelectTrigger>
					<SelectContent>
						{field.options?.map((opt) => (
							<SelectItem key={opt.id} value={opt.id}>
								{opt.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			);
		default:
			return null;
	}
}

interface ISchemaProperty {
	type?: string;
	format?: string;
	description?: string;
	title?: string;
	enum?: unknown[];
	default?: unknown;
	properties?: Record<string, ISchemaProperty>;
	required?: string[];
	items?: ISchemaProperty;
}

interface IJsonSchema {
	type?: string;
	properties?: Record<string, ISchemaProperty>;
	required?: string[];
}

function asSchema(config: IFormInteraction): IJsonSchema | null {
	if (!config.schema || typeof config.schema !== "object") {
		return null;
	}

	return config.schema as IJsonSchema;
}

function getInitialValuesFromSchema(schema: IJsonSchema): Record<string, any> {
	const initial: Record<string, any> = {};
	for (const [name, property] of Object.entries(schema.properties ?? {})) {
		if (property.default !== undefined) {
			initial[name] = property.default;
			continue;
		}

		switch (property.type) {
			case "boolean":
				initial[name] = false;
				break;
			case "integer":
			case "number":
				initial[name] = "";
				break;
			case "object":
				initial[name] = getInitialValuesFromSchema({
					type: "object",
					properties: property.properties,
					required: property.required,
				});
				break;
			case "array":
				initial[name] = [];
				break;
			default:
				initial[name] = "";
		}
	}

	return initial;
}

function normalizeDateInput(value: string, format?: string): string {
	if (!value) {
		return "";
	}

	if (format === "date") {
		return value;
	}

	const date = new Date(value);
	if (Number.isNaN(date.getTime())) {
		return value;
	}

	return date.toISOString();
}

function getNestedValue(input: Record<string, any>, path: string[]): any {
	let current: any = input;
	for (const segment of path) {
		if (current == null || typeof current !== "object") {
			return undefined;
		}
		current = current[segment];
	}
	return current;
}

function setNestedValue(
	input: Record<string, any>,
	path: string[],
	value: any,
): Record<string, any> {
	if (path.length === 0) {
		return input;
	}

	const [head, ...tail] = path;
	if (tail.length === 0) {
		return { ...input, [head]: value };
	}

	const current = input[head];
	const next =
		current && typeof current === "object" && !Array.isArray(current)
			? current
			: {};

	return {
		...input,
		[head]: setNestedValue(next, tail, value),
	};
}

function renderSchemaField({
	path,
	property,
	value,
	onChange,
	isDisabled,
}: {
	path: string[];
	property: ISchemaProperty;
	value: any;
	onChange: (path: string[], value: any) => void;
	isDisabled: boolean;
}) {
	const id = `schema-${path.join("-")}`;
	const label = property.title ?? path[path.length - 1] ?? "field";
	const description = property.description;
	const required = false;

	if (property.enum && property.enum.length > 0) {
		return (
			<div key={id} className="space-y-1.5">
				<Label htmlFor={id} className="text-sm">
					{label}
					{required && <span className="text-destructive ml-0.5">*</span>}
				</Label>
				<Select
					value={value == null ? "" : String(value)}
					onValueChange={(selected) => onChange(path, selected)}
					disabled={isDisabled}
				>
					<SelectTrigger id={id}>
						<SelectValue placeholder={i18next.t('selectAnOption', 'Select an option...')} />
					</SelectTrigger>
					<SelectContent>
						{property.enum.map((option) => {
							const optionValue = String(option);
							return (
								<SelectItem key={optionValue} value={optionValue}>
									{optionValue}
								</SelectItem>
							);
						})}
					</SelectContent>
				</Select>
				{description && (
					<p className="text-xs text-muted-foreground">{description}</p>
				)}
			</div>
		);
	}

	if (property.type === "boolean") {
		return (
			<div key={id} className="space-y-1.5">
				<div className="flex items-center gap-3">
					<Switch
						id={id}
						checked={Boolean(value)}
						onCheckedChange={(checked) => onChange(path, checked)}
						disabled={isDisabled}
					/>
					<Label htmlFor={id} className="text-sm cursor-pointer">
						{label}
					</Label>
				</div>
				{description && (
					<p className="text-xs text-muted-foreground">{description}</p>
				)}
			</div>
		);
	}

	if (property.type === "integer" || property.type === "number") {
		return (
			<div key={id} className="space-y-1.5">
				<Label htmlFor={id} className="text-sm">
					{label}
					{required && <span className="text-destructive ml-0.5">*</span>}
				</Label>
				<Input
					id={id}
					type="number"
					value={value ?? ""}
					onChange={(event) => {
						const next = event.target.value;
						if (next === "") {
							onChange(path, "");
							return;
						}
						onChange(
							path,
							property.type === "integer"
								? Number.parseInt(next, 10)
								: Number.parseFloat(next),
						);
					}}
					disabled={isDisabled}
					placeholder={description}
				/>
			</div>
		);
	}

	if (property.type === "object" && property.properties) {
		return (
			<div key={id} className="space-y-2 rounded-md border p-3">
				<div>
					<Label className="text-sm font-medium">{label}</Label>
					{description && (
						<p className="text-xs text-muted-foreground">{description}</p>
					)}
				</div>
				<div className="space-y-2">
					{Object.entries(property.properties).map(
						([childName, childProperty]) =>
							renderSchemaField({
								path: [...path, childName],
								property: childProperty,
								value: getNestedValue(value ?? {}, [childName]),
								onChange,
								isDisabled,
							}),
					)}
				</div>
			</div>
		);
	}

	if (property.type === "array") {
		const items = property.items;
		const arrayValue = Array.isArray(value) ? value : [];

		// Structured array with object items - render proper nested UI
		if (items?.type === "object" && items.properties) {
			const addItem = () => {
				const newItem: Record<string, unknown> = {};
				for (const [propName, propSchema] of Object.entries(
					items.properties ?? {},
				)) {
					if (propSchema.default !== undefined) {
						newItem[propName] = propSchema.default;
					} else if (propSchema.type === "boolean") {
						newItem[propName] = false;
					} else if (propSchema.type === "array") {
						newItem[propName] = [];
					} else if (propSchema.type === "object") {
						newItem[propName] = {};
					} else {
						newItem[propName] = "";
					}
				}
				onChange(path, [...arrayValue, newItem]);
			};

			const removeItem = (index: number) => {
				onChange(
					path,
					arrayValue.filter((_, i) => i !== index),
				);
			};

			const updateItem = (
				index: number,
				itemPath: string[],
				itemValue: unknown,
			) => {
				const updated = arrayValue.map((item, i) => {
					if (i !== index) return item;
					return setNestedValue(item ?? {}, itemPath, itemValue);
				});
				onChange(path, updated);
			};

			return (
				<div key={id} className="space-y-2 rounded-md border p-3">
					<div className="flex items-center justify-between">
						<div>
							<Label className="text-sm font-medium">{label}</Label>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
						{!isDisabled && (
							<Button
								type="button"
								variant="outline"
								size="sm"
								onClick={addItem}
							>
								<Plus className="h-3 w-3 mr-1" /> {i18next.t('add', 'Add')}
							</Button>
						)}
					</div>
					{arrayValue.length === 0 && (
						<p className="text-xs text-muted-foreground italic">{i18next.t('noItems', 'No items')}</p>
					)}
					{arrayValue.map((item, index) => (
						<div
							key={index}
							className="space-y-2 rounded-md border bg-muted/20 p-3"
						>
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium text-muted-foreground">
									{i18next.t('item', 'Item')} {index + 1}
								</span>
								{!isDisabled && (
									<Button
										type="button"
										variant="ghost"
										size="sm"
										onClick={() => removeItem(index)}
										className="h-6 w-6 p-0 text-destructive hover:text-destructive"
									>
										<Trash2 className="h-3 w-3" />
									</Button>
								)}
							</div>
							<div className="space-y-2">
								{Object.entries(items.properties ?? {}).map(
									([propName, propSchema]) =>
										renderSchemaField({
											path: [propName],
											property: propSchema,
											value: getNestedValue(item ?? {}, [propName]),
											onChange: (itemPath, itemValue) =>
												updateItem(index, itemPath, itemValue),
											isDisabled,
										}),
								)}
							</div>
						</div>
					))}
				</div>
			);
		}

		// Simple primitive array - render add/remove list of inputs
		if (
			items &&
			["string", "integer", "number", "boolean"].includes(items.type ?? "")
		) {
			const addItem = () => {
				const defaultValue =
					items.type === "boolean"
						? false
						: items.type === "integer" || items.type === "number"
							? 0
							: "";
				onChange(path, [...arrayValue, defaultValue]);
			};

			const removeItem = (index: number) => {
				onChange(
					path,
					arrayValue.filter((_, i) => i !== index),
				);
			};

			const updateItem = (index: number, itemValue: unknown) => {
				const updated = arrayValue.map((item, i) =>
					i === index ? itemValue : item,
				);
				onChange(path, updated);
			};

			return (
				<div key={id} className="space-y-2">
					<div className="flex items-center justify-between">
						<div>
							<Label className="text-sm">{label}</Label>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
						{!isDisabled && (
							<Button
								type="button"
								variant="outline"
								size="sm"
								onClick={addItem}
							>
								<Plus className="h-3 w-3 mr-1" /> {i18next.t('add', 'Add')}
							</Button>
						)}
					</div>
					{arrayValue.length === 0 && (
						<p className="text-xs text-muted-foreground italic">{i18next.t('noItems', 'No items')}</p>
					)}
					{arrayValue.map((item, index) => (
						<div key={index} className="flex items-center gap-2">
							{items.type === "boolean" ? (
								<Switch
									checked={Boolean(item)}
									onCheckedChange={(checked) => updateItem(index, checked)}
									disabled={isDisabled}
								/>
							) : (
								<Input
									type={
										items.type === "integer" || items.type === "number"
											? "number"
											: "text"
									}
									value={item ?? ""}
									onChange={(e) => {
										const next = e.target.value;
										if (items.type === "integer") {
											updateItem(
												index,
												next === "" ? "" : Number.parseInt(next, 10),
											);
										} else if (items.type === "number") {
											updateItem(
												index,
												next === "" ? "" : Number.parseFloat(next),
											);
										} else {
											updateItem(index, next);
										}
									}}
									disabled={isDisabled}
									className="flex-1"
								/>
							)}
							{!isDisabled && (
								<Button
									type="button"
									variant="ghost"
									size="sm"
									onClick={() => removeItem(index)}
									className="h-8 w-8 p-0 text-destructive hover:text-destructive"
								>
									<Trash2 className="h-3 w-3" />
								</Button>
							)}
						</div>
					))}
				</div>
			);
		}

		// Fallback: JSON textarea for complex arrays without proper items schema
		return (
			<div key={id} className="space-y-1.5">
				<Label htmlFor={id} className="text-sm">
					{label}
					{required && <span className="text-destructive ml-0.5">*</span>}
				</Label>
				<Textarea
					id={id}
					value={Array.isArray(value) ? JSON.stringify(value, null, 2) : ""}
					onChange={(event) => {
						const next = event.target.value;
						if (!next.trim()) {
							onChange(path, []);
							return;
						}
						try {
							onChange(path, JSON.parse(next));
						} catch {
							// Keep invalid input local until user provides valid JSON
						}
					}}
					disabled={isDisabled}
					placeholder={description ?? i18next.t('enterJsonArray', 'Enter JSON array')}
					className="font-mono text-xs"
				/>
			</div>
		);
	}

	const isDate = property.type === "string" && property.format === "date";
	const isDateTime =
		property.type === "string" &&
		(property.format === "date-time" || property.format === "datetime");

	return (
		<div key={id} className="space-y-1.5">
			<Label htmlFor={id} className="text-sm">
				{label}
				{required && <span className="text-destructive ml-0.5">*</span>}
			</Label>
			<Input
				id={id}
				type={isDate ? "date" : isDateTime ? "datetime-local" : "text"}
				value={
					isDateTime && typeof value === "string" && value
						? new Date(value).toISOString().slice(0, 16)
						: (value ?? "")
				}
				onChange={(event) => {
					const rawValue = event.target.value;
					if (isDate || isDateTime) {
						onChange(path, normalizeDateInput(rawValue, property.format));
						return;
					}
					onChange(path, rawValue);
				}}
				disabled={isDisabled}
				placeholder={description}
			/>
			{description && (
				<p className="text-xs text-muted-foreground">{description}</p>
			)}
		</div>
	);
}

function FormInteractionForm({
	config,
	onSubmit,
	isDisabled,
}: {
	config: IFormInteraction;
	onSubmit: (value: any) => void;
	isDisabled: boolean;
}) {
	const { t } = useTranslation("chat");
	const schema = useMemo(() => asSchema(config), [config]);

	const [values, setValues] = useState<Record<string, any>>(() => {
		if (schema?.properties) {
			return getInitialValuesFromSchema(schema);
		}

		const initial: Record<string, any> = {};
		for (const field of config.fields ?? []) {
			if (field.default_value !== undefined) {
				initial[field.id] = field.default_value;
			}
		}
		return initial;
	});

	const setFieldValue = useCallback((fieldId: string, value: any) => {
		setValues((prev) => ({ ...prev, [fieldId]: value }));
	}, []);

	const setSchemaFieldValue = useCallback((path: string[], value: any) => {
		setValues((prev) => setNestedValue(prev, path, value));
	}, []);

	const missingRequired = useMemo(() => {
		if (schema?.properties) {
			const required = new Set(schema.required ?? []);
			return Object.keys(schema.properties)
				.filter((name) => required.has(name))
				.filter((name) => {
					const v = values[name];
					return v === undefined || v === null || v === "";
				})
				.map((name) => ({ label: name }));
		}

		return (config.fields ?? [])
			.filter((f) => f.required)
			.filter((f) => {
				const v = values[f.id];
				return v === undefined || v === null || v === "";
			});
	}, [config.fields, schema, values]);

	const handleSubmit = useCallback(() => {
		if (missingRequired.length > 0) return;
		onSubmit(values);
	}, [values, missingRequired, onSubmit]);

	if (schema?.properties) {
		return (
			<div className="space-y-4">
				{Object.entries(schema.properties).map(([name, property]) =>
					renderSchemaField({
						path: [name],
						property,
						value: values[name],
						onChange: setSchemaFieldValue,
						isDisabled,
					}),
				)}

				{missingRequired.length > 0 && Object.keys(values).length > 0 && (
					<p className="text-xs text-destructive">
						{t('fillInRequiredFields', 'Fill in required fields:')}{" "}
						{missingRequired.map((f) => f.label).join(", ")}
					</p>
				)}

				<Button
					size="sm"
					onClick={handleSubmit}
					disabled={isDisabled || missingRequired.length > 0}
				>
					{t('submit', 'Submit')}
				</Button>
			</div>
		);
	}

	return (
		<div className="space-y-4">
			{(config.fields ?? []).map((field) => (
				<div key={field.id} className="space-y-1.5">
					<Label htmlFor={`field-${field.id}`} className="text-sm">
						{field.label}
						{field.required && (
							<span className="text-destructive ml-0.5">*</span>
						)}
					</Label>
					{field.description &&
						field.field_type !== "text" &&
						field.field_type !== "number" && (
							<p className="text-xs text-muted-foreground">
								{field.description}
							</p>
						)}
					<FormFieldInput
						field={field}
						value={values[field.id]}
						onChange={(v) => setFieldValue(field.id, v)}
						isDisabled={isDisabled}
					/>
				</div>
			))}

			{missingRequired.length > 0 && Object.keys(values).length > 0 && (
				<p className="text-xs text-destructive">
					{t('fillInRequiredFields', 'Fill in required fields:')}{" "}
					{missingRequired.map((f) => f.label).join(", ")}
				</p>
			)}

			<Button
				size="sm"
				onClick={handleSubmit}
				disabled={isDisabled || missingRequired.length > 0}
			>
				{t('submit', 'Submit')}
			</Button>
		</div>
	);
}

function InteractionBody({
	interactionType,
	onSubmit,
	isDisabled,
}: {
	interactionType: IInteractionType;
	onSubmit: (value: any) => void;
	isDisabled: boolean;
}) {
	switch (interactionType.type) {
		case "single_choice":
			return (
				<SingleChoiceForm
					config={interactionType}
					onSubmit={onSubmit}
					isDisabled={isDisabled}
				/>
			);
		case "multiple_choice":
			return (
				<MultipleChoiceForm
					config={interactionType}
					onSubmit={onSubmit}
					isDisabled={isDisabled}
				/>
			);
		case "form":
			return (
				<FormInteractionForm
					config={interactionType}
					onSubmit={onSubmit}
					isDisabled={isDisabled}
				/>
			);
	}
}

function getCompactResponseSummary(interaction: IInteractionRequest): string {
	const { interaction_type, response_value } = interaction;
	if (!response_value) return "Responded";

	switch (interaction_type.type) {
		case "single_choice": {
			const option = interaction_type.options.find(
				(o) => o.id === response_value.selected_id,
			);
			const label = option?.label ?? response_value.selected_id;
			return response_value.freeform_value
				? `${label}: ${response_value.freeform_value}`
				: label;
		}
		case "multiple_choice": {
			const ids: string[] = response_value.selected_ids ?? [];
			return (
				ids
					.map(
						(id) =>
							interaction_type.options.find((o) => o.id === id)?.label ?? id,
					)
					.join(", ") || i18next.t('noneSelected', 'None selected')
			);
		}
		case "form": {
			const fields = interaction_type.fields ?? [];
			const entries = Object.entries(response_value).filter(
				([, v]) => v !== undefined && v !== "",
			);
			if (entries.length === 0) return "Responded";
			return entries
				.map(([fieldId, value]) => {
					const field = fields.find((f) => f.id === fieldId);
					const label = field?.label ?? fieldId;
					const display =
						typeof value === "boolean" ? (value ? "Yes" : "No") : String(value);
					return `${label}: ${display}`;
				})
				.join(", ");
		}
		default:
			return "Responded";
	}
}

function CompactInteraction({
	interaction,
	onExpand,
}: {
	interaction: IInteractionRequest;
	onExpand: () => void;
}) {
	const statusIcon =
		interaction.status === "responded" ? (
			<CheckCircle2 className="h-3 w-3 shrink-0 text-emerald-500" />
		) : interaction.status === "expired" ? (
			<XCircle className="h-3 w-3 shrink-0 text-red-500/70" />
		) : (
			<AlertCircle className="h-3 w-3 shrink-0 text-muted-foreground" />
		);

	const summary =
		interaction.status === "responded"
			? getCompactResponseSummary(interaction)
			: interaction.status === "expired"
				? "Expired"
				: "Cancelled";

	return (
		<button
			type="button"
			onClick={onExpand}
			className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors group min-w-0"
		>
			{statusIcon}
			<span className="font-medium text-foreground/80 truncate">
				{interaction.name}
			</span>
			<span className="truncate">{summary}</span>
			<ChevronRight className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity" />
		</button>
	);
}

export function InteractionGroup({
	interactions,
	onRespond,
}: {
	interactions: IInteractionRequest[];
	onRespond: (interactionId: string, value: any) => void;
}) {
	const [expandedId, setExpandedId] = useState<string | null>(null);

	if (interactions.length === 0) return null;

	const expanded = interactions.find((i) => i.id === expandedId);

	if (expanded) {
		return (
			<div className="w-full max-w-lg space-y-2">
				<Interaction
					interaction={expanded}
					onRespond={onRespond}
					onCollapse={() => setExpandedId(null)}
					forceExpanded
				/>
				{interactions.length > 1 && (
					<div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md bg-muted/30 px-3 py-1.5">
						{interactions
							.filter((i) => i.id !== expandedId)
							.map((i) => (
								<CompactInteraction
									key={i.id}
									interaction={i}
									onExpand={() => setExpandedId(i.id)}
								/>
							))}
					</div>
				)}
			</div>
		);
	}

	return (
		<div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md bg-muted/30 px-3 py-1.5 max-w-lg w-full">
			{interactions.map((i) => (
				<CompactInteraction
					key={i.id}
					interaction={i}
					onExpand={() => setExpandedId(i.id)}
				/>
			))}
		</div>
	);
}

export function Interaction({
	interaction,
	onRespond,
	disabled,
	onCollapse,
	forceExpanded,
}: InteractionProps & { onCollapse?: () => void; forceExpanded?: boolean }) {
	const { t } = useTranslation("chat");
	const remaining = useCountdown(interaction.expires_at);
	const isPending = interaction.status === "pending" && remaining > 0;
	const isDisabled = disabled || !isPending;
	const isSettled = !isPending && interaction.status !== "pending";
	const [expanded, setExpanded] = useState(false);

	const handleSubmit = useCallback(
		(value: any) => {
			if (isDisabled) return;
			onRespond(interaction.id, value);
		},
		[interaction.id, isDisabled, onRespond],
	);

	// Standalone condensed view (not in a group)
	if (isSettled && !expanded && !forceExpanded) {
		return (
			<div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md bg-muted/30 px-3 py-1.5 max-w-lg w-full">
				<CompactInteraction
					interaction={interaction}
					onExpand={() => setExpanded(true)}
				/>
			</div>
		);
	}

	return (
		<Card
			className={cn(
				"w-full max-w-lg border",
				isPending && "border-amber-500/40",
				interaction.status === "responded" &&
					"border-emerald-500/30 opacity-80",
				(interaction.status === "expired" ||
					interaction.status === "cancelled") &&
					"opacity-60",
			)}
		>
			<CardHeader className="pb-2 gap-1">
				<div className="flex items-center justify-between gap-2">
					<CardTitle className="text-sm font-semibold">
						{interaction.name}
					</CardTitle>
					<div className="flex items-center gap-2">
						{isPending && (
							<span
								className={cn(
									"flex items-center gap-1 text-xs tabular-nums",
									remaining <= 30 ? "text-red-500" : "text-muted-foreground",
								)}
							>
								<Clock className="h-3 w-3" />
								{formatCountdown(remaining)}
							</span>
						)}
						<StatusBadge
							status={
								remaining <= 0 && interaction.status === "pending"
									? "expired"
									: interaction.status
							}
						/>
						{isSettled && (
							<button
								type="button"
								onClick={() => {
									if (onCollapse) onCollapse();
									else setExpanded(false);
								}}
								className="text-muted-foreground hover:text-foreground transition-colors"
							>
								<ChevronUp className="h-4 w-4" />
							</button>
						)}
					</div>
				</div>
				{interaction.description && (
					<CardDescription className="text-xs">
						<TextEditor
							initialContent={interaction.description}
							isMarkdown={true}
							minimal={true}
						/>
					</CardDescription>
				)}
			</CardHeader>

			<CardContent className="pt-0">
				{interaction.status === "expired" ||
				(interaction.status === "pending" && remaining <= 0) ? (
					<p className="text-sm text-muted-foreground flex items-center gap-1.5">
						<XCircle className="h-4 w-4 text-red-500" />
						{t('thisInteractionHasExpired', 'This interaction has expired.')}
					</p>
				) : interaction.status === "cancelled" ? (
					<p className="text-sm text-muted-foreground flex items-center gap-1.5">
						<AlertCircle className="h-4 w-4" />
						{t('thisInteractionWasCancelled', 'This interaction was cancelled.')}
					</p>
				) : interaction.status === "responded" ? (
					<ResponseDisplay
						interactionType={interaction.interaction_type}
						responseValue={interaction.response_value}
					/>
				) : (
					<InteractionBody
						interactionType={interaction.interaction_type}
						onSubmit={handleSubmit}
						isDisabled={isDisabled}
					/>
				)}
			</CardContent>
		</Card>
	);
}
