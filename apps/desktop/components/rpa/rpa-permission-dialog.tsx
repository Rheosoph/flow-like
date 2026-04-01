"use client";

import { invoke } from "@tauri-apps/api/core";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	cn,
} from "@tm9657/flow-like-ui";
import { Check, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

interface RpaPermissionStatus {
	accessibility: boolean;
	screen_recording: boolean;
}

interface RpaPermissionDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onPermissionsGranted?: () => void;
}

export function RpaPermissionDialog({
	open,
	onOpenChange,
	onPermissionsGranted,
}: RpaPermissionDialogProps) {
	const [permissions, setPermissions] = useState<RpaPermissionStatus | null>(
		null,
	);
	const [checking, setChecking] = useState(false);

	const checkPermissions = useCallback(async () => {
		setChecking(true);
		try {
			const status = await invoke<RpaPermissionStatus>("check_rpa_permissions");
			setPermissions(status);

			if (status.accessibility && status.screen_recording) {
				onPermissionsGranted?.();
				onOpenChange(false);
			}
		} catch (error) {
			console.error("Failed to check RPA permissions:", error);
		} finally {
			setChecking(false);
		}
	}, [onOpenChange, onPermissionsGranted]);

	useEffect(() => {
		if (open) {
			checkPermissions();
		}
	}, [open, checkPermissions]);

	const requestPermission = async (
		type: "accessibility" | "screen_recording",
	) => {
		try {
			await invoke("request_rpa_permission", { permissionType: type });
			await new Promise((resolve) => setTimeout(resolve, 500));
			checkPermissions();
		} catch (error) {
			console.error(`Failed to request ${type} permission:`, error);
		}
	};

	const allGranted =
		permissions?.accessibility && permissions?.screen_recording;

	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent className="max-w-md">
				<AlertDialogHeader>
					<AlertDialogTitle className="flex items-center gap-2">
						<ShieldAlert className="h-5 w-5 text-orange-500" />
						Permissions Required
					</AlertDialogTitle>
					<AlertDialogDescription>
						RPA workflow recording requires system permissions to capture your
						interactions. These permissions allow the app to:
					</AlertDialogDescription>
				</AlertDialogHeader>

				<div className="space-y-3 py-4">
					<PermissionItem
						title="Accessibility Access"
						description="Capture UI elements and their properties"
						granted={permissions?.accessibility ?? false}
						onRequest={() => requestPermission("accessibility")}
						checking={checking}
					/>
					<PermissionItem
						title="Screen Recording"
						description="Take screenshots of interaction areas"
						granted={permissions?.screen_recording ?? false}
						onRequest={() => requestPermission("screen_recording")}
						checking={checking}
					/>
				</div>

				<AlertDialogFooter>
					<AlertDialogAction
						onClick={() => {
							if (allGranted) {
								onPermissionsGranted?.();
							}
							onOpenChange(false);
						}}
						disabled={!allGranted}
					>
						{allGranted ? "Start Recording" : "Grant Permissions First"}
					</AlertDialogAction>
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
