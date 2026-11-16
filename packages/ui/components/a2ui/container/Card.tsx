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
import { useAssetImage } from "../../../hooks/use-asset-image";
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
import { useAssetUrl } from "../hooks/use-asset-url";
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

/** Edge-to-edge cover picture; keeps its slot while signing and leaves the card when it cannot load. */
function CardMedia({
	source,
	alt,
	zoom,
}: Readonly<{ source: string; alt: string; zoom: boolean }>) {
	const { url, isLoading, refresh } = useAssetUrl(source);
	const image = useAssetImage(url);
	const onError = useCallback(() => {
		image.onError();
		refresh();
	}, [image.onError, refresh]);
	if (!image.canRender && !isLoading) return null;
	return (
		<div
			data-slot="card-media"
			className="relative aspect-video w-full overflow-hidden bg-muted"
		>
			{image.canRender && (
				<img
					ref={image.imgRef}
					src={image.src}
					alt={alt}
					loading="lazy"
					onLoad={image.onLoad}
					onError={onError}
					className={cn(
						"size-full object-cover transition-[opacity,scale] duration-500 ease-out",
						image.loaded ? "opacity-100" : "opacity-0",
						zoom && "motion-safe:group-hover/card:scale-[1.04]",
					)}
				/>
			)}
		</div>
	);
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
	const rawHeaderImage = useResolved<unknown>(component.headerImage);
	const headerImage =
		typeof rawHeaderImage === "string" ? rawHeaderImage.trim() : undefined;
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
				"group/card",
				headerImage && "overflow-hidden pt-0",
				resolveStyle(style),
				hoverable && "hover:shadow-lg motion-safe:hover:-translate-y-0.5",
				clickable && "cursor-pointer",
			)}
			style={resolveInlineStyle(style)}
			onClick={actionMode ? handleClick : undefined}
			onKeyDown={actionMode === "card" ? handleKeyDown : undefined}
			role={actionMode === "card" ? "button" : actionMode ? "group" : undefined}
			aria-labelledby={actionMode === "title" ? titleId : undefined}
			tabIndex={actionMode === "card" ? 0 : undefined}
		>
			{headerImage && (
				<CardMedia
					source={headerImage}
					alt={title ?? ""}
					zoom={Boolean(hoverable || clickable)}
				/>
			)}
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
