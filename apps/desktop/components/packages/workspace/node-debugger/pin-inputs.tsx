"use client";

import {
	Accordion,
	AccordionContent,
	AccordionItem,
	AccordionTrigger,
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
import type { IBit } from "@flow-like/flow-like-ui/lib/schema/bit/bit";
import type { WasmPinDefinition } from "@flow-like/flow-like-ui/lib/schema/developer";
import { useTranslation } from "@flow-like/locales";
import { ChevronRight, Code2, Minus, Plus } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import {
	type JsonSchema,
	createDefaultFromSchema,
	getBitDisplayName,
	getBitKey,
	getBitProviderName,
	isModelBitPin,
	isSelectedBitValue,
	parseSchema,
	resolveSchema,
} from "./schema";

function ModelBitInput({
	bits,
	value,
	onChange,
}: {
	bits: IBit[];
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	const { t } = useTranslation("common");
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
					<SelectValue
						placeholder={t("selectLlmvlmBit", "Select LLM/VLM bit...")}
					/>
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
					{t(
						"noLlmOrVlmBitsAreAvailableInTheCurrentProfile",
						"No LLM or VLM bits are available in the current profile.",
					)}
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
	const { t } = useTranslation("common");
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
					placeholder={
						resolved.description ??
						t("enterVal", "Enter {{val}}...", { val: label ?? "value" })
					}
				/>
			);

		default:
			return (
				<Input
					value={String(value ?? "")}
					onChange={(e) => onChange(e.target.value)}
					className="h-9"
					placeholder={t("enterVal", "Enter {{val}}...", {
						val: label ?? "value",
					})}
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
	const { t } = useTranslation("common");
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
							// biome-ignore lint/suspicious/noArrayIndexKey: items are positional JSON values without an identity of their own
							key={`item-${i}`}
							value={String(i)}
							className="border border-border/20 rounded-lg overflow-hidden"
						>
							<div className="flex items-center">
								<AccordionTrigger className="flex-1 px-3 py-2 text-xs hover:no-underline">
									<span className="font-mono text-muted-foreground/70">{`[${i}]`}</span>
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
								<SchemaField
									schema={itemSchema}
									rootSchema={rootSchema}
									value={item}
									onChange={(v) => updateItem(i, v)}
								/>
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
				{t("addItem", "Add Item")}
			</Button>
		</div>
	);
}

function JsonModeButton({ onClick }: { onClick: () => void }) {
	return (
		<div className="flex justify-end">
			<Button
				variant="ghost"
				size="sm"
				onClick={onClick}
				className="h-6 text-[10px] gap-1 px-2"
			>
				<Code2 className="h-3 w-3" />
				{"JSON"}
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
	const { t } = useTranslation("common");
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
							{t("form", "Form")}
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

	if (isArray) {
		const itemSchema: JsonSchema =
			schema.type === "object" ? schema : (schema.items ?? schema);
		return (
			<div className="space-y-1.5">
				<JsonModeButton onClick={() => setRawMode(true)} />
				<SchemaArrayField
					itemSchema={itemSchema}
					rootSchema={schema}
					value={value}
					onChange={onChange}
				/>
			</div>
		);
	}

	return (
		<div className="space-y-1.5">
			<JsonModeButton onClick={() => setRawMode(true)} />
			<SchemaField
				schema={schema}
				rootSchema={schema}
				value={value}
				onChange={onChange}
			/>
		</div>
	);
}

export function PinInput({
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
	const { t } = useTranslation("common");
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
					placeholder={t("enterValValue", "Enter {{val}} value...", {
						val: pin.data_type.toLowerCase(),
					})}
				/>
			);
	}
}

const DATA_TYPE_TONE: Record<string, string> = {
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

export function DataTypeBadge({ dataType }: { dataType: string }) {
	return (
		<Badge
			variant="outline"
			className={cn("text-[10px] font-mono", DATA_TYPE_TONE[dataType])}
		>
			{dataType}
		</Badge>
	);
}

export function OutputValue({ name, value }: { name: string; value: unknown }) {
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
