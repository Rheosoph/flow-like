"use client";

import { useTranslation } from "@flow-like/locales";
import Loader2 from "lucide-react/dist/esm/icons/loader-2.js";
import { DynamicIcon, type IconName, iconNames } from "lucide-react/dynamic";
import { useRef } from "react";
import { toast } from "sonner";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import {
	useComponentEventTrigger,
	useIsComponentTriggering,
	useMarkComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { firstEventAction } from "../event-handlers";
import { useActivationSnapshot } from "../hooks/use-activation-snapshot";
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

const lucideIconNames = new Set<string>(iconNames);

function toKebabCase(str: string): string {
	return str
		.trim()
		.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
		.replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
		.replace(/[_\s]+/g, "-")
		.toLowerCase();
}

function LucideIcon({ name, className }: { name: string; className?: string }) {
	const resolvedIconName = toKebabCase(name);
	if (!lucideIconNames.has(resolvedIconName)) return null;
	return (
		<DynamicIcon name={resolvedIconName as IconName} className={className} />
	);
}

export function A2UIButton({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps<ButtonComponent>) {
	const { t } = useTranslation("common");
	const actionInFlightRef = useRef(false);
	const label = useResolved<string>(component.label) ?? "";
	const disabled = useResolved<boolean>(component.disabled);
	const explicitLoading = useResolved<boolean>(component.loading);
	const isTriggering = useIsComponentTriggering(componentId);
	const loading = explicitLoading || isTriggering;
	const variantValue = useResolved<string>(component.variant);
	const sizeValue = useResolved<string>(component.size);
	const icon = useResolved<string>(component.icon);
	const iconPosition = useResolved<string>(component.iconPosition) ?? "left";
	const triggerEvent = useComponentEventTrigger(componentId);
	const markComponentTriggering = useMarkComponentTriggering();

	const variant = variantMap[variantValue ?? "default"] ?? "default";
	const size = sizeMap[sizeValue ?? "md"] ?? "default";

	const activation = useActivationSnapshot<HTMLButtonElement>(async () => {
		if (disabled || loading || actionInFlightRef.current) return;

		const action = firstEventAction(
			component.eventHandlers,
			"click",
			component.actions,
		);
		if (action) {
			actionInFlightRef.current = true;
			// Keep feedback active between the event's ordered actions.
			markComponentTriggering?.(componentId, true);
			try {
				await triggerEvent("click", component);
			} catch (error) {
				console.error("[A2UI Button] Action failed", error);
				toast.error(t("somethingWentWrong"), {
					description: error instanceof Error ? error.message : undefined,
				});
			} finally {
				actionInFlightRef.current = false;
				markComponentTriggering?.(componentId, false);
			}
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
	});

	const showIcon = icon && icon.trim() !== "";
	const iconLeft = iconPosition === "left" && showIcon;
	const iconRight = iconPosition === "right" && showIcon;

	return (
		<Button
			ref={elementRef}
			variant={variant}
			size={size}
			disabled={disabled || loading}
			aria-busy={Boolean(loading)}
			className={cn(loading && "cursor-wait", resolveStyle(style))}
			style={resolveInlineStyle(style)}
			{...activation}
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
