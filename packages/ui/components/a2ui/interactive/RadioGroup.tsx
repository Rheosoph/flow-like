"use client";

import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../ui/radio-group";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, RadioGroupComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	// Handle literalOptions directly
	if ("literalOptions" in boundValue) {
		return boundValue.literalOptions as T;
	}
	return resolve(boundValue) as T;
}

export function A2UIRadioGroup({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<RadioGroupComponent>) {
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const value = useResolved<string>(component.value);
	const options =
		useResolved<Array<{ value: string; label: string }>>(component.options) ??
		[];
	const disabled = useResolved<boolean>(component.disabled);
	const orientation = useResolved<string>(component.orientation);
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
		void triggerEvent("change", component, { value: newValue });
	};

	const orientationClass =
		orientation === "horizontal" ? "flex-row gap-4" : "flex-col gap-2";

	return (
		<div
			className={cn("space-y-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<RadioGroup
				value={value ?? ""}
				onValueChange={handleChange}
				disabled={disabled}
				className={cn("flex", orientationClass)}
			>
				{options.map((option) => {
					const id = `radio-${componentId}-${option.value}`;
					return (
						<div key={option.value} className="flex items-center gap-2">
							<RadioGroupItem value={option.value} id={id} />
							<Label htmlFor={id} className="cursor-pointer font-normal">
								{option.label}
							</Label>
						</div>
					);
				})}
			</RadioGroup>
		</div>
	);
}
