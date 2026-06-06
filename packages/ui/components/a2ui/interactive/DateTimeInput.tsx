"use client";

import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { useComponentActionTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
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
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<DateTimeInputComponent>) {
	const onAction = useOnAction();
	const triggerAction = useComponentActionTrigger(componentId);
	const value = useResolved<string>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const mode = useResolved<string>(component.mode) ?? "date";
	const min = useResolved<string>(component.min);
	const max = useResolved<string>(component.max);
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

	const inputType = inputTypeMap[mode] ?? "date";

	return (
		<div
			className={cn("space-y-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Input
				type={inputType}
				value={value ?? ""}
				disabled={disabled}
				min={min}
				max={max}
				onChange={(e) => handleChange(e.target.value)}
			/>
		</div>
	);
}
