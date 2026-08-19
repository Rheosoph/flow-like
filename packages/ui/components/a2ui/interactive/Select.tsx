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
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import type { BoundValue, SelectComponent } from "../types";
import { normalizeOptions, toOptionValue } from "./options";

/** Events added after select shipped never inherit `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

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
	const triggerEvent = useComponentEventTrigger(componentId);
	const [value, setValue] = useBoundInputValue<string>(component.value, "", {
		revision: valueRevisionOf(component),
	});
	const options = normalizeOptions(useResolved<unknown>(component.options));
	const disabled = useResolved<boolean>(component.disabled);
	const label = resolveStringOrBound(
		component.label as string | BoundValue | undefined,
	);
	const placeholder = resolveStringOrBound(
		component.placeholder as string | BoundValue | undefined,
	);

	const handleChange = (newValue: string) => {
		setValue(newValue);
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
		void triggerEvent("change", component, { value: newValue });
	};

	const handleOpenChange = (open: boolean) => {
		void triggerEvent(
			open ? "open" : "close",
			component,
			{ value },
			EXACT_ONLY,
		);
	};

	return (
		<div
			className={cn("space-y-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && <Label>{label}</Label>}
			<Select
				value={toOptionValue(value)}
				onValueChange={handleChange}
				onOpenChange={handleOpenChange}
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
