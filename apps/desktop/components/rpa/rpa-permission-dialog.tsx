"use client";

import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	cn,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircle, Check, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

interface RpaPermissionStatus {
	accessibility: boolean;
	executable_path?: string | null;
	screen_recording: boolean;
}

interface RpaPermissionDialogProps {
	open: boolean;
	onContinueAnyway?: () => void;
	onOpenChange: (open: boolean) => void;
	onPermissionsGranted?: () => void;
}

export function RpaPermissionDialog({
	open,
	onContinueAnyway,
	onOpenChange,
	onPermissionsGranted,
}: RpaPermissionDialogProps) {
	const { t } = useTranslation("common");
	const [permissions, setPermissions] = useState<RpaPermissionStatus | null>(
		null,
	);
	const [checking, setChecking] = useState(false);
	const [checkError, setCheckError] = useState<string | null>(null);

	const checkPermissions = useCallback(async () => {
		setChecking(true);
		try {
			const status = await invoke<RpaPermissionStatus>("check_rpa_permissions");
			setPermissions(status);
			setCheckError(null);

			if (status.accessibility && status.screen_recording) {
				onPermissionsGranted?.();
				onOpenChange(false);
			}
		} catch (error) {
			console.error("Failed to check RPA permissions:", error);
			setCheckError(error instanceof Error ? error.message : String(error));
		} finally {
			setChecking(false);
		}
	}, [onOpenChange, onPermissionsGranted]);

	useEffect(() => {
		if (open) {
			checkPermissions();
		}
	}, [open, checkPermissions]);

	useEffect(() => {
		if (!open) return;

		const recheck = () => {
			checkPermissions();
		};
		const recheckWhenVisible = () => {
			if (document.visibilityState === "visible") recheck();
		};

		window.addEventListener("focus", recheck);
		document.addEventListener("visibilitychange", recheckWhenVisible);
		return () => {
			window.removeEventListener("focus", recheck);
			document.removeEventListener("visibilitychange", recheckWhenVisible);
		};
	}, [open, checkPermissions]);

	const requestPermission = async (
		type: "accessibility" | "screen_recording",
	) => {
		try {
			setCheckError(null);
			await invoke("request_rpa_permission", { permissionType: type });
			await new Promise((resolve) => setTimeout(resolve, 500));
			checkPermissions();
		} catch (error) {
			console.error(`Failed to request ${type} permission:`, error);
			setCheckError(error instanceof Error ? error.message : String(error));
		}
	};

	const allGranted =
		permissions?.accessibility && permissions?.screen_recording;
	const canContinueAnyway =
		!allGranted && !checking && !!(permissions || checkError);

	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent className="max-w-md">
				<AlertDialogHeader>
					<AlertDialogTitle className="flex items-center gap-2">
						<ShieldAlert className="h-5 w-5 text-orange-500" />
						{t("permissionsRequired", "Permissions Required")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"localComputerAutomationRequiresSystemPermissionsToControlTheDesktopAndInspectTheScreen",
							"Local computer automation requires system permissions to control the desktop and inspect the screen.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>

				<div className="space-y-3 py-4">
					<PermissionItem
						title={t("accessibilityAccess", "Accessibility Access")}
						description={t(
							"captureUiElementsAndTheirProperties",
							"Capture UI elements and their properties",
						)}
						granted={permissions?.accessibility ?? false}
						onRequest={() => requestPermission("accessibility")}
						checking={checking}
					/>
					<PermissionItem
						title={t("screenRecording", "Screen Recording")}
						description={t(
							"takeScreenshotsOfInteractionAreas",
							"Take screenshots of interaction areas",
						)}
						granted={permissions?.screen_recording ?? false}
						onRequest={() => requestPermission("screen_recording")}
						checking={checking}
					/>
					{permissions &&
					(!permissions.accessibility || !permissions.screen_recording) &&
					permissions.executable_path ? (
						<div className="rounded-md border bg-muted/30 p-3 text-xs text-muted-foreground">
							<p className="font-medium text-foreground">
								{t(
									"macosIsCheckingThisExecutable",
									"macOS is checking this executable:",
								)}
							</p>
							<p className="mt-1 break-all font-mono">
								{permissions.executable_path}
							</p>
							<p className="mt-2">
								{t(
									"ifSystemSettingsShowsAnotherFlowLikeEntryRemoveTheStaleEntryOrAddThisExecutableWithTheButton",
									"If System Settings shows another Flow Like entry, remove the stale entry or add this executable with the + button.",
								)}
							</p>
						</div>
					) : null}
					{checkError ? (
						<div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive">
							<AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
							<div>
								<p className="font-medium">
									{t("permissionCheckFailed", "Permission check failed")}
								</p>
								<p className="mt-1 break-words">{checkError}</p>
							</div>
						</div>
					) : null}
				</div>

				<AlertDialogFooter className="gap-2">
					<button
						type="button"
						onClick={checkPermissions}
						disabled={checking}
						className="rounded-md border px-3 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
					>
						{checking ? "Checking..." : "Re-check"}
					</button>
					{allGranted ? (
						<AlertDialogAction
							onClick={() => {
								onPermissionsGranted?.();
								onOpenChange(false);
							}}
						>
							{t("continue", "Continue")}
						</AlertDialogAction>
					) : canContinueAnyway ? (
						<button
							type="button"
							onClick={() => {
								onContinueAnyway?.();
								onOpenChange(false);
							}}
							className="rounded-md border px-3 py-2 text-sm font-medium hover:bg-accent"
						>
							{t("continueAnyway", "Continue Anyway")}
						</button>
					) : (
						<AlertDialogAction disabled>
							{t("grantPermissionsFirst", "Grant Permissions First")}
						</AlertDialogAction>
					)}
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

interface PermissionItemProps {
	title: string;
	description: string;
	granted: boolean;
	onRequest: () => void;
	checking: boolean;
}

function PermissionItem({
	title,
	description,
	granted,
	onRequest,
	checking,
}: PermissionItemProps) {
	return (
		<div
			className={cn(
				"flex items-center justify-between rounded-lg border p-3 transition-colors",
				granted
					? "border-green-500/30 bg-green-500/10"
					: "border-orange-500/30 bg-orange-500/10",
			)}
		>
			<div className="flex items-center gap-3">
				<div
					className={cn(
						"flex h-8 w-8 items-center justify-center rounded-full",
						granted ? "bg-green-500/20" : "bg-orange-500/20",
					)}
				>
					{granted ? (
						<Check className="h-4 w-4 text-green-500" />
					) : (
						<X className="h-4 w-4 text-orange-500" />
					)}
				</div>
				<div>
					<p className="text-sm font-medium">{title}</p>
					<p className="text-xs text-muted-foreground">{description}</p>
				</div>
			</div>
			{!granted && (
				<button
					type="button"
					onClick={onRequest}
					disabled={checking}
					className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
				>
					{checking ? "Checking..." : "Grant"}
				</button>
			)}
		</div>
	);
}

export function useRpaPermissions() {
	const [hasPermissions, setHasPermissions] = useState<boolean | null>(null);

	const checkPermissions = useCallback(async () => {
		try {
			const status = await invoke<RpaPermissionStatus>("check_rpa_permissions");
			setHasPermissions(status.accessibility && status.screen_recording);
			return status.accessibility && status.screen_recording;
		} catch {
			setHasPermissions(false);
			return false;
		}
	}, []);

	useEffect(() => {
		checkPermissions();
	}, [checkPermissions]);

	return { hasPermissions, checkPermissions };
}
