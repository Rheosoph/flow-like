"use client";

import { cn } from "../../../lib/utils";
import { Checkbox } from "../../ui/checkbox";
import { Label } from "../../ui/label";
import { useComponentActionTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
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
	const onAction = useOnAction();
	const triggerAction = useComponentActionTrigger(componentId);
	const checked = useResolved<boolean>(component.checked);
	const label = useResolved<string>(component.label);
	const disabled = useResolved<boolean>(component.disabled);
	const { setByPath } = useData();

	const handleChange = (newChecked: boolean | "indeterminate") => {
		const value = newChecked === true;
		if (component.checked && "path" in component.checked) {
			setByPath(component.checked.path, value);
		}
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
		void triggerAction(component.actions, { checked: value });
	};

	const id = `checkbox-${componentId}`;

	return (
		<div
			className={cn("flex items-center gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Checkbox
				id={id}
				checked={checked ?? false}
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
