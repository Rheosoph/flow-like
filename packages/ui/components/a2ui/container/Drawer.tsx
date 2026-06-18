"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "../../ui/sheet";
import { useComponentActionTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Children, DrawerComponent } from "../types";

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
	left: "left",
	right: "right",
	top: "top",
	bottom: "bottom",
};

export function A2UIDrawer({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
	renderChild,
}: ComponentProps<DrawerComponent>) {
	const triggerAction = useComponentActionTrigger(componentId);
	const open = useResolved<boolean>(component.open);
	const title = useResolved<string>(component.title);
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
			void triggerAction(component.actions, { open: false });
		}
	};

	const childIds = getChildIds(component.children);
	const resolvedSide = sideMap[side ?? "right"] ?? "right";

	return (
		<Sheet open={open ?? false} onOpenChange={handleOpenChange}>
			<SheetContent
				side={resolvedSide}
				className={cn(resolveStyle(style))}
				style={resolveInlineStyle(style)}
			>
				{title && (
					<SheetHeader>
						<SheetTitle>{title}</SheetTitle>
					</SheetHeader>
				)}
				{childIds.length > 0 && (
					<div className="mt-4">
						{childIds.map((id) => (
							<Fragment key={id}>{renderChild(id)}</Fragment>
						))}
					</div>
				)}
			</SheetContent>
		</Sheet>
	);
}
