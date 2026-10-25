"use client";

import {
	Fragment,
	type KeyboardEvent,
	type MouseEvent,
	type RefObject,
	useCallback,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
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

const CONTROL_SELECTOR =
	"a[href], button, input:not([type='hidden']), select, textarea, summary, iframe, [contenteditable]:not([contenteditable='false']), [role='button'], [role='link'], [tabindex]:not([tabindex='-1'])";
const CLICK_STOP_SELECTOR = `${CONTROL_SELECTOR}, a, label, [data-card-action-stop]`;

/**
 * Where a clickable card's action lives for keyboard and screen reader users:
 * on the title button, on the card itself, or nowhere when the card has no
 * title and already holds controls (a button must not contain controls).
 */
type CardActionMode = "title" | "card" | "pointer";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function useContainsControl(
	ref: RefObject<HTMLElement | null>,
	enabled: boolean,
) {
	const [containsControl, setContainsControl] = useState(false);
	useEffect(() => {
		const element = ref.current;
		if (!enabled || !element) return;
		const update = () =>
			setContainsControl(element.querySelector(CONTROL_SELECTOR) !== null);
		update();
		const observer = new MutationObserver(update);
		observer.observe(element, { childList: true, subtree: true });
		return () => observer.disconnect();
	}, [ref, enabled]);
	return enabled && containsControl;
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
	const titleId = useId();
	const cardRef = useRef<HTMLDivElement | null>(null);
	const setCardRef = useCallback(
		(element: HTMLDivElement | null) => {
			cardRef.current = element;
			elementRef?.(element);
		},
		[elementRef],
	);
	const containsControl = useContainsControl(
		cardRef,
		Boolean(clickable && !title),
	);
	const actionMode: CardActionMode | undefined = !clickable
		? undefined
		: title
			? "title"
			: containsControl
				? "pointer"
				: "card";

	const children = resolveChildSpecs(component.children, resolve);
	const triggerCardAction = () => {
		void triggerEvent("click", component);
	};
	const handleClick = (event: MouseEvent<HTMLDivElement>) => {
		const target = event.target;
		const stop =
			target instanceof Element ? target.closest(CLICK_STOP_SELECTOR) : null;
		if (
			stop &&
			stop !== event.currentTarget &&
			event.currentTarget.contains(stop)
		)
			return;
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
			ref={setCardRef}
			className={cn(
				resolveStyle(style),
				hoverable && "hover:shadow-lg transition-shadow",
				clickable && "cursor-pointer",
			)}
			style={resolveInlineStyle(style)}
			onClick={actionMode ? handleClick : undefined}
			onKeyDown={actionMode === "card" ? handleKeyDown : undefined}
			role={actionMode === "card" ? "button" : actionMode ? "group" : undefined}
			aria-labelledby={actionMode === "title" ? titleId : undefined}
			tabIndex={actionMode === "card" ? 0 : undefined}
		>
			{(title || description) && (
				<CardHeader>
					{title && (
						<CardTitle id={titleId}>
							{actionMode === "title" ? (
								<button
									type="button"
									className="cursor-pointer rounded-sm text-start outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
									onClick={triggerCardAction}
								>
									{title}
								</button>
							) : (
								title
							)}
						</CardTitle>
					)}
					{description && <CardDescription>{description}</CardDescription>}
				</CardHeader>
			)}
			{children.length > 0 && (
				<CardContent className="min-h-0 flex-1">
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
