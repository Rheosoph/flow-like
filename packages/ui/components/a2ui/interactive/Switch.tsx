"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import type { BoundValue, SwitchComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISwitch({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<SwitchComponent>) {
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

	const handleChange = (newChecked: boolean) => {
		setChecked(newChecked);
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
			ref={elementRef}
			className={cn("flex items-center gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Switch
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
