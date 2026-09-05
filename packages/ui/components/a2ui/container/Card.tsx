"use client";

import { Fragment, type KeyboardEvent, type MouseEvent } from "react";
import { cn } from "../../../lib/utils";
import {
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
	Card as ShadCard,
} from "../../ui/card";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveChildSpecs } from "../children";
import type { BoundValue, CardComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UICard({
	elementRef,
	component,
	style,
	componentId,
	renderChild,
}: ComponentProps<CardComponent>) {
	const { resolve } = useData();
	const triggerEvent = useComponentEventTrigger(componentId);
	const title = useResolved<string>(component.title);
	const description = useResolved<string>(component.description);
	const footer = useResolved<string>(component.footer);
	const hoverable = useResolved<boolean>(component.hoverable);
	const clickable = useResolved<boolean>(component.clickable);

	const children = resolveChildSpecs(component.children, resolve);
	const triggerCardAction = () => {
		void triggerEvent("click", component);
	};
	const handleClick = (event: MouseEvent<HTMLDivElement>) => {
		const target = event.target;
		const interactiveTarget =
			target instanceof Element
				? target.closest(
						"button, a, input, textarea, select, label, [role='button'], [role='link'], [data-card-action-stop]",
					)
				: null;
		if (interactiveTarget && interactiveTarget !== event.currentTarget) return;
		triggerCardAction();
	};
	const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
		if (event.target !== event.currentTarget) return;
		if (event.key !== "Enter" && event.key !== " ") return;
		event.preventDefault();
		triggerCardAction();
	};

	return (
		<ShadCard
			ref={elementRef}
			className={cn(
				resolveStyle(style),
				hoverable && "hover:shadow-lg transition-shadow",
				clickable && "cursor-pointer",
			)}
			style={resolveInlineStyle(style)}
			onClick={clickable ? handleClick : undefined}
			onKeyDown={clickable ? handleKeyDown : undefined}
			role={clickable ? "button" : undefined}
			tabIndex={clickable ? 0 : undefined}
		>
			{(title || description) && (
				<CardHeader>
					{title && <CardTitle>{title}</CardTitle>}
					{description && <CardDescription>{description}</CardDescription>}
				</CardHeader>
			)}
			{children.length > 0 && (
				<CardContent>
					{children.map((child) => (
						<Fragment key={child.key}>
							{renderChild(child.id, child.scope)}
						</Fragment>
					))}
				</CardContent>
			)}
			{footer && <CardFooter>{footer}</CardFooter>}
		</ShadCard>
	);
}
