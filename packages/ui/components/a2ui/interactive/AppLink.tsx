"use client";

import { Loader2 } from "lucide-react";
import { DynamicIcon, type IconName } from "lucide-react/dynamic";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { useExecuteAction, useIsComponentTriggering } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useActivationSnapshot } from "../hooks/use-activation-snapshot";
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

function toKebabCase(str: string): string {
	return str
		.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
		.replace(/[\s_]+/g, "-")
		.toLowerCase();
}

function LucideIcon({ name, className }: { name: string; className?: string }) {
	return (
		<DynamicIcon
			name={toKebabCase(name) as IconName}
			className={className}
			fallback={() => null}
		/>
	);
}

export function A2UIAppLink({
	elementRef,
	component,
	style,
	componentId,
}: ComponentProps<AppLinkComponent>) {
	const { executeAction } = useExecuteAction();
	const isTriggering = useIsComponentTriggering(componentId);

	const target = useResolved<string>(component.target) ?? "config";
	const resolvedLabel = useResolved<string>(component.label);
	const label = resolvedLabel?.trim()
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

	const activation = useActivationSnapshot<HTMLButtonElement>(() => {
		void executeAction(
			{
				name: actionName,
				context: {
					appId: contextAppId,
					eventId: contextEventId,
				},
			},
			componentId,
		);
	});

	const showIcon = icon && icon.trim() !== "";
	const iconLeft = iconPosition === "left" && showIcon;
	const iconRight = iconPosition === "right" && showIcon;

	return (
		<Button
			ref={elementRef}
			type="button"
			variant={variant}
			size={size}
			disabled={disabled || isTriggering}
			className={cn(isTriggering && "cursor-wait", resolveStyle(style))}
			style={resolveInlineStyle(style)}
			{...activation}
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
