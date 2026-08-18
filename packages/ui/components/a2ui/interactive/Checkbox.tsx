"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Checkbox } from "../../ui/checkbox";
import { Label } from "../../ui/label";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import type { BoundValue, CheckboxComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UICheckbox({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<CheckboxComponent>) {
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const [checked, setChecked] = useBoundInputValue<boolean>(
		component.checked,
		false,
		{ revision: valueRevisionOf(component) },
	);
	const label = useResolved<string>(component.label);
	const disabled = useResolved<boolean>(component.disabled);

	const handleChange = (newChecked: boolean | "indeterminate") => {
		const value = newChecked === true;
		setChecked(value);
		if (onAction) {
			onAction({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { checked: value },
			});
		}
		void triggerEvent("change", component, { checked: value });
	};

	const id = `checkbox-${componentId}`;

	return (
		<div
			className={cn("flex items-center gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Checkbox
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={handleChange}
			/>
			{label && (
				<Label htmlFor={id} className="cursor-pointer">
					{label}
				</Label>
			)}
		</div>
	);
}
