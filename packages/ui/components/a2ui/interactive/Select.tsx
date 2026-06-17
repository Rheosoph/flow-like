"use client";

import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { useComponentActionTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, SelectComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	// Handle literalOptions directly
	if ("literalOptions" in boundValue) {
		return boundValue.literalOptions as T;
	}
	return resolve(boundValue) as T;
}

function resolveStringOrBound(
	value: string | BoundValue | undefined,
): string | undefined {
	if (!value) return undefined;
	if (typeof value === "string") return value;
	if ("literalString" in value) return value.literalString;
	if ("literalNumber" in value) return String(value.literalNumber);
	return undefined;
}

export function A2UISelect({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<SelectComponent>) {
	const onAction = useOnAction();
	const triggerAction = useComponentActionTrigger(componentId);
	const value = useResolved<string>(component.value);
	const options =
		useResolved<Array<{ value: string; label: string }>>(component.options) ??
		[];
	const disabled = useResolved<boolean>(component.disabled);
	const label = resolveStringOrBound(
		component.label as string | BoundValue | undefined,
	);
	const placeholder = resolveStringOrBound(
		component.placeholder as string | BoundValue | undefined,
	);
	const { setByPath } = useData();

	const handleChange = (newValue: string) => {
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}
		if (onAction) {
			onAction({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: newValue },
			});
		}
		void triggerAction(component.actions, { value: newValue });
	};

	return (
		<div
			className={cn("space-y-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && <Label>{label}</Label>}
			<Select
				value={value ?? ""}
				onValueChange={handleChange}
				disabled={disabled}
			>
				<SelectTrigger>
					<SelectValue placeholder={placeholder} />
				</SelectTrigger>
				<SelectContent>
					{options.map((option) => (
						<SelectItem key={option.value} value={option.value}>
							{option.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}
