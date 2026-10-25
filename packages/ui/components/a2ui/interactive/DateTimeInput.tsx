"use client";

import { useId } from "react";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import type { BoundValue, DateTimeInputComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const inputTypeMap: Record<string, string> = {
	date: "date",
	time: "time",
	datetime: "datetime-local",
};

export function A2UIDateTimeInput({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<DateTimeInputComponent>) {
	const inputId = useId();
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const [value, setValue] = useBoundInputValue<string>(component.value, "", {
		revision: valueRevisionOf(component),
	});
	const disabled = useResolved<boolean>(component.disabled);
	const label = useResolved<string>(component.label);
	const mode = useResolved<string>(component.mode) ?? "date";
	const min = useResolved<string>(component.min);
	const max = useResolved<string>(component.max);

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

	const inputType = inputTypeMap[mode] ?? "date";

	return (
		<div
			ref={elementRef}
			className={cn("space-y-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && <Label htmlFor={inputId}>{label}</Label>}
			<Input
				id={inputId}
				type={inputType}
				value={value}
				disabled={disabled}
				min={min}
				max={max}
				onChange={(e) => handleChange(e.target.value)}
			/>
		</div>
	);
}
