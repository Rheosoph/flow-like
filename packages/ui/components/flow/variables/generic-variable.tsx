"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Label } from "../../../components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../components/ui/select";
import type { IVariable } from "../../../lib/schema/flow/variable";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";

const shouldPrettyPrintDecodedJson = (decoded: string): boolean => {
	const trimmed = decoded.trimStart();
	return trimmed.startsWith("{") || trimmed.startsWith("[");
};

const decodeJsonBytes = (value: number[] | null | undefined): string => {
	if (!value || value.length === 0) return "";
	try {
		const decoded = new TextDecoder("utf-8", { fatal: true }).decode(
			new Uint8Array(value),
		);
		if (!shouldPrettyPrintDecodedJson(decoded)) return decoded;

		try {
			return JSON.stringify(JSON.parse(decoded), null, 2) ?? decoded;
		} catch {
			return decoded;
		}
	} catch {
		const parsed = parseUint8ArrayToJson(value);
		if (parsed === undefined) return "";
		return JSON.stringify(parsed, null, 2) ?? "null";
	}
};

const encodeJsonText = (value: string): number[] => {
	return Array.from(new TextEncoder().encode(value));
};

type GenericTemplate =
	| "custom"
	| "unset"
	| "string"
	| "integer"
	| "float"
	| "boolean"
	| "object"
	| "array"
	| "null";

const templateJson = (template: GenericTemplate): string => {
	switch (template) {
		case "string":
			return '""';
		case "integer":
			return "0";
		case "float":
			return "0.0";
		case "boolean":
			return "false";
		case "object":
			return "{}";
		case "array":
			return "[]";
		case "null":
			return "null";
		default:
			return "";
	}
};

const detectTemplate = (value: string): GenericTemplate => {
	const trimmed = value.trim();
	if (trimmed === "") return "unset";

	let parsed: unknown;
	try {
		parsed = JSON.parse(trimmed);
	} catch {
		return "custom";
	}

	if (parsed === null) return "null";
	if (Array.isArray(parsed)) return "array";

	switch (typeof parsed) {
		case "string":
			return "string";
		case "boolean":
			return "boolean";
		case "number":
			return /[.eE]/.test(trimmed) ? "float" : "integer";
		case "object":
			return "object";
		default:
			return "custom";
	}
};

export function GenericVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const [jsonValue, setJsonValue] = useState(() =>
		decodeJsonBytes(variable.default_value),
	);
	const [jsonError, setJsonError] = useState<string | null>(null);
	const [isFocused, setIsFocused] = useState(false);

	const template = useMemo(() => detectTemplate(jsonValue), [jsonValue]);

	useEffect(() => {
		if (isFocused) return;
		const nextJsonValue = decodeJsonBytes(variable.default_value);
		if (nextJsonValue === jsonValue) return;

		setJsonValue(nextJsonValue);
		setJsonError(null);
	}, [isFocused, jsonValue, variable.default_value]);

	const commitJsonText = useCallback(
		(nextValue: string) => {
			onChange({
				...variable,
				default_value: encodeJsonText(nextValue),
			});
		},
		[onChange, variable],
	);

	const clearDefaultValue = useCallback(() => {
		onChange({
			...variable,
			default_value: null,
		});
	}, [onChange, variable]);

	const handleJsonChange = useCallback(
		(nextJson: string) => {
			setJsonValue(nextJson);

			if (nextJson.trim() === "") {
				setJsonError(null);
				clearDefaultValue();
				return;
			}

			try {
				JSON.parse(nextJson);
				setJsonError(null);
				commitJsonText(nextJson);
			} catch {
				setJsonError("Invalid JSON");
			}
		},
		[clearDefaultValue, commitJsonText],
	);

	const handleTemplateChange = useCallback(
		(nextTemplate: GenericTemplate) => {
			if (nextTemplate === "custom" || nextTemplate === template) return;

			setJsonError(null);

			if (nextTemplate === "unset") {
				setJsonValue("");
				clearDefaultValue();
				return;
			}

			const nextValue = templateJson(nextTemplate);
			setJsonValue(nextValue);
			commitJsonText(nextValue);
		},
		[clearDefaultValue, commitJsonText, template],
	);

	return (
		<div className="grid w-full items-center gap-1.5">
			<div className="grid gap-1.5">
				<Label htmlFor="generic_value_template">
					{t("valueType", "Value Type")}
				</Label>
				<Select
					disabled={disabled}
					value={template}
					onValueChange={(value) =>
						handleTemplateChange(value as GenericTemplate)
					}
				>
					<SelectTrigger id="generic_value_template">
						<SelectValue placeholder={t("valueType", "Value Type")} />
					</SelectTrigger>
					<SelectContent>
						{template === "custom" && (
							<SelectItem value="custom">
								{t("customJson", "Custom JSON")}
							</SelectItem>
						)}
						<SelectItem value="unset">
							{t("noDefault", "No default")}
						</SelectItem>
						<SelectItem value="string">{t("string", "String")}</SelectItem>
						<SelectItem value="integer">{t("integer", "Integer")}</SelectItem>
						<SelectItem value="float">{t("float", "Float")}</SelectItem>
						<SelectItem value="boolean">{t("boolean", "Boolean")}</SelectItem>
						<SelectItem value="object">{t("object", "Object")}</SelectItem>
						<SelectItem value="array">{t("array", "Array")}</SelectItem>
						<SelectItem value="null">{t("null", "Null")}</SelectItem>
					</SelectContent>
				</Select>
			</div>

			<Label htmlFor="generic_default_value">
				{t("jsonValue2", "JSON Value")}
			</Label>
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
					id="generic_default_value"
					disabled={disabled}
					value={jsonValue}
					onChange={(event) => handleJsonChange(event.target.value)}
					onFocus={() => setIsFocused(true)}
					onBlur={() => setIsFocused(false)}
					placeholder={`{"key": "value"}`}
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
		</div>
	);
}
