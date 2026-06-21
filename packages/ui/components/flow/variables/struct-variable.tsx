"use client";

import { BracesIcon, FormInputIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../components/ui/button";
import { Checkbox } from "../../../components/ui/checkbox";
import { Input } from "../../../components/ui/input";
import { Label } from "../../../components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../components/ui/select";
import { Textarea } from "../../../components/ui/textarea";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../../components/ui/tooltip";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { cn } from "../../../lib/utils";

interface SchemaProperty {
	$ref?: string;
	allOf?: SchemaProperty[];
	anyOf?: SchemaProperty[];
	oneOf?: SchemaProperty[];
	type?: string | string[];
	title?: string;
	description?: string;
	default?: unknown;
	enum?: string[];
	items?: SchemaProperty;
	properties?: Record<string, SchemaProperty>;
	required?: string[];
}

interface JsonSchema extends SchemaProperty {
	$defs?: Record<string, SchemaProperty>;
	definitions?: Record<string, SchemaProperty>;
}

const EMPTY_STRING_HASH = "16248035215404677707";

const resolveRef = (
	value: string | undefined | null,
	refs: Record<string, string> | undefined,
): string => {
	if (!value) return "";
	if (value === EMPTY_STRING_HASH) return "";
	const resolved = refs?.[value];
	return resolved ?? value;
};

const parseSchema = (
	schemaStr: string | null | undefined,
	refs: Record<string, string> | undefined,
): JsonSchema | null => {
	if (!schemaStr) return null;
	const resolved = resolveRef(schemaStr, refs);
	if (!resolved) return null;
	try {
		return JSON.parse(resolved) as JsonSchema;
	} catch {
		return null;
	}
};

const unescapePointerSegment = (segment: string): string =>
	segment.replace(/~1/g, "/").replace(/~0/g, "~");

const resolvePointer = (
	root: JsonSchema,
	ref: string,
): SchemaProperty | JsonSchema | null => {
	if (ref === "#") return root;
	if (!ref.startsWith("#/")) return null;

	return ref
		.slice(2)
		.split("/")
		.map(unescapePointerSegment)
		.reduce<unknown>((current, segment) => {
			if (current && typeof current === "object" && segment in current) {
				return (current as Record<string, unknown>)[segment];
			}
			return null;
		}, root) as SchemaProperty | JsonSchema | null;
};

const schemaWithoutComposition = (schema: SchemaProperty): SchemaProperty => {
	const { $ref, anyOf, oneOf, allOf, ...rest } = schema;
	return rest;
};

const mergeSchemas = (
	base: SchemaProperty,
	override: SchemaProperty,
): SchemaProperty => {
	const merged: SchemaProperty = { ...base, ...override };

	if (base.properties || override.properties) {
		merged.properties = {
			...(base.properties ?? {}),
			...(override.properties ?? {}),
		};
	}

	if (base.required || override.required) {
		merged.required = Array.from(
			new Set([...(base.required ?? []), ...(override.required ?? [])]),
		);
	}

	return merged;
};

const rawSchemaType = (
	schema: SchemaProperty | JsonSchema,
): string | undefined => {
	const type = Array.isArray(schema.type)
		? schema.type.find((candidate) => candidate !== "null")
		: schema.type;

	if (type) return type;
	if (schema.properties) return "object";
	if (schema.items) return "array";
	return undefined;
};

const resolveSchema = (
	schema: SchemaProperty | JsonSchema,
	root: JsonSchema,
	seen = new Set<string>(),
): SchemaProperty => {
	if (schema.$ref) {
		if (seen.has(schema.$ref)) return schemaWithoutComposition(schema);
		const target = resolvePointer(root, schema.$ref);
		if (target) {
			const nextSeen = new Set(seen);
			nextSeen.add(schema.$ref);
			return mergeSchemas(
				resolveSchema(target, root, nextSeen),
				schemaWithoutComposition(schema),
			);
		}
	}

	const union = schema.anyOf ?? schema.oneOf;
	if (union && union.length > 0) {
		const branch =
			union.find((candidate) => {
				const type = rawSchemaType(resolveSchema(candidate, root, seen));
				return type && type !== "null";
			}) ?? union[0];
		return mergeSchemas(
			resolveSchema(branch, root, seen),
			schemaWithoutComposition(schema),
		);
	}

	if (schema.allOf && schema.allOf.length > 0) {
		return schema.allOf.reduce<SchemaProperty>(
			(merged, part) => mergeSchemas(merged, resolveSchema(part, root, seen)),
			schemaWithoutComposition(schema),
		);
	}

	return schemaWithoutComposition(schema);
};

const schemaType = (
	schema: SchemaProperty | JsonSchema,
	root: JsonSchema,
): string | undefined => {
	return rawSchemaType(resolveSchema(schema, root));
};

const defaultForSchema = (
	schema: SchemaProperty,
	root: JsonSchema,
	seenRefs = new Set<string>(),
): unknown => {
	const nextSeenRefs = new Set(seenRefs);
	if (schema.$ref) {
		if (nextSeenRefs.has(schema.$ref)) return undefined;
		nextSeenRefs.add(schema.$ref);
	}

	const resolved = resolveSchema(schema, root);
	const type = schemaType(resolved, root);

	if (resolved.default !== undefined) return resolved.default;
	if (type === "string") return "";
	if (type === "number" || type === "integer") return 0;
	if (type === "boolean") return false;
	if (type === "array") return [];
	if (type === "object") {
		const result: Record<string, unknown> = {};
		for (const [key, prop] of Object.entries(resolved.properties ?? {})) {
			result[key] = defaultForSchema(prop, root, nextSeenRefs);
		}
		return result;
	}

	return undefined;
};

const getDefaultFromSchema = (schema: JsonSchema): Record<string, unknown> => {
	const result: Record<string, unknown> = {};
	if (!schema.properties) return result;
	for (const [key, prop] of Object.entries(schema.properties)) {
		result[key] = defaultForSchema(prop, schema);
	}
	return result;
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

const valueAtPath = (
	value: Record<string, unknown>,
	path: string[],
): unknown => {
	return path.reduce<unknown>((current, key) => {
		if (!isRecord(current)) return undefined;
		return current[key];
	}, value);
};

const setValueAtPath = (
	value: Record<string, unknown>,
	path: string[],
	nextValue: unknown,
): Record<string, unknown> => {
	const [key, ...rest] = path;
	if (!key) return value;

	if (rest.length === 0) {
		return { ...value, [key]: nextValue };
	}

	return {
		...value,
		[key]: setValueAtPath(
			isRecord(value[key]) ? value[key] : {},
			rest,
			nextValue,
		),
	};
};

const formatJsonValue = (value: unknown): string => {
	if (value === undefined) return "";
	return JSON.stringify(value, null, 2) ?? "null";
};

const jsonValuesEqual = (left: unknown, right: unknown): boolean => {
	try {
		return JSON.stringify(left) === JSON.stringify(right);
	} catch {
		return left === right;
	}
};

const parseJsonDraft = (draft: string): unknown => {
	if (draft.trim() === "") return undefined;
	return JSON.parse(draft);
};

const uint8ArraysEqual = (
	left: number[] | null | undefined,
	right: number[] | null | undefined,
): boolean => {
	if (left === right) return true;
	const normalizedLeft = left ?? [];
	const normalizedRight = right ?? [];
	if (normalizedLeft.length !== normalizedRight.length) return false;
	return normalizedLeft.every(
		(value, index) => value === normalizedRight[index],
	);
};

function JsonValueTextarea({
	disabled,
	value,
	onValidChange,
	placeholder,
}: Readonly<{
	disabled?: boolean;
	value: unknown;
	onValidChange: (value: unknown) => void;
	placeholder?: string;
}>) {
	const [draft, setDraft] = useState(() => formatJsonValue(value));
	const [error, setError] = useState<string | null>(null);
	const previousValue = useRef(value);

	useEffect(() => {
		if (previousValue.current === value) return;
		previousValue.current = value;

		let parsedDraft: unknown;
		try {
			parsedDraft = parseJsonDraft(draft);
		} catch {
			parsedDraft = undefined;
		}

		if (jsonValuesEqual(parsedDraft, value)) return;
		setDraft(formatJsonValue(value));
		setError(null);
	}, [draft, value]);

	return (
		<div className="space-y-1">
			<Textarea
				disabled={disabled}
				className="font-mono text-xs h-20"
				value={draft}
				onChange={(e) => {
					const nextDraft = e.target.value;
					setDraft(nextDraft);
					try {
						const parsed = parseJsonDraft(nextDraft);
						setError(null);
						onValidChange(parsed);
					} catch {
						setError("Invalid JSON");
					}
				}}
				placeholder={placeholder}
			/>
			{error && <p className="text-xs text-destructive">{error}</p>}
		</div>
	);
}

export function StructVariable({
	disabled,
	variable,
	onChange,
	refs,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
	refs?: Record<string, string>;
}>) {
	const schema = useMemo(
		() => parseSchema(variable.schema, refs),
		[variable.schema, refs],
	);

	const formSchema = useMemo(
		() => (schema ? (resolveSchema(schema, schema) as JsonSchema) : null),
		[schema],
	);

	const hasSchema = formSchema !== null && formSchema.properties !== undefined;

	const [useJsonMode, setUseJsonMode] = useState(!hasSchema);
	const [jsonValue, setJsonValue] = useState<string>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		return typeof parsed === "object" ? JSON.stringify(parsed, null, 2) : "{}";
	});
	const [jsonError, setJsonError] = useState<string | null>(null);
	const [isFocused, setIsFocused] = useState(false);
	const defaultValueRef = useRef(variable.default_value);

	const [formValues, setFormValues] = useState<Record<string, unknown>>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		if (typeof parsed === "object" && parsed !== null) {
			return parsed as Record<string, unknown>;
		}
		if (formSchema?.properties) {
			return getDefaultFromSchema(formSchema);
		}
		return {};
	});

	useEffect(() => {
		defaultValueRef.current = variable.default_value;
	}, [variable.default_value]);

	// Re-initialize form values and mode when schema changes
	useEffect(() => {
		const parsed = parseUint8ArrayToJson(defaultValueRef.current);
		if (hasSchema && formSchema) {
			setUseJsonMode(false);
			const defaults = getDefaultFromSchema(formSchema);
			const nextValues =
				typeof parsed === "object" && parsed !== null
					? { ...defaults, ...parsed }
					: defaults;
			setFormValues((previous) =>
				jsonValuesEqual(previous, nextValues) ? previous : nextValues,
			);
		} else {
			setUseJsonMode(true);
			const nextValues =
				typeof parsed === "object" && parsed !== null
					? (parsed as Record<string, unknown>)
					: {};
			setFormValues((previous) =>
				jsonValuesEqual(previous, nextValues) ? previous : nextValues,
			);
		}
	}, [formSchema, hasSchema]);

	// Sync JSON value when switching modes
	useEffect(() => {
		if (useJsonMode) {
			setJsonValue(JSON.stringify(formValues, null, 2));
		}
	}, [formValues, useJsonMode]);

	// Update variable when form values change (non-JSON mode)
	useEffect(() => {
		if (useJsonMode) return;
		const defaultValue = convertJsonToUint8Array(formValues);
		if (uint8ArraysEqual(defaultValue, variable.default_value)) return;

		onChange({
			...variable,
			default_value: defaultValue,
		});
	}, [formValues, onChange, useJsonMode, variable]);

	const handleJsonChange = useCallback(
		(newJson: string) => {
			setJsonValue(newJson);
			try {
				const parsed = JSON.parse(newJson);
				setJsonError(null);
				onChange({
					...variable,
					default_value: convertJsonToUint8Array(parsed),
				});
			} catch (e) {
				setJsonError("Invalid JSON");
			}
		},
		[onChange, variable],
	);

	const handleFieldChange = useCallback((path: string[], value: unknown) => {
		setFormValues((prev) => setValueAtPath(prev, path, value));
	}, []);

	const renderSchemaField = (
		fieldPath: string[],
		prop: SchemaProperty,
		required: boolean,
		seenRefs = new Set<string>(),
	) => {
		if (!schema) return null;

		const fieldName = fieldPath[fieldPath.length - 1];
		const fieldId = `struct-${fieldPath.join("-")}`;
		const value = valueAtPath(formValues, fieldPath);
		const updateField = (nextValue: unknown) =>
			handleFieldChange(fieldPath, nextValue);
		const key = fieldPath.join(".");
		const label = `${fieldName}${required ? " *" : ""}`;
		const nextSeenRefs = new Set(seenRefs);

		if (prop.$ref) {
			if (nextSeenRefs.has(prop.$ref)) {
				return (
					<div key={key} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<JsonValueTextarea
							disabled={disabled}
							value={value}
							onValidChange={updateField}
							placeholder={`Enter ${fieldName} as JSON`}
						/>
						<p className="text-xs text-muted-foreground">
							Circular schema reference. Edit this value as JSON.
						</p>
					</div>
				);
			}
			nextSeenRefs.add(prop.$ref);
		}

		const resolvedProp = resolveSchema(prop, schema);
		const type = schemaType(resolvedProp, schema);
		const properties = resolvedProp.properties ?? {};
		const hasNestedProperties =
			type === "object" && Object.keys(properties).length > 0;

		if (resolvedProp.enum && resolvedProp.enum.length > 0) {
			return (
				<div key={key} className="space-y-1">
					<Label className="text-xs">{label}</Label>
					<Select
						disabled={disabled}
						value={String(value ?? "")}
						onValueChange={(v) => updateField(v)}
					>
						<SelectTrigger className="h-8">
							<SelectValue placeholder={`Select ${fieldName}`} />
						</SelectTrigger>
						<SelectContent>
							{resolvedProp.enum.map((option) => (
								<SelectItem key={option} value={option}>
									{option}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					{resolvedProp.description && (
						<p className="text-xs text-muted-foreground">
							{resolvedProp.description}
						</p>
					)}
				</div>
			);
		}

		if (hasNestedProperties) {
			return (
				<div key={fieldPath.join(".")} className="space-y-2">
					<div>
						<Label className="text-xs">{label}</Label>
						{resolvedProp.description && (
							<p className="text-xs text-muted-foreground">
								{resolvedProp.description}
							</p>
						)}
					</div>
					<div className="space-y-3 rounded-md border border-border/70 p-3">
						{Object.entries(properties).map(([childName, childProp]) =>
							renderSchemaField(
								[...fieldPath, childName],
								childProp,
								resolvedProp.required?.includes(childName) ?? false,
								nextSeenRefs,
							),
						)}
					</div>
				</div>
			);
		}

		switch (type) {
			case "boolean":
				return (
					<div key={key} className="flex items-center space-x-2 py-1">
						<Checkbox
							disabled={disabled}
							id={fieldId}
							checked={Boolean(value)}
							onCheckedChange={(checked) => updateField(checked === true)}
						/>
						<Label htmlFor={fieldId} className="text-xs cursor-pointer">
							{label}
						</Label>
						{resolvedProp.description && (
							<span className="text-xs text-muted-foreground ml-2">
								{resolvedProp.description}
							</span>
						)}
					</div>
				);

			case "integer":
				return (
					<div key={key} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<Input
							disabled={disabled}
							type="number"
							step="1"
							className="h-8"
							value={String(value ?? "")}
							onChange={(e) =>
								updateField(
									e.target.value ? Number.parseInt(e.target.value, 10) : "",
								)
							}
							placeholder={resolvedProp.description || `Enter ${fieldName}`}
						/>
					</div>
				);

			case "number":
				return (
					<div key={key} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<Input
							disabled={disabled}
							type="number"
							step="0.1"
							className="h-8"
							value={String(value ?? "")}
							onChange={(e) =>
								updateField(
									e.target.value ? Number.parseFloat(e.target.value) : "",
								)
							}
							placeholder={resolvedProp.description || `Enter ${fieldName}`}
						/>
					</div>
				);

			case "array":
			case "object":
				return (
					<div key={key} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<JsonValueTextarea
							disabled={disabled}
							value={value}
							onValidChange={updateField}
							placeholder={
								resolvedProp.description || `Enter ${fieldName} as JSON`
							}
						/>
					</div>
				);

			default:
				return (
					<div key={key} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<Input
							disabled={disabled}
							type="text"
							className="h-8"
							value={String(value ?? "")}
							onChange={(e) => updateField(e.target.value)}
							placeholder={resolvedProp.description || `Enter ${fieldName}`}
						/>
					</div>
				);
		}
	};

	return (
		<div className="grid w-full items-center gap-2">
			{hasSchema && (
				<div className="flex items-center justify-end gap-2">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={useJsonMode ? "outline" : "secondary"}
								size="sm"
								className="h-7 px-2 gap-1"
								onClick={() => {
									if (useJsonMode) {
										// Switching to form mode - parse JSON into form values
										try {
											const parsed = JSON.parse(jsonValue);
											setFormValues(parsed);
											setJsonError(null);
										} catch {
											// Keep current form values if JSON is invalid
										}
									}
									setUseJsonMode(false);
								}}
							>
								<FormInputIcon className="w-3 h-3" />
								<span className="text-xs">Form</span>
							</Button>
						</TooltipTrigger>
						<TooltipContent>Edit using generated form</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={useJsonMode ? "secondary" : "outline"}
								size="sm"
								className="h-7 px-2 gap-1"
								onClick={() => {
									setJsonValue(JSON.stringify(formValues, null, 2));
									setUseJsonMode(true);
								}}
							>
								<BracesIcon className="w-3 h-3" />
								<span className="text-xs">JSON</span>
							</Button>
						</TooltipTrigger>
						<TooltipContent>Edit raw JSON</TooltipContent>
					</Tooltip>
				</div>
			)}

			{useJsonMode || !hasSchema ? (
				<div className="space-y-1">
					<div
						className={cn(
							"relative w-full rounded-md border bg-transparent transition-all duration-200",
							"border-input dark:bg-input/30",
							isFocused && "border-ring ring-ring/50 ring-[3px]",
							jsonError && "border-destructive",
							disabled && "opacity-50 cursor-not-allowed",
						)}
					>
						<textarea
							disabled={disabled}
							value={jsonValue}
							onChange={(e) => handleJsonChange(e.target.value)}
							onFocus={() => setIsFocused(true)}
							onBlur={() => setIsFocused(false)}
							placeholder='{"key": "value"}'
							autoComplete="off"
							spellCheck="false"
							autoCorrect="off"
							autoCapitalize="off"
							rows={8}
							className={cn(
								"w-full resize-none bg-transparent px-3 py-2 text-sm outline-none",
								"font-mono leading-[22px]",
								"placeholder:text-muted-foreground",
							)}
						/>
					</div>
					{jsonError && <p className="text-xs text-destructive">{jsonError}</p>}
					{!hasSchema && (
						<p className="text-xs text-muted-foreground">
							No schema defined. Add a schema to enable form mode.
						</p>
					)}
				</div>
			) : (
				<div className="space-y-3 border rounded-md p-3">
					{formSchema.description && (
						<p className="text-xs text-muted-foreground mb-2">
							{formSchema.description}
						</p>
					)}
					{Object.entries(formSchema.properties || {}).map(
						([fieldName, prop]) =>
							renderSchemaField(
								[fieldName],
								prop,
								formSchema.required?.includes(fieldName) ?? false,
							),
					)}
				</div>
			)}
		</div>
	);
}
