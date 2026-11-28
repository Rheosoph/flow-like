"use client";

import { useTranslation } from "@flow-like/locales";
import { MoreHorizontalIcon } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "../ui/context-menu";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";

/** The pieces a menu is built from, so one list of items serves a dropdown and a context menu. */
export interface MenuKit {
	Item: ComponentType<{
		onSelect?: (event: Event) => void;
		disabled?: boolean;
		className?: string;
		children?: ReactNode;
	}>;
	Separator: ComponentType;
}

export const DROPDOWN_KIT: MenuKit = {
	Item: DropdownMenuItem,
	Separator: DropdownMenuSeparator,
};

export const CONTEXT_KIT: MenuKit = {
	Item: ContextMenuItem,
	Separator: ContextMenuSeparator,
};

export type MenuItems = (kit: MenuKit) => ReactNode;

/** The "…" button that opens `items` as a dropdown. */
export function OptionsDropdown({
	items,
	className,
}: Readonly<{ items: MenuItems; className?: string }>) {
	const { t } = useTranslation("bots");
	const label = t("crew.conversationMenu.open", "Conversation options");
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className={cn("shrink-0 text-muted-foreground", className)}
					aria-label={label}
					title={label}
				>
					<MoreHorizontalIcon aria-hidden />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end">
				{items(DROPDOWN_KIT)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/** A list row whose right-click, or long press on touch, opens `items`. */
export function RowContextMenu({
	items,
	className,
	children,
}: Readonly<{ items: MenuItems; className?: string; children: ReactNode }>) {
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div className={className}>{children}</div>
			</ContextMenuTrigger>
			<ContextMenuContent className="min-w-48">
				{items(CONTEXT_KIT)}
			</ContextMenuContent>
		</ContextMenu>
	);
}
