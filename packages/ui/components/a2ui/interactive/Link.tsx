"use client";

import NextLink from "next/link";
import { useRef } from "react";
import { cn } from "../../../lib/utils";
import { useActionContext, useExecuteAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, LinkComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const variantStyles: Record<string, string> = {
	default: "text-foreground hover:text-foreground/80",
	muted: "text-muted-foreground hover:text-muted-foreground/80",
	primary: "text-primary hover:text-primary/80",
	destructive: "text-destructive hover:text-destructive/80",
};

const underlineStyles: Record<string, string> = {
	always: "underline",
	hover: "no-underline hover:underline",
	none: "no-underline",
};

export function A2UILink({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps<LinkComponent>) {
	const pointerActivationAtRef = useRef(0);
	const keyboardActivationAtRef = useRef(0);
	const label = useResolved<string>(component.label) ?? "";
	const href = useResolved<string>(component.href) ?? "";
	const route = useResolved<string>(component.route);
	const queryParams = useResolved<Record<string, string>>(
		component.queryParams,
	);
	const disabled = useResolved<boolean>(component.disabled);
	const { executeAction, isPreviewMode } = useExecuteAction();
	const { appId } = useActionContext();

	const variant = component.variant ?? "primary";
	const underline = component.underline ?? "hover";

	// Check for action defined in ComponentBase.actions
	const action = component.actions?.[0];

	const handleClick = (e: React.MouseEvent) => {
		// Only handle actions in preview mode
		if (!isPreviewMode) return;

		const now = Date.now();
		const hasPointerIntent = now - pointerActivationAtRef.current < 1000;
		const hasKeyboardIntent = now - keyboardActivationAtRef.current < 1000;

		if (!hasPointerIntent && !hasKeyboardIntent) {
			e.preventDefault();
			console.log(
				"[A2UI Link] Ignoring click without local activation intent:",
				{
					componentId,
				},
			);
			return;
		}

		pointerActivationAtRef.current = 0;
		keyboardActivationAtRef.current = 0;

		if (action) {
			e.preventDefault();
			executeAction(action, componentId);
			return;
		}
		if (onAction) {
			onAction({
				type: "userAction",
				name: "navigate",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { href, route, queryParams },
			});
		}
	};

	// Build the resolved href
	let resolvedHref = href;

	if (route && appId) {
		// Build internal navigation URL using query params format
		// Route: /path -> URL: /use?id=appId&route=/path
		const params = new URLSearchParams();
		params.set("id", appId);
		params.set("route", route);

		// Add additional query params if specified
		if (queryParams) {
			for (const [key, value] of Object.entries(queryParams)) {
				params.set(key, value);
			}
		}
		resolvedHref = `/use?${params.toString()}`;
	} else if (route) {
		// Fallback if no appId - use route-only format
		const params = new URLSearchParams();
		params.set("route", route);
		if (queryParams) {
			for (const [key, value] of Object.entries(queryParams)) {
				params.set(key, value);
			}
		}
		resolvedHref = `/use?${params.toString()}`;
	} else if (queryParams && Object.keys(queryParams).length > 0) {
		// External href with query params
		const separator = href.includes("?") ? "&" : "?";
		const params = new URLSearchParams(queryParams);
		resolvedHref = `${href}${separator}${params.toString()}`;
	}

	const baseClasses = cn(
		"inline-flex items-center transition-colors cursor-pointer",
		variantStyles[variant],
		underlineStyles[underline],
		disabled && "pointer-events-none opacity-50",
		resolveStyle(style),
	);

	// If action is defined and in preview mode, render as button-styled element
	if (action && isPreviewMode) {
		return (
			<button
				type="button"
				className={baseClasses}
				style={resolveInlineStyle(style)}
				onPointerDown={() => {
					pointerActivationAtRef.current = Date.now();
				}}
				onKeyDown={(event) => {
					if (event.key === "Enter" || event.key === " ") {
						keyboardActivationAtRef.current = Date.now();
					}
				}}
				onClick={handleClick}
				disabled={disabled}
			>
				{label}
			</button>
		);
	}

	// External links
	if (
		component.external ||
		href.startsWith("http://") ||
		href.startsWith("https://")
	) {
		return (
			<a
				href={resolvedHref}
				target={component.target ?? "_blank"}
				rel="noopener noreferrer"
				className={baseClasses}
				style={resolveInlineStyle(style)}
				onPointerDown={() => {
					pointerActivationAtRef.current = Date.now();
				}}
				onKeyDown={(event) => {
					if (event.key === "Enter" || event.key === " ") {
						keyboardActivationAtRef.current = Date.now();
					}
				}}
				onClick={handleClick}
			>
				{label}
			</a>
		);
	}

	// Internal navigation
	return (
		<NextLink
			href={resolvedHref}
			className={baseClasses}
			style={resolveInlineStyle(style)}
			onPointerDown={() => {
				pointerActivationAtRef.current = Date.now();
			}}
			onKeyDown={(event) => {
				if (event.key === "Enter" || event.key === " ") {
					keyboardActivationAtRef.current = Date.now();
				}
			}}
			onClick={handleClick}
		>
			{label}
		</NextLink>
	);
}
