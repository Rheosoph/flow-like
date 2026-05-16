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
import type { BoundValue, ButtonComponent } from "../types";

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

export function A2UIButton({
	component,
	style,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps<ButtonComponent>) {
	const pointerActivationAtRef = useRef(0);
	const keyboardActivationAtRef = useRef(0);
	const label = useResolved<string>(component.label) ?? "";
	const disabled = useResolved<boolean>(component.disabled);
	const explicitLoading = useResolved<boolean>(component.loading);
	const isTriggering = useIsComponentTriggering(componentId);
	const loading = explicitLoading || isTriggering;
	const variantValue = useResolved<string>(component.variant);
	const sizeValue = useResolved<string>(component.size);
	const icon = useResolved<string>(component.icon);
	const iconPosition = useResolved<string>(component.iconPosition) ?? "left";
	const { executeAction } = useExecuteAction();

	const variant = variantMap[variantValue ?? "default"] ?? "default";
	const size = sizeMap[sizeValue ?? "md"] ?? "default";

	const handleClick = () => {
		const now = Date.now();
		const hasPointerIntent = now - pointerActivationAtRef.current < 1000;
		const hasKeyboardIntent = now - keyboardActivationAtRef.current < 1000;

		if (!hasPointerIntent && !hasKeyboardIntent) {
			console.log(
				"[A2UI Button] Ignoring click without local activation intent:",
				{
					componentId,
				},
			);
			return;
		}

		pointerActivationAtRef.current = 0;
		keyboardActivationAtRef.current = 0;

		const action = component.actions?.[0];
		console.log("[A2UI Button] handleClick:", {
			componentId,
			action,
			hasActions: !!component.actions,
			actionsLength: component.actions?.length,
		});
		if (action) {
			executeAction(action, componentId);
		} else if (onAction) {
			onAction({
				type: "userAction",
				name: "click",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {},
			});
		}
	};

	const showIcon = icon && icon.trim() !== "";
	const iconLeft = iconPosition === "left" && showIcon;
	const iconRight = iconPosition === "right" && showIcon;

	return (
		<Button
			variant={variant}
			size={size}
			disabled={disabled || loading}
			className={cn(loading && "cursor-wait", resolveStyle(style))}
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
			{loading ? (
				<Loader2 className="size-4 animate-spin" />
			) : iconLeft ? (
				<LucideIcon name={icon} className="size-4" />
			) : null}
			{label}
			{!loading && iconRight && <LucideIcon name={icon} className="size-4" />}
		</Button>
	);
}
