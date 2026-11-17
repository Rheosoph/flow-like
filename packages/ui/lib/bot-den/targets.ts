import { type RefObject, useEffect, useRef } from "react";
import {
	APP_POINTED_AT_KINDS,
	type AppPointedAtKind,
	type IAppPointedAt,
} from "../pointed-at";

/**
 * Element sensitivity for the bot den. Anything a bot can be dropped on either carries
 * `denTargetProps(target)` (plain DOM, so it works in the browser too) or sits inside an element a
 * canvas registered a resolver for, which names what is drawn under the pointer (the board's nodes).
 */
export const DEN_TARGET_ATTRIBUTE = "data-bot-den-target";
export const DEN_KEY_ATTRIBUTE = "data-bot-den-key";
/** Marks what the den must not cover, like the chat overlay: it moves above it or tucks away. */
export const DEN_AVOID_ATTRIBUTE = "data-bot-den-avoid";

export interface DenRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

/** What a drop would point at, and where that is on screen now. */
export interface DenHit {
	target: IAppPointedAt;
	rect: DenRect;
}

/** Names what a canvas draws, for the parts of a surface that are not DOM elements. */
export interface DenTargetResolver {
	/** The target under a viewport point; null lets the element's own target, if any, apply. */
	at(x: number, y: number): DenHit | null;
	/** Where a target this resolver named is now; null when it is gone or out of view. */
	rectOf(target: IAppPointedAt): DenRect | null;
	/** Shows that a bot hovers it; null clears. */
	highlight?(target: IAppPointedAt | null): void;
	/** Makes it the surface's focus before the message goes out, e.g. selects the nodes. */
	focus?(target: IAppPointedAt): void;
}

const KINDS = new Set<string>(APP_POINTED_AT_KINDS);
const resolvers = new Map<Element, DenTargetResolver>();

/** Stable identity of a target: the same thing found again after a re-render or a scroll. */
export function denTargetKey(target: IAppPointedAt): string {
	return [
		target.kind,
		target.app_id,
		target.board_id ?? target.page_id ?? target.widget_id ?? "",
		target.id ?? "",
		...(target.ids ?? []),
	].join(":");
}

/** Spread on an element a bot can be dropped on. */
export function denTargetProps(target: IAppPointedAt): Record<string, string> {
	return {
		[DEN_TARGET_ATTRIBUTE]: JSON.stringify(target),
		[DEN_KEY_ATTRIBUTE]: denTargetKey(target),
	};
}

function optionalText(value: unknown): string | undefined {
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function parseDenTarget(raw: string | null): IAppPointedAt | null {
	if (!raw) return null;
	let value: unknown;
	try {
		value = JSON.parse(raw);
	} catch {
		return null;
	}
	if (typeof value !== "object" || value === null) return null;
	const record = value as Record<string, unknown>;
	const label = optionalText(record.label)?.trim();
	if (
		typeof record.kind !== "string" ||
		!KINDS.has(record.kind) ||
		!optionalText(record.app_id) ||
		!label
	)
		return null;
	const ids = Array.isArray(record.ids)
		? record.ids.filter((id): id is string => typeof id === "string")
		: [];
	return {
		kind: record.kind as AppPointedAtKind,
		app_id: record.app_id as string,
		label,
		...(optionalText(record.id) ? { id: record.id as string } : {}),
		...(ids.length > 0 ? { ids } : {}),
		...(optionalText(record.board_id)
			? { board_id: record.board_id as string }
			: {}),
		...(optionalText(record.page_id)
			? { page_id: record.page_id as string }
			: {}),
		...(optionalText(record.widget_id)
			? { widget_id: record.widget_id as string }
			: {}),
	};
}

function rectOfElement(element: Element): DenRect {
	const box = element.getBoundingClientRect();
	return { x: box.left, y: box.top, width: box.width, height: box.height };
}

export function registerDenResolver(
	element: Element,
	resolver: DenTargetResolver,
): () => void {
	resolvers.set(element, resolver);
	return () => {
		if (resolvers.get(element) === resolver) resolvers.delete(element);
	};
}

/** Registers `resolver` for the element in `ref` while mounted; the latest resolver is always used. */
export function useDenResolver(
	ref: RefObject<Element | null>,
	resolver: DenTargetResolver | null,
): void {
	const latest = useRef(resolver);
	latest.current = resolver;
	const active = resolver !== null;
	useEffect(() => {
		const element = ref.current;
		if (!element || !active) return;
		return registerDenResolver(element, {
			at: (x, y) => latest.current?.at(x, y) ?? null,
			rectOf: (target) => latest.current?.rectOf(target) ?? null,
			highlight: (target) => latest.current?.highlight?.(target),
			focus: (target) => latest.current?.focus?.(target),
		});
	}, [ref, active]);
}

/**
 * The innermost target under a viewport point. Only the topmost element there counts, so a dialog
 * over the board never lets a bot reach the nodes underneath; `skip` passes over the den itself.
 */
export function denHitAt(
	x: number,
	y: number,
	skip: (element: Element) => boolean = () => false,
): DenHit | null {
	if (typeof document === "undefined") return null;
	const top = document
		.elementsFromPoint(x, y)
		.find((element) => !skip(element));
	for (
		let node: Element | null = top ?? null;
		node;
		node = node.parentElement
	) {
		const hit = resolvers.get(node)?.at(x, y);
		if (hit) return hit;
		const target = parseDenTarget(node.getAttribute(DEN_TARGET_ATTRIBUTE));
		if (target) return { target, rect: rectOfElement(node) };
	}
	return null;
}

/** Where a target is on screen now: asked of the resolvers, else found by its key in the DOM. */
export function denTargetRect(target: IAppPointedAt): DenRect | null {
	for (const resolver of resolvers.values()) {
		const rect = resolver.rectOf(target);
		if (rect) return rect;
	}
	if (typeof document === "undefined") return null;
	const element = document.querySelector(
		`[${DEN_KEY_ATTRIBUTE}="${CSS.escape(denTargetKey(target))}"]`,
	);
	return element ? rectOfElement(element) : null;
}

export function highlightDenTarget(target: IAppPointedAt | null): void {
	for (const resolver of resolvers.values()) resolver.highlight?.(target);
}

export function focusDenTarget(target: IAppPointedAt): void {
	for (const resolver of resolvers.values()) resolver.focus?.(target);
}
