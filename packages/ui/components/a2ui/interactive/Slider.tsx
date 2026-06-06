"use client";

import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { Slider } from "../../ui/slider";
import { useComponentActionTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, SliderComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISlider({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<SliderComponent>) {
	const onAction = useOnAction();
	const triggerAction = useComponentActionTrigger(componentId);
	const value = useResolved<number>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const min = useResolved<number>(component.min) ?? 0;
	const max = useResolved<number>(component.max) ?? 100;
	const step = useResolved<number>(component.step) ?? 1;
	const showValue = useResolved<boolean>(component.showValue);
	const { setByPath } = useData();

	const handleChange = (newValues: number[]) => {
		const newValue = newValues[0];
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
			className={cn("space-y-3", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{showValue && (
				<div className="flex justify-between items-center">
					<Label>Value</Label>
					<span className="text-sm text-muted-foreground">{value ?? min}</span>
				</div>
			)}
			<Slider
				value={[value ?? min]}
				min={min}
				max={max}
				step={step}
				disabled={disabled}
				onValueChange={handleChange}
			/>
		</div>
	);
}
