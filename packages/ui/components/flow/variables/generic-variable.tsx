"use client";

import { useCallback, useState } from "react";
import { Label } from "../../../components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../components/ui/select";
import { type IVariable } from "../../../lib/schema/flow/variable";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";

const decodeJsonBytes = (value: number[] | null | undefined): string => {
	if (!value || value.length === 0) return "";
	try {
		return new TextDecoder("utf-8").decode(new Uint8Array(value));
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

export function GenericVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const [jsonValue, setJsonValue] = useState(() =>
		decodeJsonBytes(variable.default_value),
	);
	const [template, setTemplate] = useState<GenericTemplate>("custom");
	const [jsonError, setJsonError] = useState<string | null>(null);
	const [isFocused, setIsFocused] = useState(false);

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
			setTemplate("custom");

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
			setTemplate(nextTemplate);
			setJsonError(null);

			if (nextTemplate === "unset") {
				setJsonValue("");
				clearDefaultValue();
				return;
			}

			if (nextTemplate === "custom") {
				return;
			}

			const nextValue = templateJson(nextTemplate);
			setJsonValue(nextValue);
			commitJsonText(nextValue);
		},
		[clearDefaultValue, commitJsonText],
	);

	return (
		<div className="grid w-full items-center gap-1.5">
			<div className="grid gap-1.5">
				<Label htmlFor="generic_value_template">Value Type</Label>
				<Select
					disabled={disabled}
					value={template}
					onValueChange={(value) =>
						handleTemplateChange(value as GenericTemplate)
					}
				>
					<SelectTrigger id="generic_value_template">
						<SelectValue placeholder="Value Type" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="custom">Custom JSON</SelectItem>
						<SelectItem value="unset">No default</SelectItem>
						<SelectItem value="string">String</SelectItem>
						<SelectItem value="integer">Integer</SelectItem>
						<SelectItem value="float">Float</SelectItem>
						<SelectItem value="boolean">Boolean</SelectItem>
						<SelectItem value="object">Object</SelectItem>
						<SelectItem value="array">Array</SelectItem>
						<SelectItem value="null">Null</SelectItem>
					</SelectContent>
				</Select>
			</div>

			<Label htmlFor="generic_default_value">JSON Value</Label>
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
		</div>
	);
}
