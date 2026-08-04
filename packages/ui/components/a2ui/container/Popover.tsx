"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import {
	PopoverContent,
	PopoverTrigger,
	Popover as ShadPopover,
} from "../../ui/popover";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Children, PopoverComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function getChildIds(children: Children | undefined): string[] {
	if (!children) return [];
	if ("explicitList" in children) return children.explicitList;
	return [];
}

const sideMap: Record<string, "top" | "bottom" | "left" | "right"> = {
	top: "top",
	bottom: "bottom",
	left: "left",
	right: "right",
};

export function A2UIPopover({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
	renderChild,
}: ComponentProps<PopoverComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const open = useResolved<boolean>(component.open);
	const side = useResolved<string>(component.side);
	const { setByPath } = useData();

	const handleOpenChange = (newOpen: boolean) => {
		if (component.open && "path" in component.open) {
			setByPath(component.open.path, newOpen);
		}
		if (!newOpen && onAction) {
			onAction({
				type: "userAction",
				name: "close",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { open: false },
			});
			void triggerEvent("close", component, { open: false });
		}
	};

	const childIds = getChildIds(component.children);
	const resolvedSide = sideMap[side ?? "bottom"] ?? "bottom";

	return (
		<ShadPopover open={open} onOpenChange={handleOpenChange}>
			<PopoverTrigger asChild>
				<span
					className={cn("inline-block cursor-pointer", resolveStyle(style))}
					style={resolveInlineStyle(style)}
				>
					{childIds.map((id) => (
						<Fragment key={id}>{renderChild(id)}</Fragment>
					))}
				</span>
			</PopoverTrigger>
			<PopoverContent side={resolvedSide}>
				{renderChild(component.contentComponentId)}
			</PopoverContent>
		</ShadPopover>
	);
}
