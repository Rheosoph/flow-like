"use client";

import { DragOverlay } from "@dnd-kit/core";
import { motion } from "framer-motion";
import { memo } from "react";
import { cn } from "../../lib/utils";
import {
	COMPONENT_MOVE_TYPE,
	type DragData,
	useBuilderDnd,
} from "./BuilderDndContext";

// Component type to icon mapping
const COMPONENT_ICONS: Record<string, string> = {
	row: "⬌",
	column: "⬍",
	grid: "⊞",
	stack: "☰",
	text: "T",
	button: "☐",
	image: "🖼",
	card: "▢",
	textField: "⌨",
	select: "▼",
	checkbox: "☑",
	switch: "◐",
};

interface DragLayerProps {
	className?: string;
}

function DragPreview({ data }: { data: DragData }) {
	const getLabel = (): string => {
		if ("componentType" in data) {
			return data.componentType;
		}
		if ("componentId" in data) {
			// Extract type from ID (e.g., "button-12345" -> "button")
			const match = data.componentId.match(/^([a-zA-Z]+)/);
			return match?.[1] || "Component";
		}
		if ("name" in data) {
			return data.name;
		}
		if ("widgetId" in data) {
			return "Widget";
		}
		return "Component";
	};

	const getIcon = (): string => {
		if ("componentType" in data) {
			return COMPONENT_ICONS[data.componentType] || "▢";
		}
		if ("componentId" in data) {
			const match = data.componentId.match(/^([a-zA-Z]+)/);
			const type = match?.[1] || "";
			return COMPONENT_ICONS[type] || "▢";
		}
		if ("widgetId" in data) {
			return "☰";
		}
		return "▢";
	};

	const label = getLabel();
	const icon = getIcon();
	const isMove = data.type === COMPONENT_MOVE_TYPE;

	return (
		<motion.div
			initial={{ scale: 0.9, opacity: 0, y: 4 }}
			animate={{ scale: 1, opacity: 1, y: 0 }}
			className={cn(
				"inline-flex max-w-64 items-center gap-2 px-3 py-2 rounded-lg shadow-xl text-sm font-medium backdrop-blur-sm",
				isMove
					? "bg-blue-500/95 text-white shadow-blue-500/30"
					: "bg-primary/95 text-primary-foreground shadow-primary/30",
			)}
		>
			<span className="shrink-0 text-base">{icon}</span>
			<span className="min-w-0 truncate capitalize" title={label}>
				{label}
			</span>
		</motion.div>
	);
}

export const BuilderDragOverlay = memo(function BuilderDragOverlay({
	className,
}: DragLayerProps) {
	const { activeData } = useBuilderDnd();

	return (
		<DragOverlay
			dropAnimation={{
				duration: 200,
				easing: "cubic-bezier(0.18, 0.67, 0.6, 1.22)",
			}}
			className={cn("z-10000", className)}
		>
			{activeData ? <DragPreview data={activeData} /> : null}
		</DragOverlay>
	);
});
