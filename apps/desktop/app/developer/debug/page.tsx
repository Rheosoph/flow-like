"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	Accordion,
	AccordionContent,
	AccordionItem,
	AccordionTrigger,
	Badge,
	Button,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import type { IBit } from "@tm9657/flow-like-ui/lib/schema/bit/bit";
import { IBitTypes } from "@tm9657/flow-like-ui/lib/schema/bit/bit";
import type {
	PackageInspection,
	WasmExecutionResult,
	WasmNodeDefinition,
	WasmPinDefinition,
} from "@tm9657/flow-like-ui/lib/schema/developer";
import type { PackageManifest } from "@tm9657/flow-like-ui/lib/schema/wasm";
import { AnimatePresence, motion } from "framer-motion";
import {
	AlertCircle,
	AlertTriangle,
	ArrowLeft,
	Bug,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Clock,
	Code2,
	FileCode2,
	FolderOpen,
	Globe,
	HardDrive,
	Info,
	Loader2,
	Lock,
	Minus,
	Package,
	Play,
	Plus,
	Search,
	Shield,
	ShieldCheck,
	Sparkles,
	Zap,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	type LintIssue,
	type LintSeverity,
	countBySeverity,
	lintNodes,
} from "../../../lib/validate-nodes";

interface JsonSchema {
	type?: string;
	properties?: Record<string, JsonSchema>;
	required?: string[];
	items?: JsonSchema;
	title?: string;
	description?: string;
	default?: unknown;
	enum?: unknown[];
	format?: string;
	minimum?: number;
	maximum?: number;
	oneOf?: JsonSchema[];
	anyOf?: JsonSchema[];
	allOf?: JsonSchema[];
	$ref?: string;
	definitions?: Record<string, JsonSchema>;
}

const MODEL_BIT_TYPES = new Set<IBitTypes>([IBitTypes.Llm, IBitTypes.Vlm]);

function parseSchema(raw: string | undefined): JsonSchema | null {
	if (!raw) return null;
	try {
		const parsed = JSON.parse(raw) as JsonSchema;
		if (parsed && typeof parsed === "object") return parsed;
	} catch {
		/* malformed schema */
	}
	return null;
}

function getBitKey(bit: Pick<IBit, "id" | "hub">): string {
	return bit.hub ? `${bit.hub}:${bit.id}` : bit.id;
}

function getBitDisplayName(bit: IBit): string {
	return Object.values(bit.meta ?? {})[0]?.name ?? bit.id;
}

function getBitProviderName(bit: IBit): string | null {
	if (typeof bit.parameters !== "object" || bit.parameters == null) return null;
	const provider = (
		bit.parameters as {
			provider?: { provider_name?: string | null };
		}
	).provider?.provider_name;
	return typeof provider === "string" && provider.length > 0 ? provider : null;
}

function isSelectedBitValue(value: unknown): value is Pick<IBit, "id" | "hub"> {
	return (
		typeof value === "object" &&
		value !== null &&
		typeof (value as { id?: unknown }).id === "string"
	);
}

function isBitSchema(schema: JsonSchema | null): boolean {
	if (!schema) return false;

	const resolved = resolveSchema(schema, schema);
	if (resolved.type !== "object") return false;

	const properties = resolved.properties ?? {};
	const typeSchema = properties.type
		? resolveSchema(properties.type, schema)
		: undefined;
	const typeValues = (typeSchema?.enum ?? []).filter(
		(value): value is string => typeof value === "string",
	);

	return (
		"id" in properties &&
		"type" in properties &&
		(typeValues.some((value) => MODEL_BIT_TYPES.has(value as IBitTypes)) ||
			"parameters" in properties ||
			"meta" in properties ||
			"hub" in properties ||
			"hash" in properties ||
			"model_slug" in properties ||
			"model_evaluation" in properties)
	);
}

function isModelBitPin(pin: WasmPinDefinition): boolean {
	return pin.data_type === "Struct" && isBitSchema(parseSchema(pin.schema));
}

function applySpecialInputDefaults(
	node: WasmNodeDefinition,
	values: Record<string, unknown>,
	availableModelBits: IBit[],
): Record<string, unknown> {
	if (availableModelBits.length === 0) return values;

	let nextValues = values;
	for (const pin of node.pins) {
		if (
			pin.pin_type !== "Input" ||
			pin.data_type === "Execution" ||
			!isModelBitPin(pin) ||
			isSelectedBitValue(values[pin.name])
		) {
			continue;
		}

		if (nextValues === values) {
			nextValues = { ...values };
		}
		nextValues[pin.name] = availableModelBits[0];
	}

	return nextValues;
}

function buildInitialInputValues(
	node: WasmNodeDefinition,
	availableModelBits: IBit[],
): Record<string, unknown> {
	return applySpecialInputDefaults(
		node,
		initInputDefaults(node),
		availableModelBits,
	);
}

function getPermissionMeta(permission: string): {
	label: string;
	description: string;
	className: string;
	icon: typeof Globe;
} {
	switch (permission) {
		case "network:http":
			return {
				label: "HTTP",
				description: "Allows outbound HTTP requests during debug execution.",
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:websocket":
			return {
				label: "WebSocket",
				description: "Allows WebSocket connections from the node sandbox.",
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:tcp":
			return {
				label: "TCP",
				description: "Allows outbound TCP sockets for this node.",
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:udp":
			return {
				label: "UDP",
				description: "Allows outbound UDP sockets for this node.",
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:dns":
			return {
				label: "DNS",
				description: "Allows hostname resolution during execution.",
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "storage:read":
			return {
				label: "Storage Read",
				description:
					"Allows reading node or user storage through Flow-Like APIs.",
				className: "text-blue-600 border-blue-500/30",
				icon: HardDrive,
			};
		case "storage:write":
			return {
				label: "Storage Write",
				description:
					"Allows writing node or user storage through Flow-Like APIs.",
				className: "text-blue-600 border-blue-500/30",
				icon: HardDrive,
			};
		case "variables":
			return {
				label: "Variables",
				description: "Allows reading and writing flow variables.",
				className: "text-slate-600 border-slate-500/30",
				icon: HardDrive,
			};
		case "cache":
			return {
				label: "Cache",
				description: "Allows access to execution cache entries.",
				className: "text-slate-600 border-slate-500/30",
				icon: HardDrive,
			};
		case "streaming":
			return {
				label: "Streaming",
				description: "Allows incremental output streaming while the node runs.",
				className: "text-emerald-600 border-emerald-500/30",
				icon: Zap,
			};
		case "models":
			return {
				label: "Models",
				description:
					"Allows invoking LLM or VLM model providers from the host.",
				className: "text-fuchsia-600 border-fuchsia-500/30",
				icon: Sparkles,
			};
		case "a2ui":
			return {
				label: "A2UI",
				description: "Allows agent-to-UI rendering features.",
				className: "text-sky-600 border-sky-500/30",
				icon: Shield,
			};
		case "oauth":
			return {
				label: "OAuth",
				description: "Allows access to OAuth-backed credentials.",
				className: "text-orange-600 border-orange-500/30",
				icon: Lock,
			};
		case "functions":
			return {
				label: "Functions",
				description: "Allows invoking other functions or sub-flows.",
				className: "text-violet-600 border-violet-500/30",
				icon: Code2,
			};
		default:
			return {
				label: permission,
				description: "Custom node capability declared by the WASM node.",
				className: "text-muted-foreground border-border/30",
				icon: Shield,
			};
	}
}

function resolveRef(
	ref: string,
	rootSchema: JsonSchema,
): JsonSchema | undefined {
	const parts = ref.replace(/^#\//, "").split("/");
	let current: unknown = rootSchema;
	for (const part of parts) {
		if (current && typeof current === "object" && part in current) {
			current = (current as Record<string, unknown>)[part];
		} else {
			return undefined;
		}
	}
	return current as JsonSchema | undefined;
}

function resolveSchema(
	schema: JsonSchema,
	root: JsonSchema,
	seen = new Set<string>(),
): JsonSchema {
	if (schema.$ref) {
		if (seen.has(schema.$ref)) return schema;
		const nextSeen = new Set(seen).add(schema.$ref);
		const resolved = resolveRef(schema.$ref, root);
		if (resolved) return resolveSchema(resolved, root, nextSeen);
	}
	if (schema.allOf && schema.allOf.length > 0) {
		if (schema.allOf.length === 1) {
			return resolveSchema(schema.allOf[0], root, seen);
		}
		let merged: JsonSchema = {};
		for (const sub of schema.allOf) {
			const resolved = resolveSchema(sub, root, seen);
			merged = {
				...merged,
				...resolved,
				...(merged.properties || resolved.properties
					? { properties: { ...merged.properties, ...resolved.properties } }
					: {}),
			};
		}
		return merged;
	}
	if (schema.anyOf?.length === 1) {
		return resolveSchema(schema.anyOf[0], root, seen);
	}
	return schema;
}

function createDefaultFromSchema(
	schema: JsonSchema,
	root: JsonSchema,
	depth = 0,
): unknown {
	if (depth > 32) return null;
	const resolved = resolveSchema(schema, root);
	if (resolved.default !== undefined) return resolved.default;

	if (resolved.enum && resolved.enum.length > 0) return resolved.enum[0];

	switch (resolved.type) {
		case "string":
			return "";
		case "integer":
		case "number":
			return 0;
		case "boolean":
			return false;
		case "array":
			return [];
		case "object": {
			if (!resolved.properties) return {};
			const obj: Record<string, unknown> = {};
			for (const [key, propSchema] of Object.entries(resolved.properties)) {
				obj[key] = createDefaultFromSchema(propSchema, root, depth + 1);
			}
			return obj;
		}
		default:
			return null;
	}
}

function NodePermissionBadges({
	permissions,
	showEmpty = false,
}: {
	permissions: string[];
	showEmpty?: boolean;
}) {
	if (permissions.length === 0) {
		if (!showEmpty) return null;
		return (
			<Badge variant="outline" className="text-xs text-muted-foreground/70">
				No extra permissions
			</Badge>
		);
	}

	return (
		<div className="flex flex-wrap gap-1.5">
			{permissions.map((permission) => {
				const meta = getPermissionMeta(permission);
				const Icon = meta.icon;
				return (
					<Badge
						key={permission}
						variant="outline"
						className={cn("gap-1 text-xs", meta.className)}
					>
						<Icon className="h-3 w-3" />
						{meta.label}
					</Badge>
				);
			})}
		</div>
	);
}

function ModelBitInput({
	bits,
	value,
	onChange,
}: {
	bits: IBit[];
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	const selectedKey = isSelectedBitValue(value) ? getBitKey(value) : undefined;
	const selectedBit = bits.find((bit) => getBitKey(bit) === selectedKey);

	return (
		<div className="space-y-2">
			<Select
				value={selectedKey}
				onValueChange={(nextKey) => {
					const nextBit = bits.find((bit) => getBitKey(bit) === nextKey);
					onChange(nextBit ?? null);
				}}
				disabled={bits.length === 0}
			>
				<SelectTrigger className="h-9">
					<SelectValue placeholder="Select LLM/VLM bit..." />
				</SelectTrigger>
				<SelectContent>
					{bits.map((bit) => (
						<SelectItem key={getBitKey(bit)} value={getBitKey(bit)}>
							{getBitDisplayName(bit)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{selectedBit ? (
				<div className="rounded-lg border border-border/20 bg-muted/5 p-3 space-y-1.5">
					<div className="flex items-center justify-between gap-2">
						<span className="text-xs font-medium truncate">
							{getBitDisplayName(selectedBit)}
						</span>
						<Badge variant="outline" className="text-[10px]">
							{selectedBit.type}
						</Badge>
					</div>
					<p className="text-[11px] text-muted-foreground/60">
						{getBitProviderName(selectedBit) ??
							selectedBit.hub ??
							"Current profile"}
					</p>
				</div>
			) : (
				<p className="text-xs text-amber-600">
					No LLM or VLM bits are available in the current profile.
				</p>
			)}
		</div>
	);
}

function SchemaField({
	schema,
	rootSchema,
	value,
	onChange,
	label,
	required,
}: {
	schema: JsonSchema;
	rootSchema: JsonSchema;
	value: unknown;
	onChange: (val: unknown) => void;
	label?: string;
	required?: boolean;
}) {
	const resolved = resolveSchema(schema, rootSchema);

	if (resolved.oneOf || resolved.anyOf) {
		const variants = resolved.oneOf ?? resolved.anyOf ?? [];
		const nullVariant = variants.find((v) => v.type === "null");
		const nonNullVariants = variants.filter((v) => v.type !== "null");
		if (nullVariant && nonNullVariants.length === 1) {
			return (
				<SchemaField
					schema={nonNullVariants[0]}
					rootSchema={rootSchema}
					value={value}
					onChange={onChange}
					label={label}
					required={false}
				/>
			);
		}
	}

	if (resolved.enum && resolved.enum.length > 0) {
		return (
			<Select value={String(value ?? "")} onValueChange={(v) => onChange(v)}>
				<SelectTrigger className="h-9">
					<SelectValue placeholder="Select..." />
				</SelectTrigger>
				<SelectContent>
					{resolved.enum.map((v) => (
						<SelectItem key={String(v)} value={String(v)}>
							{String(v)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		);
	}

	switch (resolved.type) {
		case "boolean":
			return (
				<div className="flex items-center gap-2">
					<Switch
						checked={Boolean(value)}
						onCheckedChange={(v) => onChange(v)}
					/>
					<span className="text-sm text-muted-foreground/70">
						{value ? "true" : "false"}
					</span>
				</div>
			);

		case "integer":
			return (
				<Input
					type="number"
					step={1}
					min={resolved.minimum}
					max={resolved.maximum}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseInt(e.target.value) || 0)}
					className="h-9"
				/>
			);

		case "number":
			return (
				<Input
					type="number"
					step={0.01}
					min={resolved.minimum}
					max={resolved.maximum}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseFloat(e.target.value) || 0)}
					className="h-9"
				/>
			);

		case "array":
			return (
				<SchemaArrayField
					itemSchema={resolved.items ?? { type: "string" }}
					rootSchema={rootSchema}
					value={value}
					onChange={onChange}
				/>
			);

		case "object":
			if (resolved.properties) {
				return (
					<SchemaObjectFields
						schema={resolved}
						rootSchema={rootSchema}
						value={value}
						onChange={onChange}
					/>
				);
			}
			return (
				<Textarea
					value={
						typeof value === "string"
							? value
							: JSON.stringify(value ?? {}, null, 2)
					}
					onChange={(e) => {
						try {
							onChange(JSON.parse(e.target.value));
						} catch {
							onChange(e.target.value);
						}
					}}
					rows={3}
					className="font-mono text-xs"
					placeholder="{}"
				/>
			);

		case "string":
			if (resolved.format === "date-time") {
				return (
					<Input
						type="datetime-local"
						value={String(value ?? "")}
						onChange={(e) => onChange(e.target.value)}
						className="h-9"
					/>
				);
			}
			return (
				<Input
					value={String(value ?? "")}
					onChange={(e) => onChange(e.target.value)}
					className="h-9"
					placeholder={resolved.description ?? `Enter ${label ?? "value"}...`}
				/>
			);

		default:
			return (
				<Input
					value={String(value ?? "")}
					onChange={(e) => onChange(e.target.value)}
					className="h-9"
					placeholder={`Enter ${label ?? "value"}...`}
				/>
			);
	}
}

function SchemaObjectFields({
	schema,
	rootSchema,
	value,
	onChange,
}: {
	schema: JsonSchema;
	rootSchema: JsonSchema;
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	const obj = (
		value && typeof value === "object" && !Array.isArray(value) ? value : {}
	) as Record<string, unknown>;
	const properties = schema.properties ?? {};
	const requiredFields = new Set(schema.required ?? []);

	const setField = useCallback(
		(key: string, fieldValue: unknown) => {
			onChange({ ...obj, [key]: fieldValue });
		},
		[obj, onChange],
	);

	return (
		<div className="space-y-3 rounded-lg border border-border/20 bg-muted/5 p-3">
			{Object.entries(properties).map(([key, propSchema]) => {
				const resolved = resolveSchema(propSchema, rootSchema);
				const title = resolved.title ?? propSchema.title ?? key;
				return (
					<div key={key} className="space-y-1">
						<div className="flex items-center gap-1.5">
							<Label className="text-xs font-medium">{title}</Label>
							{requiredFields.has(key) && (
								<span className="text-[10px] text-destructive">*</span>
							)}
							<span className="text-[10px] text-muted-foreground/50 font-mono">
								{resolved.type ?? "any"}
							</span>
						</div>
						{resolved.description && (
							<p className="text-[11px] text-muted-foreground/50">
								{resolved.description}
							</p>
						)}
						<SchemaField
							schema={propSchema}
							rootSchema={rootSchema}
							value={obj[key]}
							onChange={(v) => setField(key, v)}
							label={title}
							required={requiredFields.has(key)}
						/>
					</div>
				);
			})}
		</div>
	);
}

function SchemaArrayField({
	itemSchema,
	rootSchema,
	value,
	onChange,
}: {
	itemSchema: JsonSchema;
	rootSchema: JsonSchema;
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	const items = Array.isArray(value) ? (value as unknown[]) : [];

	const addItem = useCallback(() => {
		const newItem = createDefaultFromSchema(itemSchema, rootSchema);
		onChange([...items, newItem]);
	}, [items, itemSchema, rootSchema, onChange]);

	const removeItem = useCallback(
		(index: number) => {
			onChange(items.filter((_, i) => i !== index));
		},
		[items, onChange],
	);

	const updateItem = useCallback(
		(index: number, val: unknown) => {
			const next = [...items];
			next[index] = val;
			onChange(next);
		},
		[items, onChange],
	);

	const resolvedItem = resolveSchema(itemSchema, rootSchema);
	const isComplex =
		resolvedItem.type === "object" || resolvedItem.type === "array";

	return (
		<div className="space-y-2">
			{items.length > 0 && (
				<Accordion
					type="multiple"
					defaultValue={items.map((_, i) => String(i))}
					className="space-y-1"
				>
					{items.map((item, i) => (
						<AccordionItem
							key={`item-${i}`}
							value={String(i)}
							className="border border-border/20 rounded-lg overflow-hidden"
						>
							<div className="flex items-center">
								<AccordionTrigger className="flex-1 px-3 py-2 text-xs hover:no-underline">
									<span className="font-mono text-muted-foreground/70">
										[{i}]
									</span>
								</AccordionTrigger>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7 mr-1 text-muted-foreground/50 hover:text-destructive"
									onClick={() => removeItem(i)}
								>
									<Minus className="h-3 w-3" />
								</Button>
							</div>
							<AccordionContent className="px-3 pb-3">
								{isComplex ? (
									<SchemaField
										schema={itemSchema}
										rootSchema={rootSchema}
										value={item}
										onChange={(v) => updateItem(i, v)}
									/>
								) : (
									<SchemaField
										schema={itemSchema}
										rootSchema={rootSchema}
										value={item}
										onChange={(v) => updateItem(i, v)}
									/>
								)}
							</AccordionContent>
						</AccordionItem>
					))}
				</Accordion>
			)}
			<Button
				variant="outline"
				size="sm"
				onClick={addItem}
				className="w-full h-8 text-xs gap-1.5 border-dashed"
			>
				<Plus className="h-3 w-3" />
				Add Item
			</Button>
		</div>
	);
}

function StructInput({
	pin,
	value,
	onChange,
}: {
	pin: WasmPinDefinition;
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	const [rawMode, setRawMode] = useState(false);
	const schema = useMemo(() => parseSchema(pin.schema), [pin.schema]);
	const isArray = pin.value_type === "Array";

	if (!schema || rawMode) {
		const effectiveValue = isArray ? (value ?? []) : (value ?? {});
		return (
			<div className="space-y-1.5">
				{schema && (
					<div className="flex justify-end">
						<Button
							variant="ghost"
							size="sm"
							onClick={() => setRawMode(false)}
							className="h-6 text-[10px] gap-1 px-2"
						>
							<ChevronRight className="h-3 w-3" />
							Form
						</Button>
					</div>
				)}
				<Textarea
					value={
						typeof value === "string"
							? value
							: JSON.stringify(effectiveValue, null, 2)
					}
					onChange={(e) => {
						try {
							onChange(JSON.parse(e.target.value));
						} catch {
							onChange(e.target.value);
						}
					}}
					rows={4}
					className="font-mono text-xs"
					placeholder={isArray ? "[]" : "{}"}
				/>
			</div>
		);
	}

	const rootSchema = schema;

	if (isArray) {
		const itemSchema: JsonSchema =
			schema.type === "object" ? schema : (schema.items ?? schema);
		return (
			<div className="space-y-1.5">
				<div className="flex justify-end">
					<Button
						variant="ghost"
						size="sm"
						onClick={() => setRawMode(true)}
						className="h-6 text-[10px] gap-1 px-2"
					>
						<Code2 className="h-3 w-3" />
						JSON
					</Button>
				</div>
				<SchemaArrayField
					itemSchema={itemSchema}
					rootSchema={rootSchema}
					value={value}
					onChange={onChange}
				/>
			</div>
		);
	}

	return (
		<div className="space-y-1.5">
			<div className="flex justify-end">
				<Button
					variant="ghost"
					size="sm"
					onClick={() => setRawMode(true)}
					className="h-6 text-[10px] gap-1 px-2"
				>
					<Code2 className="h-3 w-3" />
					JSON
				</Button>
			</div>
			<SchemaField
				schema={schema}
				rootSchema={rootSchema}
				value={value}
				onChange={onChange}
			/>
		</div>
	);
}

function PinInput({
	pin,
	value,
	onChange,
	availableModelBits,
}: {
	pin: WasmPinDefinition;
	value: unknown;
	onChange: (val: unknown) => void;
	availableModelBits: IBit[];
}) {
	if (isModelBitPin(pin)) {
		return (
			<ModelBitInput
				bits={availableModelBits}
				value={value}
				onChange={onChange}
			/>
		);
	}

	if (pin.valid_values && pin.valid_values.length > 0) {
		return (
			<Select value={String(value ?? "")} onValueChange={(v) => onChange(v)}>
				<SelectTrigger className="h-9">
					<SelectValue placeholder="Select..." />
				</SelectTrigger>
				<SelectContent>
					{pin.valid_values.map((v) => (
						<SelectItem key={v} value={v}>
							{v}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		);
	}

	switch (pin.data_type) {
		case "Boolean":
			return (
				<div className="flex items-center gap-2">
					<Switch
						checked={Boolean(value)}
						onCheckedChange={(v) => onChange(v)}
					/>
					<span className="text-sm text-muted-foreground/70">
						{value ? "true" : "false"}
					</span>
				</div>
			);
		case "Integer":
			return (
				<Input
					type="number"
					step={pin.range ? 1 : undefined}
					min={pin.range?.[0]}
					max={pin.range?.[1]}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseInt(e.target.value) || 0)}
					className="h-9"
				/>
			);
		case "Float":
			return (
				<Input
					type="number"
					step={0.01}
					min={pin.range?.[0]}
					max={pin.range?.[1]}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseFloat(e.target.value) || 0)}
					className="h-9"
				/>
			);
		case "Struct":
			return <StructInput pin={pin} value={value} onChange={onChange} />;
		default:
			return (
				<Input
					value={String(value ?? "")}
					onChange={(e) => onChange(e.target.value)}
					className="h-9"
					placeholder={`Enter ${pin.data_type.toLowerCase()} value...`}
				/>
			);
	}
}

function DataTypeBadge({ dataType }: { dataType: string }) {
	const colorMap: Record<string, string> = {
		String: "bg-green-500/10 text-green-700 dark:text-green-400",
		Integer: "bg-blue-500/10 text-blue-700 dark:text-blue-400",
		Float: "bg-cyan-500/10 text-cyan-700 dark:text-cyan-400",
		Boolean: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
		Struct: "bg-purple-500/10 text-purple-700 dark:text-purple-400",
		Execution: "bg-red-500/10 text-red-700 dark:text-red-400",
		Date: "bg-orange-500/10 text-orange-700 dark:text-orange-400",
		PathBuf: "bg-slate-500/10 text-slate-700 dark:text-slate-400",
		Byte: "bg-gray-500/10 text-gray-700 dark:text-gray-400",
		Generic: "bg-pink-500/10 text-pink-700 dark:text-pink-400",
	};

	return (
		<Badge
			variant="outline"
			className={cn("text-[10px] font-mono", colorMap[dataType])}
		>
			{dataType}
		</Badge>
	);
}

function OutputValue({ name, value }: { name: string; value: unknown }) {
	const formatted =
		typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);

	return (
		<div className="space-y-1">
			<Label className="text-xs font-medium text-muted-foreground/70">
				{name}
			</Label>
			<pre className="bg-muted/30 rounded-lg p-3 text-xs font-mono whitespace-pre-wrap break-all">
				{formatted}
			</pre>
		</div>
	);
}

function getDefaultValue(pin: WasmPinDefinition): unknown {
	if (pin.default_value !== undefined && pin.default_value !== null) {
		return pin.default_value;
	}
	if (pin.value_type === "Array") return [];
	switch (pin.data_type) {
		case "Boolean":
			return false;
		case "Integer":
			return 0;
		case "Float":
			return 0.0;
		case "Struct":
			return {};
		default:
			return "";
	}
}

function PermissionsBadges({ manifest }: { manifest: PackageManifest }) {
	const p = manifest.permissions;
	return (
		<div className="flex flex-wrap gap-1.5">
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<HardDrive className="h-3 w-3" />
						{p.memory}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>Memory tier</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<Clock className="h-3 w-3" />
						{p.timeout}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>Timeout tier</TooltipContent>
			</Tooltip>
			{p.network?.httpEnabled && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-amber-600 border-amber-500/30"
				>
					<Globe className="h-3 w-3" />
					HTTP
				</Badge>
			)}
			{(p.filesystem?.nodeStorage || p.filesystem?.userStorage) && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-blue-600 border-blue-500/30"
				>
					<HardDrive className="h-3 w-3" />
					Storage
				</Badge>
			)}
			{p.streaming && (
				<Badge variant="outline" className="gap-1 text-xs">
					<Zap className="h-3 w-3" />
					Streaming
				</Badge>
			)}
			{p.models && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-purple-600 border-purple-500/30"
				>
					<Sparkles className="h-3 w-3" />
					Models
				</Badge>
			)}
			{p.variables && (
				<Badge variant="outline" className="gap-1 text-xs">
					Variables
				</Badge>
			)}
			{p.cache && (
				<Badge variant="outline" className="gap-1 text-xs">
					Cache
				</Badge>
			)}
			{p.a2ui && (
				<Badge variant="outline" className="gap-1 text-xs">
					A2UI
				</Badge>
			)}
			{p.oauthScopes?.length > 0 && (
				<Tooltip>
					<TooltipTrigger>
						<Badge
							variant="outline"
							className="gap-1 text-xs text-orange-600 border-orange-500/30"
						>
							<Lock className="h-3 w-3" />
							OAuth ({p.oauthScopes.length})
						</Badge>
					</TooltipTrigger>
					<TooltipContent>
						{p.oauthScopes
							.map((s) => `${s.provider}: ${s.scopes.join(", ")}`)
							.join("\n")}
					</TooltipContent>
				</Tooltip>
			)}
		</div>
	);
}

function PermissionsDetail({ manifest }: { manifest: PackageManifest }) {
	const p = manifest.permissions;
	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-sm">
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> Resources
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>Memory</span>
						<span className="font-mono">{p.memory}</span>
					</div>
					<div className="flex justify-between">
						<span>Timeout</span>
						<span className="font-mono">{p.timeout}</span>
					</div>
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Globe className="h-3.5 w-3.5" /> Network
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>HTTP</span>
						<span>{p.network?.httpEnabled ? "Yes" : "No"}</span>
					</div>
					{p.network?.allowedHosts?.length > 0 && (
						<div>
							<span className="text-xs">Allowed hosts:</span>
							<div className="flex flex-wrap gap-1 mt-0.5">
								{p.network.allowedHosts.map((h) => (
									<Badge key={h} variant="outline" className="text-[10px]">
										{h}
									</Badge>
								))}
							</div>
						</div>
					)}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> Filesystem
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							["Node Storage", p.filesystem?.nodeStorage],
							["User Storage", p.filesystem?.userStorage],
							["Upload Dir", p.filesystem?.uploadDir],
							["Cache Dir", p.filesystem?.cacheDir],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Zap className="h-3.5 w-3.5" /> Capabilities
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							["Variables", p.variables],
							["Cache", p.cache],
							["Streaming", p.streaming],
							["A2UI", p.a2ui],
							["Models/LLM", p.models],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

function NodePermissionsDetail({ permissions }: { permissions: string[] }) {
	if (permissions.length === 0) {
		return (
			<div className="rounded-xl border border-border/20 bg-muted/5 p-4">
				<p className="text-sm text-muted-foreground/70">
					This node does pure computation in the debug sandbox and does not
					request any extra host capabilities.
				</p>
			</div>
		);
	}

	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
			{permissions.map((permission) => {
				const meta = getPermissionMeta(permission);
				const Icon = meta.icon;
				return (
					<div
						key={permission}
						className="rounded-xl border border-border/20 bg-muted/5 p-4 space-y-2"
					>
						<div className="flex items-center gap-2">
							<Icon className="h-4 w-4 text-muted-foreground/70" />
							<span className="text-sm font-medium">{meta.label}</span>
						</div>
						<p className="text-xs text-muted-foreground/60">
							{meta.description}
						</p>
					</div>
				);
			})}
		</div>
	);
}

function NodePermissionsSummary({
	permissions,
	title = "Permissions",
	description,
	className,
}: {
	permissions: string[];
	title?: string;
	description?: string;
	className?: string;
}) {
	return (
		<div
			className={cn(
				"rounded-xl border border-border/20 bg-muted/5 p-3",
				className,
			)}
		>
			<div className="flex items-center justify-between gap-3">
				<div className="min-w-0">
					<div className="flex items-center gap-2">
						<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
						<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
							{title}
						</span>
					</div>
					{description && (
						<p className="mt-1 text-xs text-muted-foreground/60">
							{description}
						</p>
					)}
				</div>
				<Badge variant="outline" className="text-[10px] shrink-0">
					{permissions.length === 0
						? "Pure"
						: `${permissions.length} ${permissions.length === 1 ? "permission" : "permissions"}`}
				</Badge>
			</div>
			<div className="mt-3">
				<NodePermissionBadges permissions={permissions} showEmpty />
			</div>
		</div>
	);
}

function NodeCard({
	node,
	isSelected,
	onSelect,
}: {
	node: WasmNodeDefinition;
	isSelected: boolean;
	onSelect: () => void;
}) {
	const inputCount = node.pins.filter(
		(p) => p.pin_type === "Input" && p.data_type !== "Execution",
	).length;
	const outputCount = node.pins.filter(
		(p) => p.pin_type === "Output" && p.data_type !== "Execution",
	).length;

	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"w-full text-left rounded-lg border p-3 transition-colors",
				isSelected
					? "border-primary/40 bg-primary/5"
					: "border-border/20 hover:bg-muted/10",
			)}
		>
			<div className="flex items-start justify-between gap-2">
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						{node.icon && <span className="text-lg">{node.icon}</span>}
						<span className="font-medium text-sm truncate">
							{node.friendly_name}
						</span>
					</div>
					{node.description && (
						<p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">
							{node.description}
						</p>
					)}
				</div>
				<Badge variant="secondary" className="text-[10px] shrink-0">
					{node.category}
				</Badge>
			</div>
			<div className="flex items-center gap-2 mt-2">
				<span className="text-[10px] text-muted-foreground/60">
					{inputCount} in / {outputCount} out
				</span>
				{node.long_running && (
					<Badge variant="outline" className="text-[10px]">
						Long Running
					</Badge>
				)}
			</div>
			<div className="mt-3">
				<NodePermissionsSummary
					permissions={node.permissions}
					title="Requires"
					description="Sandbox capabilities this node asks for during execution."
					className={cn(
						"transition-colors",
						isSelected ? "border-primary/20 bg-primary/5" : "bg-background/50",
					)}
				/>
			</div>
		</button>
	);
}

function initInputDefaults(node: WasmNodeDefinition): Record<string, unknown> {
	const defaults: Record<string, unknown> = {};
	for (const pin of node.pins) {
		if (pin.pin_type === "Input" && pin.data_type !== "Execution") {
			defaults[pin.name] = getDefaultValue(pin);
		}
	}
	return defaults;
}

function SeverityIcon({ severity }: { severity: LintSeverity }) {
	switch (severity) {
		case "error":
			return <AlertCircle className="h-3.5 w-3.5 text-destructive shrink-0" />;
		case "warning":
			return <AlertTriangle className="h-3.5 w-3.5 text-amber-500 shrink-0" />;
		case "info":
			return <Info className="h-3.5 w-3.5 text-blue-500 shrink-0" />;
	}
}

function LintPanel({
	issues,
	counts,
	onJumpToNode,
}: {
	issues: LintIssue[];
	counts: { errors: number; warnings: number; infos: number };
	onJumpToNode: (nodeIndex: number) => void;
}) {
	const [filter, setFilter] = useState<LintSeverity | "all">("all");

	const filtered = useMemo(
		() =>
			filter === "all" ? issues : issues.filter((i) => i.severity === filter),
		[issues, filter],
	);

	const total = counts.errors + counts.warnings + counts.infos;

	return (
		<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					<ShieldCheck className="h-3.5 w-3.5 text-muted-foreground/60" />
					<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
						Node Lint
					</span>
				</div>
				<div className="flex items-center gap-1.5">
					{counts.errors > 0 && (
						<Badge
							variant={filter === "error" ? "destructive" : "outline"}
							className="text-[10px] cursor-pointer gap-1"
							onClick={() => setFilter(filter === "error" ? "all" : "error")}
						>
							<AlertCircle className="h-3 w-3" />
							{counts.errors} error{counts.errors !== 1 ? "s" : ""}
						</Badge>
					)}
					{counts.warnings > 0 && (
						<Badge
							variant="outline"
							className={cn(
								"text-[10px] cursor-pointer gap-1",
								filter === "warning"
									? "bg-amber-500/20 text-amber-600 border-amber-500/40"
									: "text-amber-600 border-amber-500/20",
							)}
							onClick={() =>
								setFilter(filter === "warning" ? "all" : "warning")
							}
						>
							<AlertTriangle className="h-3 w-3" />
							{counts.warnings} warning{counts.warnings !== 1 ? "s" : ""}
						</Badge>
					)}
					{counts.infos > 0 && (
						<Badge
							variant="outline"
							className={cn(
								"text-[10px] cursor-pointer gap-1",
								filter === "info"
									? "bg-blue-500/20 text-blue-600 border-blue-500/40"
									: "text-blue-600 border-blue-500/20",
							)}
							onClick={() => setFilter(filter === "info" ? "all" : "info")}
						>
							<Info className="h-3 w-3" />
							{counts.infos} info{counts.infos !== 1 ? "s" : ""}
						</Badge>
					)}
					{total === 0 && (
						<Badge
							variant="outline"
							className="text-[10px] gap-1 text-green-600 border-green-500/20"
						>
							<CheckCircle2 className="h-3 w-3" />
							All clear
						</Badge>
					)}
				</div>
			</div>

			{filtered.length === 0 ? (
				<div className="text-center py-8">
					<CheckCircle2 className="h-8 w-8 text-green-500/30 mx-auto mb-2" />
					<p className="text-sm text-muted-foreground/60">
						{total === 0 ? "No issues found" : "No issues matching filter"}
					</p>
				</div>
			) : (
				<ScrollArea className="max-h-125">
					<div className="space-y-2 pr-3">
						{filtered.map((issue, i) => (
							<button
								key={`${issue.nodeIndex}-${issue.severity}-${i}`}
								type="button"
								className={cn(
									"w-full text-left rounded-lg border p-3 transition-colors hover:bg-muted/10",
									issue.severity === "error"
										? "border-destructive/20 bg-destructive/5"
										: issue.severity === "warning"
											? "border-amber-500/20 bg-amber-500/5"
											: "border-blue-500/20 bg-blue-500/5",
								)}
								onClick={() => onJumpToNode(issue.nodeIndex)}
							>
								<div className="flex items-start gap-2">
									<SeverityIcon severity={issue.severity} />
									<div className="min-w-0 flex-1">
										<div className="flex items-center gap-2 mb-0.5">
											<span className="text-xs font-medium">
												{issue.nodeName}
											</span>
											{issue.pinName && (
												<Badge
													variant="outline"
													className="text-[10px] font-mono"
												>
													{issue.pinName}
												</Badge>
											)}
										</div>
										<p className="text-xs text-muted-foreground/70">
											{issue.message}
										</p>
									</div>
								</div>
							</button>
						))}
					</div>
				</ScrollArea>
			)}
		</div>
	);
}

function DebugPageContent() {
	const backend = useBackend();
	const router = useRouter();
	const searchParams = useSearchParams();
	const initialProject = searchParams.get("project") ?? "";
	const [wasmPath, setWasmPath] = useState("");
	const [nodes, setNodes] = useState<WasmNodeDefinition[]>([]);
	const [manifest, setManifest] = useState<PackageManifest | null>(null);
	const [isPackage, setIsPackage] = useState(false);
	const [selectedNodeIndex, setSelectedNodeIndex] = useState(0);
	const [loading, setLoading] = useState(false);
	const [running, setRunning] = useState(false);
	const [inputValues, setInputValues] = useState<Record<string, unknown>>({});
	const [result, setResult] = useState<WasmExecutionResult | null>(null);
	const [outputsExpanded, setOutputsExpanded] = useState(true);
	const [activeTab, setActiveTab] = useState("debug");
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
	);

	const selectedNode = nodes[selectedNodeIndex] ?? null;
	const availableModelBits = useMemo(
		() =>
			(profileBits.data ?? []).filter((bit) => MODEL_BIT_TYPES.has(bit.type)),
		[profileBits.data],
	);

	const inputPins = useMemo(
		() =>
			selectedNode?.pins.filter(
				(p) => p.pin_type === "Input" && p.data_type !== "Execution",
			) ?? [],
		[selectedNode],
	);

	const outputPins = useMemo(
		() =>
			selectedNode?.pins.filter(
				(p) => p.pin_type === "Output" && p.data_type !== "Execution",
			) ?? [],
		[selectedNode],
	);

	const lintIssues = useMemo(() => lintNodes(nodes), [nodes]);
	const lintCounts = useMemo(() => countBySeverity(lintIssues), [lintIssues]);
	const missingModelInput = useMemo(
		() =>
			inputPins.some(
				(pin) =>
					isModelBitPin(pin) && !isSelectedBitValue(inputValues[pin.name]),
			),
		[inputPins, inputValues],
	);
	const selectedNodeRequiresModels =
		selectedNode?.permissions.includes("models") ?? false;

	const selectWasm = useCallback(async () => {
		const selected = await open({
			multiple: false,
			filters: [{ name: "WASM Files", extensions: ["wasm"] }],
		});
		if (selected) setWasmPath(selected);
	}, []);

	const inspectNode = useCallback(async () => {
		if (!wasmPath) return;
		setLoading(true);
		setResult(null);
		setSelectedNodeIndex(0);
		try {
			const defs = await invoke<WasmNodeDefinition[]>(
				"developer_inspect_node",
				{ wasmPath },
			);
			setNodes(defs);
			setIsPackage(defs.length > 1);
			setManifest(null);
			if (defs.length > 0) {
				setInputValues(buildInitialInputValues(defs[0], availableModelBits));
			}
		} catch (err) {
			toast.error(`Failed to inspect: ${err}`);
		} finally {
			setLoading(false);
		}
	}, [availableModelBits, wasmPath]);

	const inspectProject = useCallback(
		async (projectPath?: string) => {
			const target =
				projectPath ?? (await open({ directory: true, multiple: false }));
			if (!target) return;
			setLoading(true);
			setResult(null);
			setSelectedNodeIndex(0);
			try {
				const inspection = await invoke<PackageInspection>(
					"developer_inspect_package",
					{ projectPath: target },
				);
				setNodes(inspection.nodes);
				setManifest(inspection.manifest);
				setIsPackage(inspection.isPackage);
				setWasmPath(inspection.wasmPath);
				if (inspection.nodes.length > 0)
					setInputValues(
						buildInitialInputValues(inspection.nodes[0], availableModelBits),
					);
			} catch (err) {
				toast.error(`Failed to inspect project: ${err}`);
			} finally {
				setLoading(false);
			}
		},
		[availableModelBits],
	);

	useEffect(() => {
		if (initialProject) inspectProject(initialProject);
	}, [initialProject, inspectProject]);

	const selectNode = useCallback(
		(index: number) => {
			setSelectedNodeIndex(index);
			setResult(null);
			const node = nodes[index];
			if (node) {
				setInputValues(buildInitialInputValues(node, availableModelBits));
			}
		},
		[availableModelBits, nodes],
	);

	useEffect(() => {
		if (!selectedNode) return;
		setInputValues((prev) =>
			applySpecialInputDefaults(selectedNode, prev, availableModelBits),
		);
	}, [availableModelBits, selectedNode]);

	const runNode = useCallback(async () => {
		if (!wasmPath || !selectedNode) return;
		setRunning(true);
		try {
			const res = await invoke<WasmExecutionResult>("developer_run_node", {
				input: {
					wasmPath,
					inputs: inputValues,
					nodeName: selectedNode.name,
				},
			});
			setResult(res);
			toast[res.error ? "error" : "success"](
				res.error ? `Node error: ${res.error}` : "Node executed successfully",
			);
		} catch (err) {
			toast.error(`Execution failed: ${err}`);
		} finally {
			setRunning(false);
		}
	}, [wasmPath, selectedNode, inputValues]);

	const setInputValue = useCallback((name: string, value: unknown) => {
		setInputValues((prev) => ({ ...prev, [name]: value }));
	}, []);

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center gap-4 pb-4 border-b border-border/10">
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
					onClick={() => router.push("/store/packages?tab=projects")}
				>
					<ArrowLeft className="h-4 w-4" />
				</Button>
				<div>
					<div className="flex items-center gap-2">
						<Bug className="h-4 w-4 text-muted-foreground/60" />
						<h1 className="text-2xl font-semibold tracking-tight">
							Debug Node
						</h1>
					</div>
					<p className="text-sm text-muted-foreground/70">
						Inspect package nodes, permissions, and test execution
					</p>
				</div>
			</div>

			<div className="flex-1 overflow-y-auto py-4 space-y-4">
				<div className="rounded-xl bg-muted/10 border border-border/20 p-3">
					<div className="flex items-center gap-3">
						<FileCode2 className="h-4 w-4 text-muted-foreground/60 shrink-0" />
						<Input
							value={wasmPath}
							onChange={(e) => setWasmPath(e.target.value)}
							placeholder="Path to .wasm file..."
							className="flex-1 h-9 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50"
						/>
						<Button
							variant="ghost"
							size="sm"
							onClick={selectWasm}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<FolderOpen className="h-3.5 w-3.5" />
							WASM
						</Button>
						<Button
							variant="ghost"
							size="sm"
							onClick={() => inspectProject()}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<Package className="h-3.5 w-3.5" />
							Project
						</Button>
						<Button
							size="sm"
							onClick={inspectNode}
							disabled={!wasmPath || loading}
							className="h-8 rounded-full gap-1.5 px-4"
						>
							{loading ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<>
									<Search className="h-3.5 w-3.5" />
									Inspect
								</>
							)}
						</Button>
					</div>
				</div>

				<AnimatePresence mode="wait">
					{nodes.length > 0 && (
						<motion.div
							key="package-view"
							initial={{ opacity: 0 }}
							animate={{ opacity: 1 }}
							exit={{ opacity: 0 }}
						>
							<Tabs
								value={activeTab}
								onValueChange={setActiveTab}
								className="space-y-4"
							>
								<TabsList>
									<TabsTrigger value="debug" className="gap-1.5">
										<Play className="h-3.5 w-3.5" />
										Debug
									</TabsTrigger>
									<TabsTrigger value="nodes" className="gap-1.5">
										<Package className="h-3.5 w-3.5" />
										Nodes ({nodes.length})
									</TabsTrigger>
									<TabsTrigger value="lint" className="gap-1.5">
										<ShieldCheck className="h-3.5 w-3.5" />
										Lint
										{lintCounts.errors > 0 ? (
											<Badge
												variant="destructive"
												className="text-[10px] ml-1 px-1.5 py-0 h-4"
											>
												{lintCounts.errors}
											</Badge>
										) : lintCounts.warnings > 0 ? (
											<Badge className="text-[10px] ml-1 px-1.5 py-0 h-4 bg-amber-500/10 text-amber-600 border-amber-500/20">
												{lintCounts.warnings}
											</Badge>
										) : (
											<Badge
												variant="outline"
												className="text-[10px] ml-1 px-1.5 py-0 h-4 text-green-600"
											>
												0
											</Badge>
										)}
									</TabsTrigger>
									{selectedNode && (
										<TabsTrigger value="permissions" className="gap-1.5">
											<Shield className="h-3.5 w-3.5" />
											Permissions
										</TabsTrigger>
									)}
								</TabsList>

								<TabsContent value="nodes" className="space-y-3">
									<div className="rounded-xl border border-border/20 bg-card/50 p-4">
										<div className="flex items-center gap-2 mb-3">
											<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
												Package Nodes
											</span>
											{isPackage && (
												<Badge variant="secondary" className="text-[10px]">
													Multi-node
												</Badge>
											)}
										</div>
										<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
											{nodes.map((node, i) => (
												<NodeCard
													key={node.name}
													node={node}
													isSelected={i === selectedNodeIndex}
													onSelect={() => {
														selectNode(i);
														setActiveTab("debug");
													}}
												/>
											))}
										</div>
									</div>
								</TabsContent>

								<TabsContent value="lint" className="space-y-3">
									<LintPanel
										issues={lintIssues}
										counts={lintCounts}
										onJumpToNode={(i) => {
											selectNode(i);
											setActiveTab("debug");
										}}
									/>
								</TabsContent>

								{selectedNode && (
									<TabsContent value="permissions" className="space-y-3">
										<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
											<div className="flex items-center gap-2">
												<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
												<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
													Node Permissions
												</span>
												<Badge variant="outline" className="text-[10px]">
													Runtime-enforced
												</Badge>
											</div>
											<p className="text-sm text-muted-foreground/70">
												These capabilities come from the selected node
												definition and are applied during debug execution.
											</p>
											<NodePermissionBadges
												permissions={selectedNode.permissions}
												showEmpty
											/>
											<div className="border-t border-border/10" />
											<NodePermissionsDetail
												permissions={selectedNode.permissions}
											/>
										</div>
										{manifest && (
											<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
												<div className="flex items-center gap-2">
													<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
													<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
														Package Resource Tiers
													</span>
												</div>
												<PermissionsBadges manifest={manifest} />
												<div className="border-t border-border/10" />
												<PermissionsDetail manifest={manifest} />
											</div>
										)}
									</TabsContent>
								)}

								<TabsContent value="debug" className="space-y-3">
									{selectedNode && (
										<div className="rounded-xl bg-muted/5 p-4">
											<div className="flex items-center justify-between">
												<div className="flex items-center gap-3">
													{selectedNode.icon && (
														<span className="text-2xl">
															{selectedNode.icon}
														</span>
													)}
													<div>
														<h2 className="text-lg font-semibold tracking-tight">
															{selectedNode.friendly_name}
														</h2>
														<p className="text-sm text-muted-foreground/70">
															{selectedNode.description}
														</p>
													</div>
												</div>
												<div className="flex items-center gap-2">
													<Badge variant="secondary">
														{selectedNode.category}
													</Badge>
													{nodes.length > 1 && (
														<Badge variant="outline" className="text-xs">
															{selectedNodeIndex + 1}/{nodes.length}
														</Badge>
													)}
												</div>
											</div>
											<div className="mt-3 flex flex-wrap items-center gap-2">
												<NodePermissionsSummary
													permissions={selectedNode.permissions}
													title="Runtime Permissions"
													description="These capabilities are granted to this node for debug execution."
													className="w-full"
												/>
											</div>
										</div>
									)}

									{selectedNode && (
										<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
											<div className="flex items-center gap-2">
												<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
												<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
													Execution Permissions
												</span>
												<Badge variant="outline" className="text-[10px]">
													Applied on Run
												</Badge>
											</div>
											<p className="text-sm text-muted-foreground/70">
												The debug runner uses these node-declared capabilities
												when instantiating the WASM sandbox.
											</p>
											<NodePermissionsDetail
												permissions={selectedNode.permissions}
											/>
										</div>
									)}

									<div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
										<div className="rounded-xl border border-border/20 bg-card/50 p-4">
											<div className="flex items-center justify-between mb-3">
												<div className="flex items-center gap-2">
													<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
														Input Pins
													</span>
													<Badge variant="outline" className="text-[10px]">
														{inputPins.length}
													</Badge>
												</div>
												<Button
													size="sm"
													onClick={runNode}
													disabled={running || !wasmPath || missingModelInput}
													className="h-8 rounded-full gap-1.5 px-4"
												>
													{running ? (
														<Loader2 className="h-3.5 w-3.5 animate-spin" />
													) : (
														<Play className="h-3.5 w-3.5" />
													)}
													Run
												</Button>
											</div>
											{inputPins.length === 0 ? (
												<p className="text-sm text-muted-foreground/60 text-center py-4">
													No input pins
												</p>
											) : (
												<ScrollArea className="max-h-125">
													<div className="space-y-4 pr-3">
														{selectedNodeRequiresModels &&
															missingModelInput && (
																<div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-700">
																	Select an LLM or VLM bit for this node before
																	running it.
																</div>
															)}
														{inputPins.map((pin) => (
															<div key={pin.name} className="space-y-1.5">
																<div className="flex items-center justify-between gap-2">
																	<Label className="text-sm font-medium">
																		{pin.friendly_name}
																	</Label>
																	<DataTypeBadge dataType={pin.data_type} />
																</div>
																{pin.description && (
																	<p className="text-xs text-muted-foreground/60">
																		{pin.description}
																	</p>
																)}
																<PinInput
																	pin={pin}
																	value={inputValues[pin.name]}
																	onChange={(v) => setInputValue(pin.name, v)}
																	availableModelBits={availableModelBits}
																/>
															</div>
														))}
													</div>
												</ScrollArea>
											)}
										</div>

										<div className="rounded-xl border border-border/20 bg-card/50 p-4">
											<button
												type="button"
												className="flex items-center justify-between w-full mb-3"
												onClick={() => setOutputsExpanded(!outputsExpanded)}
											>
												<div className="flex items-center gap-2">
													<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
														Output
													</span>
													{result && (
														<>
															{result.error ? (
																<AlertCircle className="h-3.5 w-3.5 text-destructive" />
															) : (
																<CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
															)}
														</>
													)}
												</div>
												{outputsExpanded ? (
													<ChevronDown className="h-4 w-4 text-muted-foreground/60" />
												) : (
													<ChevronRight className="h-4 w-4 text-muted-foreground/60" />
												)}
											</button>
											{outputsExpanded && (
												<>
													{!result ? (
														<div className="text-center py-8">
															<Play className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
															<p className="text-sm text-muted-foreground/60">
																Run the node to see output values
															</p>
														</div>
													) : result.error ? (
														<div className="bg-destructive/10 text-destructive rounded-lg p-3 text-sm">
															<p className="font-medium mb-1">Error</p>
															<pre className="text-xs whitespace-pre-wrap font-mono">
																{result.error}
															</pre>
														</div>
													) : (
														<ScrollArea className="max-h-125">
															<div className="space-y-4 pr-3">
																{outputPins.map((pin) => (
																	<OutputValue
																		key={pin.name}
																		name={pin.friendly_name}
																		value={
																			result.outputs[pin.name] ?? "(no value)"
																		}
																	/>
																))}
																{Object.keys(result.outputs).length === 0 && (
																	<p className="text-sm text-muted-foreground/60 text-center py-4">
																		No output values
																	</p>
																)}
																{result.activate_exec.length > 0 && (
																	<div className="pt-2">
																		<div className="border-t border-border/10 mb-3" />
																		<Label className="text-xs text-muted-foreground/60">
																			Activated Execution Pins
																		</Label>
																		<div className="flex gap-1 mt-1">
																			{result.activate_exec.map((e) => (
																				<Badge
																					key={e}
																					variant="outline"
																					className="text-xs"
																				>
																					{e}
																				</Badge>
																			))}
																		</div>
																	</div>
																)}
															</div>
														</ScrollArea>
													)}
												</>
											)}
										</div>
									</div>
								</TabsContent>
							</Tabs>
						</motion.div>
					)}
				</AnimatePresence>
			</div>
		</div>
	);
}

export default function DebugPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<DebugPageContent />
		</Suspense>
	);
}
