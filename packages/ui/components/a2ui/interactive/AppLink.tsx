"use client";

import * as LucideIcons from "lucide-react";
import { Loader2 } from "lucide-react";
import { useRef } from "react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { useExecuteAction, useIsComponentTriggering } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { AppLinkComponent, BoundValue } from "../types";

const variantMap: Record<
	string,
	"default" | "destructive" | "outline" | "secondary" | "ghost" | "link"
> = {
	primary: "default",
	default: "default",
	secondary: "secondary",
	outline: "outline",
	ghost: "ghost",
	link: "link",
	destructive: "destructive",
};

const sizeMap: Record<string, "default" | "sm" | "lg" | "icon"> = {
	xs: "sm",
	sm: "sm",
	md: "default",
	lg: "lg",
	xl: "lg",
	icon: "icon",
};

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function toPascalCase(str: string): string {
	return str
		.split(/[-_\s]+/)
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
		.join("");
}

function LucideIcon({ name, className }: { name: string; className?: string }) {
	const IconComp = (LucideIcons as Record<string, unknown>)[
		toPascalCase(name)
	] as React.ComponentType<{ className?: string }> | undefined;
	if (!IconComp) return null;
	return <IconComp className={className} />;
}

export function A2UIAppLink({
	component,
	style,
	componentId,
}: ComponentProps<AppLinkComponent>) {
	const pointerActivationAtRef = useRef(0);
	const keyboardActivationAtRef = useRef(0);
	const { executeAction } = useExecuteAction();
	const isTriggering = useIsComponentTriggering(componentId);

	const target = useResolved<string>(component.target) ?? "config";
	const resolvedLabel = useResolved<string>(component.label);
	const label =
		resolvedLabel && resolvedLabel.trim()
			? resolvedLabel
			: target === "overview"
				? "Overview"
				: target === "settings"
					? "Settings"
					: "Configure";
	const variantValue = useResolved<string>(component.variant) ?? "outline";
	const sizeValue = useResolved<string>(component.size) ?? "sm";
	const disabled = useResolved<boolean>(component.disabled) ?? false;
	const icon =
		useResolved<string>(component.icon) ??
		(target === "overview" ? "info" : "settings");
	const iconPosition = useResolved<string>(component.iconPosition) ?? "left";
	const contextAppId = useResolved<string>(component.appId);
	const contextEventId = useResolved<string>(component.eventId);

	const variant = variantMap[variantValue] ?? "outline";
	const size = sizeMap[sizeValue] ?? "sm";
	const actionName =
		target === "overview" ? "navigate_app_overview" : "navigate_app_config";

	const handleClick = () => {
		const now = Date.now();
		const hasPointerIntent = now - pointerActivationAtRef.current < 1000;
		const hasKeyboardIntent = now - keyboardActivationAtRef.current < 1000;
		if (!hasPointerIntent && !hasKeyboardIntent) return;

		pointerActivationAtRef.current = 0;
		keyboardActivationAtRef.current = 0;

		executeAction(
			{
				name: actionName,
				context: {
					appId: contextAppId,
					eventId: contextEventId,
				},
			},
			componentId,
		);
	};

	const showIcon = icon && icon.trim() !== "";
	const iconLeft = iconPosition === "left" && showIcon;
	const iconRight = iconPosition === "right" && showIcon;

	return (
		<Button
			type="button"
			variant={variant}
			size={size}
			disabled={disabled || isTriggering}
			className={cn(isTriggering && "cursor-wait", resolveStyle(style))}
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
			{isTriggering ? (
				<Loader2 className="size-4 animate-spin" />
			) : iconLeft ? (
				<LucideIcon name={icon} className="size-4" />
			) : null}
			{label}
			{!isTriggering && iconRight && (
				<LucideIcon name={icon} className="size-4" />
			)}
		</Button>
	);
}
