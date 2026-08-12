"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Children, ModalComponent } from "../types";

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

export function A2UIModal({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
	renderChild,
}: ComponentProps<ModalComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const open = useResolved<boolean>(component.open);
	const title = useResolved<string>(component.title);
	const description = useResolved<string>(component.description);
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

	return (
		<Dialog open={open ?? false} onOpenChange={handleOpenChange}>
			<DialogContent
				className={cn(resolveStyle(style))}
				style={resolveInlineStyle(style)}
			>
				{(title || description) && (
					<DialogHeader>
						{title && <DialogTitle>{title}</DialogTitle>}
						{description && (
							<DialogDescription>{description}</DialogDescription>
						)}
					</DialogHeader>
				)}
				{childIds.length > 0 && (
					<div>
						{childIds.map((id) => (
							<Fragment key={id}>{renderChild(id)}</Fragment>
						))}
					</div>
				)}
			</DialogContent>
		</Dialog>
	);
}
