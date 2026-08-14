"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { cn } from "../../lib";

export interface ComponentPreviewProps {
	componentType: string;
	className?: string;
	size?: "sm" | "md" | "lg";
}

const COMPONENT_ICONS: Record<string, string> = {
	row: "↔",
	column: "↕",
	stack: "☐",
	grid: "▦",
	scrollArea: "⊞",
	aspectRatio: "▭",
	overlay: "◫",
	absolute: "◱",
	text: "T",
	image: "🖼",
	icon: "★",
	video: "▷",
	lottie: "◌",
	markdown: "M↓",
	divider: "—",
	badge: "○",
	avatar: "◉",
	userProfile: "UP",
	progress: "▰",
	spinner: "↻",
	skeleton: "▯",
	button: "◯",
	textField: "▢",
	richText: "¶",
	select: "▾",
	slider: "━",
	checkbox: "☑",
	switch: "◐",
	radioGroup: "◎",
	dateTimeInput: "📅",
	card: "▢",
	modal: "◳",
	tabs: "⊟",
	accordion: "⊞",
	drawer: "◨",
	tooltip: "💬",
	popover: "◰",
	canvas2D: "🎨",
	sprite: "👾",
	shape: "◆",
	scene3D: "🧊",
	model3D: "📦",
	dialogue: "💭",
	characterPortrait: "🧑",
	choiceMenu: "☰",
	inventoryGrid: "🎒",
	healthBar: "❤️",
	miniMap: "🗺",
	geoMap: "🌍",
	graph: "🕸",
	ontologyGraph: "🕸",
};

const COMPONENT_COLORS: Record<
	string,
	{ bg: string; border: string; text: string }
> = {
	// Layout - Blue
	row: { bg: "bg-blue-50", border: "border-blue-200", text: "text-blue-600" },
	column: {
		bg: "bg-blue-50",
		border: "border-blue-200",
		text: "text-blue-600",
	},
	stack: { bg: "bg-blue-50", border: "border-blue-200", text: "text-blue-600" },
	grid: { bg: "bg-blue-50", border: "border-blue-200", text: "text-blue-600" },
	scrollArea: {
		bg: "bg-blue-50",
		border: "border-blue-200",
		text: "text-blue-600",
	},
	aspectRatio: {
		bg: "bg-blue-50",
		border: "border-blue-200",
		text: "text-blue-600",
	},
	overlay: {
		bg: "bg-blue-50",
		border: "border-blue-200",
		text: "text-blue-600",
	},
	absolute: {
		bg: "bg-blue-50",
		border: "border-blue-200",
		text: "text-blue-600",
	},

	// Display - Green
	text: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	image: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	icon: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	video: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	lottie: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	markdown: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	divider: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	badge: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	avatar: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	userProfile: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	progress: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	spinner: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	skeleton: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},

	// Interactive - Purple
	button: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	textField: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	richText: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	select: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	slider: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	checkbox: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	switch: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	radioGroup: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},
	dateTimeInput: {
		bg: "bg-purple-50",
		border: "border-purple-200",
		text: "text-purple-600",
	},

	// Container - Orange
	card: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	modal: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	tabs: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	accordion: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	drawer: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	tooltip: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},
	popover: {
		bg: "bg-orange-50",
		border: "border-orange-200",
		text: "text-orange-600",
	},

	// Game - Pink/Rose
	canvas2D: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	sprite: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	shape: { bg: "bg-rose-50", border: "border-rose-200", text: "text-rose-600" },
	scene3D: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	model3D: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	dialogue: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	characterPortrait: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	choiceMenu: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	inventoryGrid: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	healthBar: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	miniMap: {
		bg: "bg-rose-50",
		border: "border-rose-200",
		text: "text-rose-600",
	},
	geoMap: {
		bg: "bg-green-50",
		border: "border-green-200",
		text: "text-green-600",
	},
	graph: {
		bg: "bg-indigo-50",
		border: "border-indigo-200",
		text: "text-indigo-600",
	},
	ontologyGraph: {
		bg: "bg-indigo-50",
		border: "border-indigo-200",
		text: "text-indigo-600",
	},
};

const DEFAULT_COLORS = {
	bg: "bg-slate-50",
	border: "border-slate-200",
	text: "text-slate-600",
};

export function ComponentPreview({
	componentType,
	className,
	size = "md",
}: ComponentPreviewProps) {
	const { t } = useTranslation("flow");
	const icon = COMPONENT_ICONS[componentType] || "?";
	const colors = COMPONENT_COLORS[componentType] || DEFAULT_COLORS;

	const sizeClasses = useMemo(() => {
		switch (size) {
			case "sm":
				return `w-8 h-8 text-sm`;
			case "lg":
				return `w-16 h-16 text-2xl`;
			default:
				return `w-12 h-12 text-lg`;
		}
	}, [size]);

	return (
		<div
			className={cn(
				"flex items-center justify-center rounded border-2",
				colors.bg,
				colors.border,
				colors.text,
				sizeClasses,
				className,
			)}
			title={componentType}
		>
			{icon}
		</div>
	);
}

export function getComponentIcon(componentType: string): string {
	return COMPONENT_ICONS[componentType] || "?";
}

export function getComponentColors(componentType: string) {
	return COMPONENT_COLORS[componentType] || DEFAULT_COLORS;
}
