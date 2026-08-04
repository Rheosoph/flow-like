"use client";

import type { JsonSchema } from "@flow-like/widget-sdk";
import { validateSchema } from "@flow-like/widget-sdk";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
	createJsonSchemaValue,
	homogeneousArrayItemSchema,
	isSchemaRecord,
	jsonSchemaType,
	resolveJsonSchema,
	summarizeWidgetListItem,
} from "../../lib/widget-schema-form";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";

export interface WidgetSchemaListEditorProps {
	fieldName: string;
	id: string;
	labelledBy: string;
	schema: JsonSchema;
	value: unknown[];
	disabled?: boolean;
	describedBy?: string;
	onChange: (value: unknown[]) => void;
}

interface ItemEditorState {
	mode: "create" | "edit";
	index: number | null;
	value: unknown;
	rawDraft: string | null;
	rawError: string | null;
}

interface SchemaValueEditorProps {
	id: string;
	schema: JsonSchema;
	value: unknown;
	disabled?: boolean;
	onChange: (value: unknown) => void;
}

function cloneJsonValue(value: unknown): unknown {
	if (value === undefined) return undefined;
	return JSON.parse(JSON.stringify(value));
}

function encodedOption(value: unknown): string {
	return JSON.stringify(value) ?? "null";
}

function optionLabel(value: unknown): string {
	if (value === "") return "Empty string";
	if (typeof value === "string") return value;
	return JSON.stringify(value) ?? String(value);
}

function schemaFormSupported(schema: JsonSchema, depth = 0): boolean {
	if (depth > 3) return false;
	const resolved = resolveJsonSchema(schema);
	if (Array.isArray(resolved.anyOf) || Array.isArray(resolved.oneOf)) {
		return false;
	}
	if (Object.hasOwn(resolved, "const")) return true;
	if (Array.isArray(resolved.enum) && resolved.enum.length > 0) return true;

	switch (jsonSchemaType(resolved)) {
		case "string":
		case "number":
		case "integer":
		case "boolean":
		case "null":
			return true;
		case "object": {
			if (!isSchemaRecord(resolved.properties)) return false;
			const properties = Object.values(resolved.properties);
			return (
				properties.length > 0 &&
				properties.every(
					(property) =>
						isSchemaRecord(property) &&
						schemaFormSupported(property, depth + 1),
				)
			);
		}
		default:
			return false;
	}
}

function errorText(error: string): string {
	return error.replace(/^\$(?:\.|:\s*)?/, "");
}

function SchemaValueEditor({
	id,
	schema,
	value,
	disabled,
	onChange,
}: SchemaValueEditorProps) {
	const resolved = resolveJsonSchema(schema);

	if (Object.hasOwn(resolved, "const")) {
		return (
			<Input
				id={id}
				value={optionLabel(resolved.const)}
				disabled
				className="h-9 font-mono text-xs"
			/>
		);
	}

	if (Array.isArray(resolved.enum) && resolved.enum.length > 0) {
		const options = [
			...new Map(
				resolved.enum.map((item) => [encodedOption(item), item]),
			).entries(),
		];
		return (
			<Select
				value={encodedOption(value)}
				onValueChange={(encoded) => onChange(JSON.parse(encoded))}
				disabled={disabled}
			>
				<SelectTrigger id={id} className="h-9">
					<SelectValue placeholder="Select a value…" />
				</SelectTrigger>
				<SelectContent>
					{options.map(([encoded, option]) => (
						<SelectItem key={encoded} value={encoded}>
							{optionLabel(option)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		);
	}

	switch (jsonSchemaType(resolved)) {
		case "boolean":
			return (
				<div className="flex h-9 items-center gap-2">
					<Switch
						id={id}
						checked={value === true}
						onCheckedChange={onChange}
						disabled={disabled}
					/>
					<span className="text-xs text-muted-foreground">
						{value === true ? "True" : "False"}
					</span>
				</div>
			);
		case "number":
		case "integer": {
			const integer = jsonSchemaType(resolved) === "integer";
			const minimum =
				typeof resolved.minimum === "number"
					? integer
						? Math.ceil(resolved.minimum)
						: resolved.minimum
					: undefined;
			const maximum =
				typeof resolved.maximum === "number"
					? integer
						? Math.floor(resolved.maximum)
						: resolved.maximum
					: undefined;
			return (
				<Input
					id={id}
					type="number"
					value={
						typeof value === "number" ? String(value) : String(value ?? "")
					}
					min={minimum}
					max={maximum}
					step={integer ? 1 : "any"}
					disabled={disabled}
					onChange={(event) => {
						const raw = event.target.value;
						if (raw === "") {
							onChange("");
							return;
						}
						const parsed = Number(raw);
						if (Number.isFinite(parsed)) onChange(parsed);
					}}
					className="h-9"
				/>
			);
		}
		case "object": {
			const object = isSchemaRecord(value) ? value : {};
			const properties = isSchemaRecord(resolved.properties)
				? resolved.properties
				: {};
			const required = new Set(
				Array.isArray(resolved.required)
					? resolved.required.filter(
							(candidate): candidate is string => typeof candidate === "string",
						)
					: [],
			);
			const updateProperty = (key: string, nextValue: unknown) => {
				onChange({ ...object, [key]: nextValue });
			};
			const omitProperty = (key: string) => {
				const next = { ...object };
				delete next[key];
				onChange(next);
			};

			return (
				<div className="space-y-4">
					{Object.entries(properties).map(([key, property]) => {
						if (!isSchemaRecord(property)) return null;
						const propertySchema = resolveJsonSchema(property);
						const propertyId = `${id}-${key}`;
						const present = Object.hasOwn(object, key);
						const isRequired = required.has(key);
						const validation = present
							? validateSchema(property, object[key])
							: {
									valid: !isRequired,
									errors: isRequired ? ["Value is required"] : [],
								};
						return (
							<div key={key} className="space-y-2">
								<div className="flex items-center justify-between gap-3">
									<Label htmlFor={propertyId} className="text-sm font-medium">
										{typeof propertySchema.title === "string"
											? propertySchema.title
											: key}
										{isRequired ? " *" : ""}
									</Label>
									{!isRequired && (
										<div className="flex items-center gap-2">
											<Switch
												id={`${propertyId}-included`}
												checked={present}
												disabled={disabled}
												onCheckedChange={(included) =>
													included
														? updateProperty(
																key,
																createJsonSchemaValue(propertySchema),
															)
														: omitProperty(key)
												}
											/>
											<Label
												htmlFor={`${propertyId}-included`}
												className="text-xs font-normal text-muted-foreground"
											>
												Include
											</Label>
										</div>
									)}
								</div>
								{typeof propertySchema.description === "string" && (
									<p className="text-xs text-muted-foreground">
										{propertySchema.description}
									</p>
								)}
								<SchemaValueEditor
									id={propertyId}
									schema={propertySchema}
									value={object[key]}
									disabled={disabled || (!isRequired && !present)}
									onChange={(nextValue) => updateProperty(key, nextValue)}
								/>
								{!validation.valid && (
									<p className="text-xs text-destructive">
										{errorText(validation.errors[0] ?? "Invalid value")}
									</p>
								)}
							</div>
						);
					})}
				</div>
			);
		}
		case "null":
			return <Input id={id} value="null" disabled className="h-9 font-mono" />;
		default:
			return (
				<Input
					id={id}
					value={typeof value === "string" ? value : String(value ?? "")}
					disabled={disabled}
					onChange={(event) => onChange(event.target.value)}
					className="h-9"
				/>
			);
	}
}

function rawItemText(value: unknown): string {
	return JSON.stringify(value, null, 2) ?? "null";
}

export function WidgetSchemaListEditor({
	fieldName,
	id,
	labelledBy,
	schema,
	value,
	disabled = false,
	describedBy,
	onChange,
}: WidgetSchemaListEditorProps) {
	const itemSchema = homogeneousArrayItemSchema(schema);
	const [editor, setEditor] = useState<ItemEditorState | null>(null);
	const resolvedArraySchema = resolveJsonSchema(schema);
	const minItems =
		typeof resolvedArraySchema.minItems === "number"
			? Math.max(0, Math.ceil(resolvedArraySchema.minItems))
			: 0;
	const maxItems =
		typeof resolvedArraySchema.maxItems === "number"
			? Math.max(0, Math.floor(resolvedArraySchema.maxItems))
			: undefined;
	const canAdd =
		!disabled &&
		itemSchema !== null &&
		(maxItems === undefined || value.length < maxItems);
	const canRemove = !disabled && value.length > minItems;
	const itemValidation = useMemo(
		() =>
			itemSchema && editor
				? validateSchema(itemSchema, editor.value)
				: { valid: false, errors: [] },
		[itemSchema, editor],
	);
	const canSave =
		editor !== null && editor.rawError === null && itemValidation.valid;

	const openCreate = () => {
		if (!itemSchema || !canAdd) return;
		const item = createJsonSchemaValue(itemSchema);
		const rawMode = !schemaFormSupported(itemSchema);
		setEditor({
			mode: "create",
			index: null,
			value: item,
			rawDraft: rawMode ? rawItemText(item) : null,
			rawError: null,
		});
	};

	const openEdit = (index: number) => {
		if (!itemSchema || disabled) return;
		const item = cloneJsonValue(value[index]);
		const rawMode = !schemaFormSupported(itemSchema);
		setEditor({
			mode: "edit",
			index,
			value: item,
			rawDraft: rawMode ? rawItemText(item) : null,
			rawError: null,
		});
	};

	const saveEditor = () => {
		if (!editor || !canSave) return;
		const next = [...value];
		if (editor.mode === "edit" && editor.index !== null) {
			if (editor.index >= next.length) return;
			next[editor.index] = editor.value;
		} else {
			next.push(editor.value);
		}
		onChange(next);
		setEditor(null);
	};

	const removeItem = (index: number) => {
		if (!canRemove) return;
		onChange(value.filter((_, itemIndex) => itemIndex !== index));
	};

	if (!itemSchema) return null;

	return (
		<>
			<fieldset
				id={id}
				className="min-w-0 overflow-hidden rounded-md border border-border/40"
				aria-labelledby={labelledBy}
				aria-describedby={describedBy}
			>
				<legend className="sr-only">{fieldName} items</legend>
				<div className="flex items-center justify-between gap-3 border-b border-border/30 bg-muted/20 px-3 py-2">
					<span className="text-xs text-muted-foreground">
						{value.length} {value.length === 1 ? "item" : "items"}
					</span>
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="h-7 gap-1.5"
						onClick={openCreate}
						disabled={!canAdd}
					>
						<Plus className="h-3.5 w-3.5" />
						Add item
					</Button>
				</div>

				{value.length === 0 ? (
					<p className="px-3 py-5 text-center text-xs text-muted-foreground">
						No items configured.
					</p>
				) : (
					<div className="max-h-52 divide-y divide-border/30 overflow-y-auto">
						{value.map((item, index) => {
							const summary = summarizeWidgetListItem(item, index);
							return (
								<div
									// biome-ignore lint/suspicious/noArrayIndexKey: contract list items have no guaranteed stable identifier
									key={index}
									className="flex items-center gap-3 px-3 py-2"
								>
									<span className="w-7 shrink-0 font-mono text-[10px] text-muted-foreground">
										#{index + 1}
									</span>
									<div className="min-w-0 flex-1">
										<p className="truncate text-sm font-medium">
											{summary.title}
										</p>
										{summary.detail && (
											<p className="truncate text-xs text-muted-foreground">
												{summary.detail}
											</p>
										)}
									</div>
									<div className="flex shrink-0 items-center gap-1">
										<Button
											type="button"
											variant="ghost"
											size="icon"
											className="h-7 w-7"
											onClick={() => openEdit(index)}
											disabled={disabled}
											aria-label={`Edit ${fieldName} item ${index + 1}`}
										>
											<Pencil className="h-3.5 w-3.5" />
										</Button>
										<Button
											type="button"
											variant="ghost"
											size="icon"
											className="h-7 w-7 text-muted-foreground hover:text-destructive"
											onClick={() => removeItem(index)}
											disabled={!canRemove}
											aria-label={`Remove ${fieldName} item ${index + 1}`}
										>
											<Trash2 className="h-3.5 w-3.5" />
										</Button>
									</div>
								</div>
							);
						})}
					</div>
				)}
			</fieldset>

			<Dialog
				open={editor !== null}
				onOpenChange={(open) => {
					if (!open) setEditor(null);
				}}
			>
				<DialogContent className="max-h-[85vh] overflow-hidden sm:max-w-2xl flex flex-col">
					<DialogHeader>
						<DialogTitle>
							{editor?.mode === "edit"
								? `Edit ${fieldName} item`
								: `Add item to ${fieldName}`}
						</DialogTitle>
						<DialogDescription>
							{typeof itemSchema.description === "string"
								? itemSchema.description
								: "Configure the item using its bundled type schema."}
						</DialogDescription>
					</DialogHeader>

					<DialogBody className="space-y-4 py-2">
						{editor && editor.rawDraft !== null ? (
							<div className="space-y-2">
								<Label htmlFor={`${id}-raw-item`}>Item JSON</Label>
								<Textarea
									id={`${id}-raw-item`}
									value={editor.rawDraft}
									onChange={(event) => {
										const rawDraft = event.target.value;
										setEditor((current) => {
											if (!current) return current;
											try {
												return {
													...current,
													rawDraft,
													rawError: null,
													value: JSON.parse(rawDraft),
												};
											} catch (error) {
												return {
													...current,
													rawDraft,
													rawError:
														error instanceof Error
															? error.message
															: String(error),
												};
											}
										});
									}}
									rows={10}
									spellCheck={false}
									className="font-mono text-xs"
								/>
								{editor.rawError && (
									<p className="text-xs text-destructive">
										Invalid JSON: {editor.rawError}
									</p>
								)}
							</div>
						) : editor ? (
							<>
								{jsonSchemaType(itemSchema) !== "object" && (
									<Label htmlFor={`${id}-item`} className="sr-only">
										Item value
									</Label>
								)}
								<SchemaValueEditor
									id={`${id}-item`}
									schema={itemSchema}
									value={editor.value}
									onChange={(nextValue) =>
										setEditor((current) =>
											current ? { ...current, value: nextValue } : current,
										)
									}
								/>
							</>
						) : null}

						{editor && !itemValidation.valid && (
							<div className="rounded-md border border-destructive/30 bg-destructive/5 p-3">
								<p className="text-xs font-medium text-destructive">
									Fix these values before saving:
								</p>
								<ul className="mt-1 list-disc space-y-0.5 pl-4 text-xs text-destructive">
									{[...new Set(itemValidation.errors)]
										.slice(0, 5)
										.map((error) => (
											<li key={error}>{errorText(error)}</li>
										))}
								</ul>
							</div>
						)}
					</DialogBody>

					<DialogFooter>
						<Button
							type="button"
							variant="outline"
							onClick={() => setEditor(null)}
						>
							Cancel
						</Button>
						<Button type="button" onClick={saveEditor} disabled={!canSave}>
							{editor?.mode === "edit" ? "Save changes" : "Add item"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
}
