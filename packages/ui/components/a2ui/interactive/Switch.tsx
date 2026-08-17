"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, SwitchComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISwitch({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<SwitchComponent>) {
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const checked = useResolved<boolean>(component.checked);
	const label = useResolved<string>(component.label);
	const disabled = useResolved<boolean>(component.disabled);
	const { setByPath } = useData();

	const handleChange = (newChecked: boolean) => {
		if (component.checked && "path" in component.checked) {
			setByPath(component.checked.path, newChecked);
		}
		if (onAction) {
			onAction({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { checked: newChecked },
			});
		}
		void triggerEvent("change", component, { checked: newChecked });
	};

	const id = `switch-${componentId}`;

	return (
		<div
			className={cn("flex items-center gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Switch
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
