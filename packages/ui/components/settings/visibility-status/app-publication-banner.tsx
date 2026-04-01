"use client";

import { Clock3, ExternalLink, PauseCircle } from "lucide-react";
import { Badge, Button } from "../../ui";
import type { AppPublicationRequestItem } from "./app-publication-review-card";

interface AppPublicationBannerProps {
	requests: AppPublicationRequestItem[];
	onNavigate?: () => void;
}

export function AppPublicationBanner({
	requests,
	onNavigate,
}: Readonly<AppPublicationBannerProps>) {
	const activeRequests = requests.filter(
		(r) => r.status === "pending" || r.status === "on_hold",
	);

	if (activeRequests.length === 0) return null;

	const latest = activeRequests[0];
	const isPending = latest.status === "pending";

	return (
		<div
			className={`flex items-center justify-between gap-3 px-4 py-2.5 rounded-lg border text-sm ${
				isPending
					? "border-blue-500/30 bg-blue-500/5"
					: "border-orange-500/30 bg-orange-500/5"
			}`}
		>
			<div className="flex items-center gap-2 min-w-0">
				{isPending ? (
					<Clock3 className="h-4 w-4 text-blue-500 shrink-0" />
				) : (
					<PauseCircle className="h-4 w-4 text-orange-500 shrink-0" />
				)}
				<span className="truncate">
					Publication request{" "}
					<Badge variant="secondary" className="text-xs mx-1">
						{latest.status.replaceAll("_", " ")}
					</Badge>{" "}
					for{" "}
					<span className="font-medium">
						{latest.targetVisibility.replaceAll("_", " ")}
					</span>
				</span>
			</div>
			{onNavigate && (
				<Button
					variant="ghost"
					size="sm"
					className="shrink-0 gap-1 text-xs"
					onClick={onNavigate}
				>
					View details
					<ExternalLink className="h-3 w-3" />
				</Button>
			)}
		</div>
	);
}
