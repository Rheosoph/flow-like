"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
} from "../../ui";
import { invalidatePackageLists, useHubProfile } from "./use-workspace-data";

interface LifecycleCardProps {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}

export function PackageRestoreCard({
	packageId,
	fetcher,
	auth,
}: LifecycleCardProps) {
	const { t } = useTranslation("store");
	const queryClient = useQueryClient();
	const profile = useHubProfile();

	const restoreMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data || !packageId) throw new Error("Missing context");
			return fetcher<{ message: string }>(
				profile.data.hub_profile,
				`registry/package/${packageId}/restore`,
				{ method: "POST" },
				auth,
			);
		},
		onSuccess: (data) => {
			toast.success(data.message);
			invalidatePackageLists(queryClient, packageId);
		},
		onError: (err: Error) =>
			toast.error(
				t(
					"failedToRestorePackageMessage",
					"Failed to restore package: {{message}}",
					{
						message: err.message,
					},
				),
			),
	});

	return (
		<Card className="border-primary/30">
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<RotateCcw className="h-4 w-4" />
					{t("packageDisabled", "Package Disabled")}
				</CardTitle>
			</CardHeader>
			<CardContent className="flex items-center justify-between">
				<div>
					<p className="text-sm font-medium">
						{t("restoreThisPackage", "Restore this package")}
					</p>
					<p className="text-sm text-muted-foreground">
						{t(
							"disabledPackageRestoreDescription",
							"This package is currently disabled and hidden from search. Restore it to make it active again.",
						)}
					</p>
				</div>
				<Button
					size="sm"
					className="gap-1.5 shrink-0 ml-4"
					onClick={() => restoreMutation.mutate()}
					disabled={restoreMutation.isPending}
				>
					{restoreMutation.isPending ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<RotateCcw className="h-4 w-4" />
					)}
					{t("restore", "Restore")}
				</Button>
			</CardContent>
		</Card>
	);
}

export function PackageDangerZoneCard({
	packageId,
	packageName,
	fetcher,
	auth,
	onDeleted,
}: LifecycleCardProps & { packageName: string; onDeleted?: () => void }) {
	const { t } = useTranslation("store");
	const profile = useHubProfile();
	const [showDeleteDialog, setShowDeleteDialog] = useState(false);

	const deleteMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data || !packageId) throw new Error("Missing context");
			return fetcher<{ message: string }>(
				profile.data.hub_profile,
				`registry/package/${packageId}`,
				{ method: "DELETE" },
				auth,
			);
		},
		onSuccess: (data) => {
			toast.success(data.message);
			onDeleted?.();
		},
		onError: (err: Error) =>
			toast.error(
				t(
					"failedToDeletePackageMessage",
					"Failed to delete package: {{message}}",
					{
						message: err.message,
					},
				),
			),
	});

	return (
		<Card className="border-destructive/30">
			<CardHeader>
				<CardTitle className="text-base text-destructive">
					{t("dangerZone", "Danger Zone")}
				</CardTitle>
			</CardHeader>
			<CardContent className="flex items-center justify-between">
				<div>
					<p className="text-sm font-medium">
						{t("deleteThisPackage", "Delete this package")}
					</p>
					<p className="text-sm text-muted-foreground">
						{t(
							"deletePackageDescription",
							"The package will be disabled and hidden from search. Existing installs will keep working.",
						)}
					</p>
				</div>
				<AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
					<AlertDialogTrigger asChild>
						<Button
							variant="destructive"
							size="sm"
							className="gap-1.5 shrink-0 ml-4"
						>
							<Trash2 className="h-4 w-4" />
							{t("delete", "Delete")}
						</Button>
					</AlertDialogTrigger>
					<AlertDialogContent>
						<AlertDialogHeader>
							<AlertDialogTitle>
								{t("deletePackage", "Delete package?")}
							</AlertDialogTitle>
							<AlertDialogDescription>
								{t(
									"disablePackageWarning",
									"This will disable {{name}} and remove it from search results. Existing installs and offline projects will continue to work.",
									{ name: packageName },
								)}
							</AlertDialogDescription>
						</AlertDialogHeader>
						<AlertDialogFooter>
							<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
							<AlertDialogAction
								className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
								onClick={() => deleteMutation.mutate()}
								disabled={deleteMutation.isPending}
							>
								{deleteMutation.isPending ? (
									<Loader2 className="mr-2 h-4 w-4 animate-spin" />
								) : (
									<Trash2 className="mr-2 h-4 w-4" />
								)}
								{t("deletePackage2", "Delete Package")}
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			</CardContent>
		</Card>
	);
}
