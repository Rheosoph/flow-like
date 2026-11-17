"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AppWindow,
	Box,
	CircleDot,
	Component,
	Folder,
	Globe,
	History,
	Layers,
	LayoutTemplate,
	type LucideIcon,
	MessageSquareText,
	PanelsTopLeft,
	Table,
	Variable,
	Workflow,
	Zap,
} from "lucide-react";
import {
	type AppPointedAtKind,
	type IPointedAt,
	isAppPointedAt,
	pointedAtLabel,
} from "../../../lib/pointed-at";

const APP_KIND_ICONS: Record<AppPointedAtKind, LucideIcon> = {
	app: AppWindow,
	board: Workflow,
	layer: Layers,
	node: Box,
	page: LayoutTemplate,
	widget: PanelsTopLeft,
	component: Component,
	event: Zap,
	table: Table,
	storage: Folder,
	comment: MessageSquareText,
	pin: CircleDot,
	variable: Variable,
	run: History,
};

/** The icon of what a message points at, shared by the chip and the den's drop label. */
export function pointedAtIcon(target: IPointedAt): LucideIcon {
	if (isAppPointedAt(target)) return APP_KIND_ICONS[target.kind];
	return target.kind === "tab" ? Globe : AppWindow;
}

/** What a message was pointed at: a window or tab from the notch, or a thing in Flow-Like from the den. */
export function PointedAtChip({ target }: Readonly<{ target: IPointedAt }>) {
	const { t } = useTranslation("bots");
	const label = pointedAtLabel(target);
	const Icon = pointedAtIcon(target);
	const title = isAppPointedAt(target) ? label : (target.url ?? label);
	return (
		<span
			className="mb-1.5 flex w-fit max-w-full items-center gap-1.5 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
			title={title}
		>
			<Icon className="size-3 shrink-0" aria-hidden />
			<span className="truncate">
				{t("notch.pointedAt", "On {{target}}", { target: label })}
			</span>
		</span>
	);
}
