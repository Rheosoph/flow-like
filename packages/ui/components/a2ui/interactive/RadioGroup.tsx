"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../ui/radio-group";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import type { BoundValue, RadioGroupComponent } from "../types";
import { normalizeOptions, toOptionValue } from "./options";

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
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const [value, setValue] = useBoundInputValue<string>(component.value, "", {
		revision: valueRevisionOf(component),
	});
	const options = normalizeOptions(useResolved<unknown>(component.options));
	const disabled = useResolved<boolean>(component.disabled);
	const orientation = useResolved<string>(component.orientation);

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

	const orientationClass =
		orientation === "horizontal" ? "flex-row gap-4" : "flex-col gap-2";

	return (
		<div
			className={cn("space-y-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<RadioGroup
				value={toOptionValue(value)}
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
