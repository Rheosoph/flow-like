"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Cloud, Download, XCircle } from "lucide-react";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../components/ui/dialog";
import { ScrollArea } from "../../components/ui/scroll-area";

export type PackageAccessStatus =
	| "installed"
	| "installable"
	| "remote_only"
	| "unavailable";

export interface PackageAvailability {
	packageId: string;
	packageName: string;
	requiredVersion: string;
	status: PackageAccessStatus;
	installedVersion?: string;
	compilationStatus?: string;
	hasUserAccess: boolean;
}

export interface ExecutionPackageCheckProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	packages: PackageAvailability[];
	onExecuteRemotely: () => void;
	onInstallAndRecheck: (packageIds: string[]) => void;
	onCancel: () => void;
	loading?: boolean;
}

const STATUS_CONFIG: Record<
	PackageAccessStatus,
	{ icon: typeof Check; label: string; color: string }
> = {
	installed: {
		icon: Check,
		label: "Local execution",
		color: "text-emerald-600 dark:text-emerald-400",
	},
	installable: {
		icon: Download,
		label: "Installable",
		color: "text-blue-600 dark:text-blue-400",
	},
	remote_only: {
		icon: Cloud,
		label: "Remote execution",
		color: "text-amber-600 dark:text-amber-400",
	},
	unavailable: {
		icon: XCircle,
		label: "Unavailable",
		color: "text-red-600 dark:text-red-400",
	},
};

export function ExecutionPackageCheck({
	open,
	onOpenChange,
	packages,
	onExecuteRemotely,
	onInstallAndRecheck,
	onCancel,
	loading,
}: ExecutionPackageCheckProps) {
	const { t } = useTranslation("flow");
	const installable = packages.filter((p) => p.status === "installable");
	const hasRemote = packages.some((p) => p.status === "remote_only");
	const hasUnavailable = packages.some((p) => p.status === "unavailable");

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t("packageAccessCheck", "Package Access Check")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"thisWorkflowUsesPackagesWithDifferentAccessLevels",
							"This workflow uses packages with different access levels.",
						)}
					</DialogDescription>
				</DialogHeader>
				<ScrollArea className="max-h-72">
					<div className="space-y-2 pr-3">
						{packages.map((pkg) => {
							const config = STATUS_CONFIG[pkg.status];
							const Icon = config.icon;
							return (
								<div
									key={pkg.packageId}
									className="flex items-center gap-3 rounded-md border px-3 py-2"
								>
									<Icon className={`h-4 w-4 shrink-0 ${config.color}`} />
									<div className="flex-1 min-w-0">
										<span className="text-sm font-medium truncate block">
											{pkg.packageName}
										</span>
										<span className="text-xs text-muted-foreground">{`v${pkg.requiredVersion}`}</span>
									</div>
									<Badge
										variant={
											pkg.status === "unavailable" ? "destructive" : "outline"
										}
										className="text-xs shrink-0"
									>
										{config.label}
									</Badge>
								</div>
							);
						})}
					</div>
				</ScrollArea>
				<DialogFooter className="flex-col gap-2 sm:flex-row">
					{installable.length > 0 && (
						<Button
							variant="outline"
							onClick={() =>
								onInstallAndRecheck(installable.map((p) => p.packageId))
							}
							disabled={loading}
						>
							{t("installRecheck", "Install & Re-check")}
						</Button>
					)}
					{hasRemote && !hasUnavailable && (
						<Button onClick={onExecuteRemotely} disabled={loading}>
							{loading ? "Starting..." : "Execute Remotely"}
						</Button>
					)}
					<Button variant="ghost" onClick={onCancel}>
						{t("cancel", "Cancel")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
