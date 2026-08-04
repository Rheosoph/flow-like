"use client";

import { cn } from "../../../lib/utils";
import {
	Tabs as ShadTabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../ui/tabs";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, TabsComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UITabs({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
	renderChild,
}: ComponentProps<TabsComponent>) {
	const { resolve, setByPath } = useData();
	const triggerEvent = useComponentEventTrigger(componentId);
	const activeTab = useResolved<string>(component.value);

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

	const defaultValue = activeTab ?? component.tabs?.[0]?.id ?? "";

	return (
		<ShadTabs
			value={activeTab ?? defaultValue}
			onValueChange={handleChange}
			className={cn(resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<TabsList
				className={cn(resolveStyle(component.listStyle))}
				style={resolveInlineStyle(component.listStyle)}
			>
				{component.tabs?.map((tab) => {
					const label = tab.label
						? (resolve(tab.label) as string | undefined)
						: undefined;
					const disabled = tab.disabled
						? (resolve(tab.disabled) as boolean | undefined)
						: undefined;
					return (
						<TabsTrigger
							key={tab.id}
							value={tab.id}
							disabled={disabled}
							className={cn(resolveStyle(component.triggerStyle))}
							style={resolveInlineStyle(component.triggerStyle)}
						>
							{label ?? tab.id}
						</TabsTrigger>
					);
				})}
			</TabsList>
			{component.tabs?.map((tab) => (
				<TabsContent
					key={tab.id}
					value={tab.id}
					className={cn(resolveStyle(component.contentStyle))}
					style={resolveInlineStyle(component.contentStyle)}
				>
					{renderChild(tab.contentComponentId)}
				</TabsContent>
			))}
		</ShadTabs>
	);
}
