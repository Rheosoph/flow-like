"use client";

import { useTranslation } from "@flow-like/locales";
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
	const { t } = useTranslation("flow");
	if (mode === "local") return null;

	const isRemote = mode === "remote";
	const Icon = isRemote ? Cloud : Download;
	const label = isRemote
		? packageName
			? t(
					"runsOnServerNoLocalAccessToPackageName",
					"Runs on server — no local access to {{packageName}}",
					{ packageName },
				)
			: t("runsOnServer", "Runs on server")
		: packageName
			? t(
					"installAvailableForPackageName",
					"Install available for {{packageName}}",
					{ packageName },
				)
			: t("installAvailable", "Install available");

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
