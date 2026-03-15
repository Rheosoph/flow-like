"use client";

import { AlertCircle, CheckCircle, Download, Loader2, RefreshCw } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "./badge";

export type CompileStatus =
	| "idle"
	| "downloading"
	| "compiling"
	| "ready"
	| "error"
	| "stale";

type ActiveStatus = Exclude<CompileStatus, "idle">;

interface StatusConfig {
	label: string;
	icon: LucideIcon;
	className: string;
	animate?: string;
}

const STATUS_CONFIG: Record<ActiveStatus, StatusConfig> = {
	downloading: {
		label: "Downloading",
		icon: Download,
		className: "bg-blue-500/10 text-blue-600 border-blue-500/20",
		animate: "animate-bounce",
	},
	compiling: {
		label: "Compiling",
		icon: Loader2,
		className: "bg-amber-500/10 text-amber-600 border-amber-500/20",
		animate: "animate-spin",
	},
	ready: {
		label: "Ready",
		icon: CheckCircle,
		className: "bg-green-500/10 text-green-600 border-green-500/20",
	},
	error: {
		label: "Error",
		icon: AlertCircle,
		className: "bg-destructive/10 text-destructive border-destructive/20",
	},
	stale: {
		label: "Changed",
		icon: RefreshCw,
		className: "bg-orange-500/10 text-orange-600 border-orange-500/20",
	},
};

export function PackageStatusBadge({ status }: { status: CompileStatus }) {
	if (status === "idle") return null;

	const config: StatusConfig = STATUS_CONFIG[status];
	const Icon = config.icon;

	return (
		<Badge
			variant="outline"
			className={`text-[10px] px-1.5 py-0 h-5 rounded-full font-normal gap-1 ${config.className}`}
		>
			<Icon className={`h-2.5 w-2.5 ${config.animate ?? ""}`} />
			{config.label}
		</Badge>
	);
}
