"use client";
import { Trans, useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	CalendarIcon,
	CloudIcon,
	FileIcon,
	HashIcon,
	PlayCircleIcon,
	ToggleLeftIcon,
	TypeIcon,
} from "lucide-react";
import { type RefObject, useCallback, useMemo, useState } from "react";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { Button } from "../../ui";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { ScrollArea } from "../../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";
import { typeToColor } from "../utils";

interface EventPayloadFormProps {
	node: INode;
	boardRef: RefObject<IBoard | undefined>;
	onLocalExecute?: (payload?: object) => Promise<void>;
	onRemoteExecute?: (payload?: object) => Promise<void>;
	canLocalExecute: boolean;
	canRemoteExecute: boolean;
	onClose: () => void;
}

interface SchemaProperty {
	type?: string;
	description?: string;
	default?: unknown;
	enum?: string[];
	items?: SchemaProperty;
	properties?: Record<string, SchemaProperty>;
	required?: string[];
}

interface JsonSchema {
	type?: string;
	properties?: Record<string, SchemaProperty>;
	required?: string[];
	description?: string;
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

const getDefaultFromSchema = (schema: JsonSchema): Record<string, unknown> => {
	const result: Record<string, unknown> = {};
	if (!schema.properties) return result;
	for (const [key, prop] of Object.entries(schema.properties)) {
		if (prop.default !== undefined) {
			result[key] = prop.default;
		} else if (prop.type === "string") {
			result[key] = "";
		} else if (prop.type === "number" || prop.type === "integer") {
			result[key] = 0;
		} else if (prop.type === "boolean") {
			result[key] = false;
		} else if (prop.type === "array") {
			result[key] = [];
		} else if (prop.type === "object") {
			result[key] = {};
		}
	}
	return result;
};

const getIconForType = (dataType: IVariableType) => {
	switch (dataType) {
		case IVariableType.Boolean:
			return <ToggleLeftIcon className="w-3 h-3" />;
		case IVariableType.Integer:
		case IVariableType.Float:
		case IVariableType.Byte:
			return <HashIcon className="w-3 h-3" />;
		case IVariableType.String:
			return <TypeIcon className="w-3 h-3" />;
		case IVariableType.Date:
			return <CalendarIcon className="w-3 h-3" />;
		case IVariableType.PathBuf:
			return <FileIcon className="w-3 h-3" />;
		case IVariableType.Struct:
			return <BracesIcon className="w-3 h-3" />;
		default:
			return null;
	}
};

const getDefaultValueForType = (
	dataType: IVariableType,
	valueType: IValueType,
): unknown => {
	if (valueType === IValueType.Array) return [];
	if (valueType === IValueType.HashMap) return {};
	if (valueType === IValueType.HashSet) return [];

	switch (dataType) {
		case IVariableType.Boolean:
			return false;
		case IVariableType.Integer:
		case IVariableType.Float:
		case IVariableType.Byte:
			return 0;
		case IVariableType.String:
			return "";
		case IVariableType.Date:
			return new Date().toISOString();
		default:
			return null;
	}
};

const isSimpleType = (dataType: IVariableType): boolean => {
	return [
		IVariableType.Boolean,
		IVariableType.Integer,
		IVariableType.Float,
		IVariableType.Byte,
		IVariableType.String,
		IVariableType.Date,
	].includes(dataType);
};

export function EventPayloadForm({
	node,
	boardRef,
	onLocalExecute,
	onRemoteExecute,
	canLocalExecute,
	canRemoteExecute,
	onClose,
}: Readonly<EventPayloadFormProps>) {
	const { t } = useTranslation("flow");
	const refs = boardRef.current?.refs;

	const outputPins = useMemo(() => {
		return Object.values(node.pins)
			.filter(
				(pin) =>
					pin.pin_type === IPinType.Output &&
					pin.data_type !== IVariableType.Execution,
			)
			.sort((a, b) => a.index - b.index);
	}, [node.pins]);

	const pinSchemas = useMemo(() => {
		const schemas: Record<string, JsonSchema | null> = {};
		for (const pin of outputPins) {
			if (pin.data_type === IVariableType.Struct && pin.schema) {
				schemas[pin.id] = parseSchema(pin.schema, refs);
			}
		}
		return schemas;
	}, [outputPins, refs]);

	const hasFormFields = useMemo(() => {
		return outputPins.some((pin) => {
			if (isSimpleType(pin.data_type) && pin.value_type === IValueType.Normal)
				return true;
			if (pin.data_type === IVariableType.Struct && pinSchemas[pin.id])
				return true;
			return false;
		});
	}, [outputPins, pinSchemas]);

	const [formValues, setFormValues] = useState<Record<string, unknown>>(() => {
		const initial: Record<string, unknown> = {};
		for (const pin of outputPins) {
			if (isSimpleType(pin.data_type) && pin.value_type === IValueType.Normal) {
				initial[pin.name] = getDefaultValueForType(
					pin.data_type,
					pin.value_type,
				);
			} else if (pin.data_type === IVariableType.Struct) {
				const schema = pinSchemas[pin.id];
				if (schema) {
					initial[pin.name] = getDefaultFromSchema(schema);
				}
			}
		}
		return initial;
	});

	const [jsonPayload, setJsonPayload] = useState<string>("");
	const [useJsonMode, setUseJsonMode] = useState(!hasFormFields);

	const handleFieldChange = useCallback((pinName: string, value: unknown) => {
		setFormValues((prev) => ({ ...prev, [pinName]: value }));
	}, []);

	const handleStructFieldChange = useCallback(
		(pinName: string, fieldName: string, value: unknown) => {
			setFormValues((prev) => ({
				...prev,
				[pinName]: {
					...(prev[pinName] as Record<string, unknown>),
					[fieldName]: value,
				},
			}));
		},
		[],
	);

	const buildPayload = useCallback((): object | undefined => {
		if (useJsonMode) {
			try {
				return jsonPayload ? JSON.parse(jsonPayload) : undefined;
			} catch {
				return undefined;
			}
		}

		const payload: Record<string, unknown> = {};
		let hasValues = false;

		for (const pin of outputPins) {
			const value = formValues[pin.name];
			if (value !== undefined && value !== null && value !== "") {
				if (
					typeof value === "object" &&
					Object.keys(value as object).length === 0
				) {
					continue;
				}
				payload[pin.name] = value;
				hasValues = true;
			}
		}

		return hasValues ? payload : undefined;
	}, [useJsonMode, jsonPayload, formValues, outputPins]);

	const handleLocalExecute = useCallback(async () => {
		if (!onLocalExecute) return;
		const payload = buildPayload();
		await onLocalExecute(payload);
		onClose();
	}, [buildPayload, onLocalExecute, onClose]);

	const handleRemoteExecute = useCallback(async () => {
		if (!onRemoteExecute) return;
		const payload = buildPayload();
		await onRemoteExecute(payload);
		onClose();
	}, [buildPayload, onRemoteExecute, onClose]);

	const renderSchemaField = useCallback(
		(
			pinName: string,
			fieldName: string,
			prop: SchemaProperty,
			required: boolean,
		) => {
			const structValue = formValues[pinName] as Record<string, unknown>;
			const value = structValue?.[fieldName];
			const label = `${fieldName}${required ? " *" : ""}`;

			if (prop.enum && prop.enum.length > 0) {
				return (
					<div key={fieldName} className="space-y-1">
						<Label className="text-xs">{label}</Label>
						<Select
							value={String(value ?? "")}
							onValueChange={(v) =>
								handleStructFieldChange(pinName, fieldName, v)
							}
						>
							<SelectTrigger className="h-8">
								<SelectValue placeholder={`Select ${fieldName}`} />
							</SelectTrigger>
							<SelectContent>
								{prop.enum.map((option) => (
									<SelectItem key={option} value={option}>
										{option}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						{prop.description && (
							<p className="text-xs text-muted-foreground">
								{prop.description}
							</p>
						)}
					</div>
				);
			}

			switch (prop.type) {
				case "boolean":
					return (
						<div key={fieldName} className="flex items-center space-x-2 py-1">
							<Checkbox
								id={`${pinName}-${fieldName}`}
								checked={Boolean(value)}
								onCheckedChange={(checked) =>
									handleStructFieldChange(pinName, fieldName, checked)
								}
							/>
							<Label
								htmlFor={`${pinName}-${fieldName}`}
								className="text-xs cursor-pointer"
							>
								{label}
							</Label>
						</div>
					);

				case "integer":
					return (
						<div key={fieldName} className="space-y-1">
							<Label className="text-xs">{label}</Label>
							<Input
								type="number"
								step="1"
								className="h-8"
								value={String(value ?? "")}
								onChange={(e) =>
									handleStructFieldChange(
										pinName,
										fieldName,
										e.target.value ? Number.parseInt(e.target.value, 10) : "",
									)
								}
								placeholder={prop.description || t('enterFieldname', 'Enter {{fieldName}}', { fieldName })}
							/>
						</div>
					);

				case "number":
					return (
						<div key={fieldName} className="space-y-1">
							<Label className="text-xs">{label}</Label>
							<Input
								type="number"
								step="0.1"
								className="h-8"
								value={String(value ?? "")}
								onChange={(e) =>
									handleStructFieldChange(
										pinName,
										fieldName,
										e.target.value ? Number.parseFloat(e.target.value) : "",
									)
								}
								placeholder={prop.description || t('enterFieldname', 'Enter {{fieldName}}', { fieldName })}
							/>
						</div>
					);

				case "array":
				case "object":
					return (
						<div key={fieldName} className="space-y-1">
							<Label className="text-xs">{label}</Label>
							<Textarea
								className="font-mono text-xs h-16"
								value={
									typeof value === "object"
										? JSON.stringify(value, null, 2)
										: String(value ?? "")
								}
								onChange={(e) => {
									try {
										const parsed = JSON.parse(e.target.value);
										handleStructFieldChange(pinName, fieldName, parsed);
									} catch {
										// Keep raw string on invalid JSON
									}
								}}
								placeholder={prop.description || t('enterFieldnameAsJson', 'Enter {{fieldName}} as JSON', { fieldName })}
							/>
						</div>
					);

				default:
					return (
						<div key={fieldName} className="space-y-1">
							<Label className="text-xs">{label}</Label>
							<Input
								type="text"
								className="h-8"
								value={String(value ?? "")}
								onChange={(e) =>
									handleStructFieldChange(pinName, fieldName, e.target.value)
								}
								placeholder={prop.description || t('enterFieldname', 'Enter {{fieldName}}', { fieldName })}
							/>
						</div>
					);
			}
		},
		[formValues, handleStructFieldChange],
	);

	const renderPinField = useCallback(
		(pin: IPin) => {
			const color = typeToColor(pin.data_type);
			const icon = getIconForType(pin.data_type);
			const value = formValues[pin.name];
			const description = resolveRef(pin.description, refs);

			// Handle struct with schema
			if (pin.data_type === IVariableType.Struct) {
				const schema = pinSchemas[pin.id];
				if (schema) {
					const properties = schema.properties || {};
					const requiredFields = new Set(schema.required || []);

					return (
						<div key={pin.id} className="space-y-3">
							<div className="flex items-center gap-2">
								<span
									className="w-2 h-2 rounded-full"
									style={{ backgroundColor: color }}
								/>
								{icon}
								<Label className="font-medium">{pin.friendly_name}</Label>
							</div>
							{description && (
								<p className="text-xs text-muted-foreground -mt-1">
									{description}
								</p>
							)}
							<div className="pl-4 border-l-2 space-y-2">
								{Object.entries(properties).map(([fieldName, prop]) =>
									renderSchemaField(
										pin.name,
										fieldName,
										prop,
										requiredFields.has(fieldName),
									),
								)}
							</div>
						</div>
					);
				}
			}

			// Skip non-simple types without schema
			if (
				pin.data_type === IVariableType.Generic ||
				pin.data_type === IVariableType.Struct ||
				pin.value_type !== IValueType.Normal
			) {
				return null;
			}

			if (pin.options?.valid_values && pin.options.valid_values.length > 0) {
				return (
					<div key={pin.id} className="space-y-2">
						<Label className="flex items-center gap-2">
							<span
								className="w-2 h-2 rounded-full"
								style={{ backgroundColor: color }}
							/>
							{icon}
							{pin.friendly_name}
						</Label>
						<Select
							value={String(value ?? "")}
							onValueChange={(v) => handleFieldChange(pin.name, v)}
						>
							<SelectTrigger>
								<SelectValue placeholder={`Select ${pin.friendly_name}`} />
							</SelectTrigger>
							<SelectContent>
								{pin.options.valid_values.map((option) => (
									<SelectItem key={option} value={option}>
										{option}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						{description && (
							<p className="text-xs text-muted-foreground">{description}</p>
						)}
					</div>
				);
			}

			switch (pin.data_type) {
				case IVariableType.Boolean:
					return (
						<div key={pin.id} className="flex items-center space-x-3 py-2">
							<Checkbox
								id={pin.id}
								checked={Boolean(value)}
								onCheckedChange={(checked) =>
									handleFieldChange(pin.name, checked)
								}
							/>
							<div className="flex flex-col">
								<Label
									htmlFor={pin.id}
									className="flex items-center gap-2 cursor-pointer"
								>
									<span
										className="w-2 h-2 rounded-full"
										style={{ backgroundColor: color }}
									/>
									{icon}
									{pin.friendly_name}
								</Label>
								{description && (
									<p className="text-xs text-muted-foreground">{description}</p>
								)}
							</div>
						</div>
					);

				case IVariableType.Integer:
				case IVariableType.Byte:
					return (
						<div key={pin.id} className="space-y-2">
							<Label className="flex items-center gap-2">
								<span
									className="w-2 h-2 rounded-full"
									style={{ backgroundColor: color }}
								/>
								{icon}
								{pin.friendly_name}
							</Label>
							<Input
								type="number"
								step="1"
								min={pin.options?.range?.[0]}
								max={pin.options?.range?.[1]}
								value={String(value ?? "")}
								onChange={(e) =>
									handleFieldChange(
										pin.name,
										e.target.value ? Number.parseInt(e.target.value, 10) : "",
									)
								}
								placeholder={description || t('enterFriendly_name', 'Enter {{friendly_name}}', { friendly_name: pin.friendly_name })}
							/>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
					);

				case IVariableType.Float:
					return (
						<div key={pin.id} className="space-y-2">
							<Label className="flex items-center gap-2">
								<span
									className="w-2 h-2 rounded-full"
									style={{ backgroundColor: color }}
								/>
								{icon}
								{pin.friendly_name}
							</Label>
							<Input
								type="number"
								step={pin.options?.step ?? 0.1}
								min={pin.options?.range?.[0]}
								max={pin.options?.range?.[1]}
								value={String(value ?? "")}
								onChange={(e) =>
									handleFieldChange(
										pin.name,
										e.target.value ? Number.parseFloat(e.target.value) : "",
									)
								}
								placeholder={description || t('enterFriendly_name', 'Enter {{friendly_name}}', { friendly_name: pin.friendly_name })}
							/>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
					);

				case IVariableType.Date:
					return (
						<div key={pin.id} className="space-y-2">
							<Label className="flex items-center gap-2">
								<span
									className="w-2 h-2 rounded-full"
									style={{ backgroundColor: color }}
								/>
								{icon}
								{pin.friendly_name}
							</Label>
							<Input
								type="datetime-local"
								value={
									value
										? new Date(value as string).toISOString().slice(0, 16)
										: ""
								}
								onChange={(e) =>
									handleFieldChange(
										pin.name,
										e.target.value
											? new Date(e.target.value).toISOString()
											: "",
									)
								}
							/>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
					);

				default:
					return (
						<div key={pin.id} className="space-y-2">
							<Label className="flex items-center gap-2">
								<span
									className="w-2 h-2 rounded-full"
									style={{ backgroundColor: color }}
								/>
								{icon}
								{pin.friendly_name}
							</Label>
							<Input
								type="text"
								value={String(value ?? "")}
								onChange={(e) => handleFieldChange(pin.name, e.target.value)}
								placeholder={description || t('enterFriendly_name', 'Enter {{friendly_name}}', { friendly_name: pin.friendly_name })}
							/>
							{description && (
								<p className="text-xs text-muted-foreground">{description}</p>
							)}
						</div>
					);
			}
		},
		[formValues, handleFieldChange, refs, pinSchemas, renderSchemaField],
	);

	const formFields = useMemo(
		() =>
			outputPins.filter((pin) => {
				if (isSimpleType(pin.data_type) && pin.value_type === IValueType.Normal)
					return true;
				if (pin.data_type === IVariableType.Struct && pinSchemas[pin.id])
					return true;
				return false;
			}),
		[outputPins, pinSchemas],
	);

	const complexFields = useMemo(
		() =>
			outputPins.filter((pin) => {
				if (isSimpleType(pin.data_type) && pin.value_type === IValueType.Normal)
					return false;
				if (pin.data_type === IVariableType.Struct && pinSchemas[pin.id])
					return false;
				return true;
			}),
		[outputPins, pinSchemas],
	);

	return (
		<div className="space-y-4">
			{hasFormFields && (
				<div className="flex items-center justify-end gap-2">
					<Label htmlFor="json-mode" className="text-xs text-muted-foreground">
						{t('jsonMode', 'JSON Mode')}
					</Label>
					<Checkbox
						id="json-mode"
						checked={useJsonMode}
						onCheckedChange={(checked) => setUseJsonMode(Boolean(checked))}
					/>
				</div>
			)}

			{useJsonMode ? (
				<div className="space-y-2">
					<Label>{t('jsonPayload', 'JSON Payload')}</Label>
					<Textarea
						rows={10}
						placeholder={`{"key": "value"}`}
						value={jsonPayload}
						onChange={(e) => setJsonPayload(e.target.value)}
						className="font-mono text-sm"
					/>
					<p className="text-xs text-muted-foreground">
						{t('enterAValidJsonObjectKeysShouldMatchTheOutputPinNames', 'Enter a valid JSON object. Keys should match the output pin names.')}
					</p>
				</div>
			) : (
				<ScrollArea className="max-h-[400px] pr-4 overflow-auto">
					<div className="space-y-4">
						{formFields.map((pin) => renderPinField(pin))}

						{complexFields.length > 0 && (
							<div className="pt-4 border-t">
								<p className="text-sm text-muted-foreground mb-2">
									{t('theFollowingFieldsRequireJsonInputSwitchToJsonModeToConfigureThem', "The following fields require JSON input. Switch to JSON mode to configure them:")}
								</p>
								<ul className="text-xs text-muted-foreground space-y-1">
									{complexFields.map((pin) => (
										<li key={pin.id} className="flex items-center gap-2"><Trans i18nKey="spanClassnamew2H2RoundedfullStyleBackgroundcolorTypetocolorpindata_typeFriendly_namePinfriendly_nameData_typePindata_type"><span
												className="w-2 h-2 rounded-full"
												style={{ backgroundColor: typeToColor(pin.data_type) }}
											/>
											{{ friendly_name: pin.friendly_name }} ({{ data_type: pin.data_type }}</Trans>{pin.value_type !== IValueType.Normal &&
												` - ${pin.value_type}`}
											)
										</li>
									))}
								</ul>
							</div>
						)}
					</div>
				</ScrollArea>
			)}

			<div className="flex gap-2 pt-4 border-t">
				{canLocalExecute && (
					<Button className="flex-1" onClick={handleLocalExecute}>
						<PlayCircleIcon className="w-4 h-4 mr-2" />
						{t('executeLocally2', 'Execute Locally')}
					</Button>
				)}
				{canRemoteExecute && (
					<Button
						className="flex-1"
						variant={canLocalExecute ? "secondary" : "default"}
						onClick={handleRemoteExecute}
					>
						<CloudIcon className="w-4 h-4 mr-2" />
						{t('executeOnServer2', 'Execute on Server')}
					</Button>
				)}
			</div>
		</div>
	);
}
