"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	CheckCircle2,
	ChevronRight,
	ChevronUp,
	CircleDotIcon,
	ClipboardListIcon,
	Clock,
	ListChecksIcon,
	type LucideIcon,
	Plus,
	Trash2,
	XCircle,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";
import { cn } from "../../../lib";
import {
	fromDateTimeInputValue,
	parseDateValue,
	toDateTimeInputValue,
} from "../../../lib/date";
import type {
	IFormField,
	IFormInteraction,
	IInteractionRequest,
	IInteractionStatus,
	IInteractionType,
	IMultipleChoiceInteraction,
	ISingleChoiceInteraction,
} from "../../../lib/schema/interaction";
import { Button } from "../../ui/button";
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
import {
	RequestCard,
	RequestCardBody,
	RequestCardFooter,
	RequestCardHeader,
	RequestChoice,
	RequestMark,
	RequestPill,
	type RequestTone,
	requestButtonClass,
} from "./request-card";

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

const STATUS_TONES: Record<IInteractionStatus, RequestTone> = {
	pending: "primary",
	responded: "good",
	expired: "critical",
	cancelled: "muted",
};

const STATUS_ICONS: Record<IInteractionStatus, LucideIcon> = {
	pending: Clock,
	responded: CheckCircle2,
	expired: XCircle,
	cancelled: AlertCircle,
};

const TYPE_ICONS: Record<IInteractionType["type"], LucideIcon> = {
	single_choice: CircleDotIcon,
	multiple_choice: ListChecksIcon,
	form: ClipboardListIcon,
};

function useStatusLabel() {
	const { t } = useTranslation("chat");
	return (status: IInteractionStatus): string =>
		({
			pending: t("pending", "Pending"),
			responded: t("responded", "Responded"),
			expired: t("expired", "Expired"),
			cancelled: t("cancelled", "Cancelled"),
		})[status];
}

/** The submit row every interaction form ends in, with its hint or validation note on the left. */
function InteractionFooter({
	note,
	error,
	disabled,
	onSubmit,
}: Readonly<{
	note?: ReactNode;
	error?: ReactNode;
	disabled: boolean;
	onSubmit: () => void;
}>) {
	const { t } = useTranslation("chat");
	return (
		<RequestCardFooter
			start={
				error ? (
					<p className="text-xs text-destructive">{error}</p>
				) : (
					note && <p className="text-xs text-muted-foreground">{note}</p>
				)
			}
		>
			<Button
				size="sm"
				onClick={onSubmit}
				disabled={disabled}
				className={requestButtonClass}
			>
				{t("submit", "Submit")}
			</Button>
		</RequestCardFooter>
	);
}

function SubmittedLine({ withValue }: Readonly<{ withValue: boolean }>) {
	const { t } = useTranslation("chat");
	return (
		<p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
			<CheckCircle2 className="size-3.5 text-good" />
			{withValue
				? t("responseSubmitted2", "Response submitted:")
				: t("responseSubmitted", "Response submitted.")}
		</p>
	);
}

function ResponseDisplay({
	interactionType,
	responseValue,
}: {
	interactionType: IInteractionType;
	responseValue: any;
}) {
	const { t } = useTranslation("chat");
	if (!responseValue) return <SubmittedLine withValue={false} />;

	switch (interactionType.type) {
		case "single_choice": {
			const selectedId = responseValue.selected_id;
			const selectedOption = interactionType.options.find(
				(o) => o.id === selectedId,
			);
			const freeformValue = responseValue.freeform_value;

			return (
				<SubmittedAnswer>
					<p className="font-medium">
						{selectedOption?.label ?? selectedId}
						{freeformValue && (
							<span className="font-normal text-muted-foreground">{`: ${freeformValue}`}</span>
						)}
					</p>
				</SubmittedAnswer>
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
				<SubmittedAnswer>
					<p className="font-medium">
						{selectedLabels || t("noneSelected", "None selected")}
					</p>
				</SubmittedAnswer>
			);
		}
		case "form": {
			const fields = interactionType.fields ?? [];
			const entries = Object.entries(responseValue).filter(
				([, v]) => v !== undefined && v !== "",
			);

			if (entries.length === 0) return <SubmittedLine withValue={false} />;

			return (
				<SubmittedAnswer>
					<dl className="grid grid-cols-[minmax(0,auto)_minmax(0,1fr)] gap-x-3 gap-y-1">
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
								<div key={fieldId} className="contents">
									<dt className="truncate text-muted-foreground">{label}</dt>
									<dd className="wrap-break-word font-medium">
										{displayValue}
									</dd>
								</div>
							);
						})}
					</dl>
				</SubmittedAnswer>
			);
		}
	}
}

function SubmittedAnswer({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<div className="space-y-2">
			<SubmittedLine withValue />
			<div className="rounded-lg bg-muted/50 px-3 py-2.5 text-sm">
				{children}
			</div>
		</div>
	);
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
	const idPrefix = useId();

	return (
		<>
			<RequestCardBody>
				<RadioGroup
					value={selected}
					onValueChange={setSelected}
					disabled={isDisabled}
					className="gap-1.5"
				>
					{config.options.map((option) => {
						const id = `${idPrefix}-${option.id}`;
						return (
							<RequestChoice
								key={option.id}
								htmlFor={id}
								active={selected === option.id}
								disabled={isDisabled}
								control={
									<RadioGroupItem
										value={option.id}
										id={id}
										className="mt-0.5"
									/>
								}
								label={option.label}
								description={option.description}
							/>
						);
					})}
				</RadioGroup>

				{activeFreeform && selected === activeFreeform.id && (
					<Input
						autoFocus
						placeholder={t("enterYourAnswer", "Enter your answer...")}
						value={freeformValue}
						onChange={(e) => setFreeformValue(e.target.value)}
						disabled={isDisabled}
						className="rounded-lg"
					/>
				)}
			</RequestCardBody>
			<InteractionFooter
				onSubmit={handleSubmit}
				disabled={isDisabled || !selected}
			/>
		</>
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
			return t(
				"selectAtLeastMin_selections",
				"Select at least {{min_selections}}",
				{ min_selections: config.min_selections },
			);
		}
		if (config.max_selections && count > config.max_selections) {
			return t(
				"selectAtMostMax_selections",
				"Select at most {{max_selections}}",
				{ max_selections: config.max_selections },
			);
		}
		return null;
	}, [selected, config.min_selections, config.max_selections]);

	const handleSubmit = useCallback(() => {
		if (selected.size === 0 || validationError) return;
		onSubmit({ selected_ids: Array.from(selected) });
	}, [selected, validationError, onSubmit]);

	const idPrefix = useId();
	const limits =
		config.min_selections && config.max_selections
			? `Select ${config.min_selections}–${config.max_selections} options`
			: config.min_selections
				? t(
						"selectAtLeastMin_selections",
						"Select at least {{min_selections}}",
						{ min_selections: config.min_selections },
					)
				: config.max_selections
					? t(
							"selectAtMostMax_selections",
							"Select at most {{max_selections}}",
							{ max_selections: config.max_selections },
						)
					: undefined;

	return (
		<>
			<RequestCardBody className="space-y-1.5">
				{config.options.map((option) => {
					const id = `${idPrefix}-${option.id}`;
					const active = selected.has(option.id);
					return (
						<RequestChoice
							key={option.id}
							htmlFor={id}
							active={active}
							disabled={isDisabled}
							control={
								<Checkbox
									id={id}
									checked={active}
									onCheckedChange={() => toggle(option.id)}
									disabled={isDisabled}
									className="mt-0.5"
								/>
							}
							label={option.label}
							description={option.description}
						/>
					);
				})}
			</RequestCardBody>
			<InteractionFooter
				note={limits}
				error={selected.size > 0 ? validationError : undefined}
				onSubmit={handleSubmit}
				disabled={isDisabled || selected.size === 0 || !!validationError}
			/>
		</>
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
						<SelectValue
							placeholder={t("selectAnOption", "Select an option...")}
						/>
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

/**
 * A `datetime-local` input hands back wall-clock time in the viewer's zone, so
 * it is read through the local parser before being stored as an instant. Going
 * through new Date() worked only because the display side made the mirror-image
 * mistake, and the pair shifted the value by the offset on every round trip.
 */
function normalizeDateInput(value: string, format?: string): string {
	if (!value) {
		return "";
	}

	if (format === "date") {
		return value;
	}

	return fromDateTimeInputValue(value)?.toISOString() ?? value;
}

function dateTimeInputValue(value: string): string | undefined {
	const parsed = parseDateValue(value);
	return parsed ? toDateTimeInputValue(parsed, "minute") : undefined;
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
						<SelectValue
							placeholder={i18next.t("selectAnOption", "Select an option...")}
						/>
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
								<Plus className="h-3 w-3 mr-1" /> {i18next.t("add", "Add")}
							</Button>
						)}
					</div>
					{arrayValue.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{i18next.t("noItems", "No items")}
						</p>
					)}
					{arrayValue.map((item, index) => (
						<div
							key={index}
							className="space-y-2 rounded-md border bg-muted/20 p-3"
						>
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium text-muted-foreground">
									{i18next.t("item", "Item")} {index + 1}
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
								<Plus className="h-3 w-3 mr-1" /> {i18next.t("add", "Add")}
							</Button>
						)}
					</div>
					{arrayValue.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{i18next.t("noItems", "No items")}
						</p>
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
					placeholder={
						description ?? i18next.t("enterJsonArray", "Enter JSON array")
					}
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
						? (dateTimeInputValue(value) ?? value)
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

	const footer = (
		<InteractionFooter
			error={
				missingRequired.length > 0 &&
				Object.keys(values).length > 0 &&
				`${t("fillInRequiredFields", "Fill in required fields:")} ${missingRequired
					.map((f) => f.label)
					.join(", ")}`
			}
			onSubmit={handleSubmit}
			disabled={isDisabled || missingRequired.length > 0}
		/>
	);

	if (schema?.properties) {
		return (
			<>
				<RequestCardBody className="space-y-4">
					{Object.entries(schema.properties).map(([name, property]) =>
						renderSchemaField({
							path: [name],
							property,
							value: values[name],
							onChange: setSchemaFieldValue,
							isDisabled,
						}),
					)}
				</RequestCardBody>
				{footer}
			</>
		);
	}

	return (
		<>
			<RequestCardBody className="space-y-4">
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
			</RequestCardBody>
			{footer}
		</>
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
					.join(", ") || i18next.t("noneSelected", "None selected")
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

const COMPACT_ICON_TONES: Record<IInteractionStatus, string> = {
	pending: "text-muted-foreground",
	responded: "text-good",
	expired: "text-critical",
	cancelled: "text-muted-foreground",
};

const COMPACT_ROW =
	"flex w-full max-w-lg flex-wrap items-center gap-x-4 gap-y-1.5 rounded-lg border border-border/50 bg-muted/20 px-3 py-2";

function CompactInteraction({
	interaction,
	onExpand,
}: {
	interaction: IInteractionRequest;
	onExpand: () => void;
}) {
	const statusLabel = useStatusLabel();
	const StatusIcon = STATUS_ICONS[interaction.status];
	const summary =
		interaction.status === "responded"
			? getCompactResponseSummary(interaction)
			: statusLabel(interaction.status);

	return (
		<button
			type="button"
			onClick={onExpand}
			className="group flex min-w-0 items-center gap-2 rounded text-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40"
		>
			<StatusIcon
				className={cn(
					"size-3.5 shrink-0",
					COMPACT_ICON_TONES[interaction.status],
				)}
			/>
			<span className="truncate font-medium text-foreground/85">
				{interaction.name}
			</span>
			<span className="truncate">{summary}</span>
			<ChevronRight className="size-3 shrink-0 opacity-0 transition-opacity group-hover:opacity-100" />
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
					<div className={COMPACT_ROW}>
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
		<div className={COMPACT_ROW}>
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
	const { t } = useTranslation(["chat", "common"]);
	const statusLabel = useStatusLabel();
	const remaining = useCountdown(interaction.expires_at);
	const isPending = interaction.status === "pending" && remaining > 0;
	const isDisabled = disabled || !isPending;
	const isSettled = !isPending && interaction.status !== "pending";
	const [expanded, setExpanded] = useState(false);
	const status: IInteractionStatus =
		remaining <= 0 && interaction.status === "pending"
			? "expired"
			: interaction.status;
	const tone = STATUS_TONES[status];

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
			<div className={COMPACT_ROW}>
				<CompactInteraction
					interaction={interaction}
					onExpand={() => setExpanded(true)}
				/>
			</div>
		);
	}

	const collapse = t("common:collapse", "Collapse");

	return (
		<RequestCard
			tone={tone}
			live={isPending}
			aria-label={interaction.name}
			className="max-w-lg"
		>
			<RequestCardHeader
				mark={
					<RequestMark
						icon={
							isPending
								? TYPE_ICONS[interaction.interaction_type.type]
								: STATUS_ICONS[status]
						}
						tone={tone}
					/>
				}
				tone={tone}
				live={isPending}
				eyebrow={
					isPending
						? t("needsAnAnswer", "Needs an answer")
						: statusLabel(status)
				}
				title={interaction.name}
				description={
					interaction.description && (
						<TextEditor
							initialContent={interaction.description}
							isMarkdown={true}
							minimal={true}
						/>
					)
				}
				aside={
					isPending ? (
						<RequestPill
							icon={Clock}
							tone={remaining <= 30 ? "critical" : "muted"}
						>
							{formatCountdown(remaining)}
						</RequestPill>
					) : (
						isSettled && (
							<button
								type="button"
								aria-label={collapse}
								title={collapse}
								onClick={() => {
									if (onCollapse) onCollapse();
									else setExpanded(false);
								}}
								className="flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40"
							>
								<ChevronUp className="size-4" />
							</button>
						)
					)
				}
			/>

			{status === "expired" ? (
				<RequestCardBody>
					<p className="text-sm text-muted-foreground">
						{t("thisInteractionHasExpired", "This interaction has expired.")}
					</p>
				</RequestCardBody>
			) : status === "cancelled" ? (
				<RequestCardBody>
					<p className="text-sm text-muted-foreground">
						{t(
							"thisInteractionWasCancelled",
							"This interaction was cancelled.",
						)}
					</p>
				</RequestCardBody>
			) : status === "responded" ? (
				<RequestCardBody>
					<ResponseDisplay
						interactionType={interaction.interaction_type}
						responseValue={interaction.response_value}
					/>
				</RequestCardBody>
			) : (
				<InteractionBody
					interactionType={interaction.interaction_type}
					onSubmit={handleSubmit}
					isDisabled={isDisabled}
				/>
			)}
		</RequestCard>
	);
}
