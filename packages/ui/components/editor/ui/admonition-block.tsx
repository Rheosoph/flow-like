"use client";

import {
	AlertCircle,
	CheckCircle2,
	Info,
	Lightbulb,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../../lib/utils";

type AdmonitionType = "info" | "warning" | "error" | "success" | "tip";

interface AdmonitionConfig {
	icon: ReactNode;
	label: string;
	borderClass: string;
	bgClass: string;
	iconClass: string;
}

const ADMONITION_CONFIG: Record<AdmonitionType, AdmonitionConfig> = {
	info: {
		icon: <Info className="size-5 shrink-0" />,
		label: "Info",
		borderClass: "border-l-blue-500",
		bgClass: "bg-blue-500/5 dark:bg-blue-500/10",
		iconClass: "text-blue-500",
	},
	warning: {
		icon: <TriangleAlert className="size-5 shrink-0" />,
		label: "Warning",
		borderClass: "border-l-amber-500",
		bgClass: "bg-amber-500/5 dark:bg-amber-500/10",
		iconClass: "text-amber-500",
	},
	error: {
		icon: <AlertCircle className="size-5 shrink-0" />,
		label: "Error",
		borderClass: "border-l-red-500",
		bgClass: "bg-red-500/5 dark:bg-red-500/10",
		iconClass: "text-red-500",
	},
	success: {
		icon: <CheckCircle2 className="size-5 shrink-0" />,
		label: "Success",
		borderClass: "border-l-green-500",
		bgClass: "bg-green-500/5 dark:bg-green-500/10",
		iconClass: "text-green-500",
	},
	tip: {
		icon: <Lightbulb className="size-5 shrink-0" />,
		label: "Tip",
		borderClass: "border-l-purple-500",
		bgClass: "bg-purple-500/5 dark:bg-purple-500/10",
		iconClass: "text-purple-500",
	},
};

function isAdmonitionType(type: string): type is AdmonitionType {
	return type in ADMONITION_CONFIG;
}

function parseAdmonitionContent(raw: string): {
	title: string | null;
	body: string;
} {
	const separatorIndex = raw.indexOf("\n---\n");
	if (separatorIndex !== -1) {
		const title = raw.slice(0, separatorIndex).trim();
		const body = raw.slice(separatorIndex + 5).trim();
		return { title: title || null, body };
	}
	return { title: null, body: raw.trim() };
}

interface AdmonitionBlockProps {
	type: string;
	content: string;
	className?: string;
}

export function AdmonitionBlock({
	type,
	content,
	className,
}: AdmonitionBlockProps) {
	const admonitionType = isAdmonitionType(type) ? type : "info";
	const config = ADMONITION_CONFIG[admonitionType];
	const { title, body } = parseAdmonitionContent(content);
	const displayTitle = title || config.label;

	return (
		<div
			className={cn(
				"my-2 rounded-md border-l-4 p-4",
				config.borderClass,
				config.bgClass,
				className,
			)}
		>
			<div className="flex items-center gap-2 mb-1">
				<span className={config.iconClass}>{config.icon}</span>
				<span className="font-semibold text-sm">{displayTitle}</span>
			</div>
			{body && (
				<div className="ml-7 text-sm leading-relaxed whitespace-pre-wrap">
					{body}
				</div>
			)}
		</div>
	);
}

export default AdmonitionBlock;
