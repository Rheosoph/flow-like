"use client";

import { cn } from "../../../lib/utils";
import {
	AccordionContent,
	AccordionItem,
	AccordionTrigger,
	Accordion as ShadAccordion,
} from "../../ui/accordion";
import { useComponentActionTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { AccordionComponent, BoundValue } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIAccordion({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
	renderChild,
}: ComponentProps<AccordionComponent>) {
	const { resolve } = useData();
	const triggerAction = useComponentActionTrigger(componentId);
	const multiple = useResolved<boolean>(component.multiple);
	const defaultExpanded = useResolved<string[]>(component.defaultExpanded);

	const handleChange = (value: string | string[]) => {
		const newValue = Array.isArray(value) ? value : [value];
		if (onAction) {
			onAction({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { expanded: newValue },
			});
		}
		void triggerAction(component.actions, { expanded: newValue });
	};

	const accordionType = multiple ? "multiple" : "single";

	return accordionType === "multiple" ? (
		<ShadAccordion
			type="multiple"
			defaultValue={defaultExpanded ?? []}
			onValueChange={handleChange}
			className={cn(resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{component.items?.map((item) => {
				const title = resolve(item.title) as string | undefined;
				return (
					<AccordionItem key={item.id} value={item.id}>
						<AccordionTrigger>{title ?? item.id}</AccordionTrigger>
						<AccordionContent>
							{renderChild(item.contentComponentId)}
						</AccordionContent>
					</AccordionItem>
				);
			})}
		</ShadAccordion>
	) : (
		<ShadAccordion
			type="single"
			collapsible
			defaultValue={defaultExpanded?.[0]}
			onValueChange={(v) => handleChange([v])}
			className={cn(resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{component.items?.map((item) => {
				const title = resolve(item.title) as string | undefined;
				return (
					<AccordionItem key={item.id} value={item.id}>
						<AccordionTrigger>{title ?? item.id}</AccordionTrigger>
						<AccordionContent>
							{renderChild(item.contentComponentId)}
						</AccordionContent>
					</AccordionItem>
				);
			})}
		</ShadAccordion>
	);
}
