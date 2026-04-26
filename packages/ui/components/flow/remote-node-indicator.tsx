"use client";

import { Cloud, Download } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export type NodeAccessMode = "local" | "remote" | "installable";

export interface RemoteNodeIndicatorProps {
	mode: NodeAccessMode;
	packageName?: string;
	className?: string;
}

export function RemoteNodeIndicator({
	mode,
	packageName,
	className,
}: RemoteNodeIndicatorProps) {
	if (mode === "local") return null;

	const isRemote = mode === "remote";
	const Icon = isRemote ? Cloud : Download;
	const label = isRemote
		? `Runs on server${packageName ? ` — no local access to ${packageName}` : ""}`
		: `Install available${packageName ? ` for ${packageName}` : ""}`;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<div
					className={`absolute -top-1.5 -right-1.5 flex h-5 w-5 items-center justify-center rounded-full ${
						isRemote
							? "bg-blue-100 text-blue-600 dark:bg-blue-900 dark:text-blue-300"
							: "bg-emerald-100 text-emerald-600 dark:bg-emerald-900 dark:text-emerald-300"
					} ${className ?? ""}`}
				>
					<Icon className="h-3 w-3" />
				</div>
			</TooltipTrigger>
			<TooltipContent side="top">
				<p className="text-xs">{label}</p>
			</TooltipContent>
		</Tooltip>
	);
}
