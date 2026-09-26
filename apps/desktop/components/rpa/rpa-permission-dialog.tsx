"use client";

import {
	AlertDialog,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Button,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircle, Check, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { RpaCapability, RpaPermissionStatus } from "./rpa-consent";

export const capabilityLabels: Record<RpaCapability, string> = {
	browser: "Browser control",
	clipboard: "Clipboard access",
	application_launch: "Launch applications",
	input_control: "Mouse and keyboard control",
	input_monitoring: "Input Monitoring",
	screen_capture: "Screen Recording",
	accessibility: "Accessibility",
	window_management: "Window management",
};
const DEFAULT_REQUIRED: RpaCapability[] = ["input_control", "screen_capture"];

interface RpaPermissionDialogProps {
	open: boolean;
	required?: RpaCapability[];
	onOpenChange: (open: boolean) => void;
	onPermissionsGranted?: () => void;
}

export function RpaPermissionDialog({
	open,
	required = DEFAULT_REQUIRED,
	onOpenChange,
	onPermissionsGranted,
}: RpaPermissionDialogProps) {
	const [permissions, setPermissions] = useState<RpaPermissionStatus | null>(
		null,
	);
	const [checking, setChecking] = useState(false);
	const [checkError, setCheckError] = useState<string | null>(null);
	const generation = useRef(0);
	const finished = useRef(false);
	const callbacks = useRef({ onOpenChange, onPermissionsGranted });
	callbacks.current = { onOpenChange, onPermissionsGranted };
	const requiredKey = JSON.stringify(required);

	const checkPermissions = useCallback(async () => {
		const request = ++generation.current;
		setChecking(true);
		try {
			const status = await invoke<RpaPermissionStatus>(
				"check_rpa_permissions",
				{ required: JSON.parse(requiredKey) },
			);
			if (request !== generation.current || finished.current) return;
			setPermissions(status);
			setCheckError(null);
			if (status.all_granted === true) {
				finished.current = true;
				callbacks.current.onPermissionsGranted?.();
			}
		} catch (error) {
			if (request !== generation.current || finished.current) return;
			setCheckError(error instanceof Error ? error.message : String(error));
		} finally {
			if (request === generation.current) setChecking(false);
		}
	}, [requiredKey]);

	useEffect(() => {
		if (!open) return;
		finished.current = false;
		setPermissions(null);
		void checkPermissions();
		const recheck = () => {
			if (!finished.current) void checkPermissions();
		};
		const visible = () => {
			if (document.visibilityState === "visible") recheck();
		};
		window.addEventListener("focus", recheck);
		document.addEventListener("visibilitychange", visible);
		return () => {
			generation.current++;
			window.removeEventListener("focus", recheck);
			document.removeEventListener("visibilitychange", visible);
		};
	}, [open, checkPermissions]);

	const requestPermission = async (capability: RpaCapability) => {
		const request = ++generation.current;
		setChecking(true);
		setCheckError(null);
		try {
			await invoke("request_rpa_permission", { permissionType: capability });
			if (request !== generation.current || finished.current) return;
			await checkPermissions();
		} catch (error) {
			if (request !== generation.current || finished.current) return;
			setCheckError(error instanceof Error ? error.message : String(error));
			setChecking(false);
		}
	};
	const cancel = () => {
		if (finished.current) return;
		finished.current = true;
		generation.current++;
		onOpenChange(false);
	};

	return (
		<AlertDialog
			open={open}
			onOpenChange={(next) => {
				if (!next) cancel();
			}}
		>
			<AlertDialogContent className="max-w-lg">
				<AlertDialogHeader>
					<AlertDialogTitle className="flex items-center gap-2">
						<ShieldAlert className="h-5 w-5" />
						Automation permissions
					</AlertDialogTitle>
					<AlertDialogDescription>
						Enable the capabilities this workflow needs, then recheck access.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<div className="space-y-3 py-3" aria-live="polite">
					{required.map((capability) => {
						const entry = permissions?.capabilities.find(
							(item) => item.capability === capability,
						);
						const granted =
							entry?.state === "granted" || entry?.state === "not_required";
						return (
							<div key={capability} className="rounded-lg border p-3">
								<div className="flex items-center justify-between gap-3">
									<p className="flex items-center gap-2 font-medium">
										{granted ? (
											<Check className="h-4 w-4 text-green-600" />
										) : (
											<AlertCircle className="h-4 w-4 text-orange-500" />
										)}
										{capabilityLabels[capability]}
									</p>
									{entry?.can_request && (
										<Button
											size="sm"
											disabled={checking}
											onClick={() => void requestPermission(capability)}
										>
											Grant access
										</Button>
									)}
								</div>
								<p className="mt-1 text-xs text-muted-foreground">
									{entry
										? `${entry.state.replaceAll("_", " ")}: ${entry.detail}`
										: "Checking availability…"}
								</p>
							</div>
						);
					})}
					{permissions?.platform === "macos" && permissions.executable_path && (
						<p className="break-all text-xs text-muted-foreground">
							macOS checks this executable:{" "}
							<span className="font-mono">{permissions.executable_path}</span>
						</p>
					)}
					{checkError && (
						<p role="alert" className="text-sm text-destructive">
							Permission check failed: {checkError}
						</p>
					)}
				</div>
				<AlertDialogFooter>
					<Button variant="outline" onClick={cancel}>
						Cancel
					</Button>
					<Button onClick={() => void checkPermissions()} disabled={checking}>
						{checking ? "Checking…" : "Recheck permissions"}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

export function useRpaPermissions(
	required: RpaCapability[] = DEFAULT_REQUIRED,
) {
	const [hasPermissions, setHasPermissions] = useState<boolean | null>(null);
	const requiredKey = JSON.stringify(required);
	const checkPermissions = useCallback(async () => {
		try {
			const status = await invoke<RpaPermissionStatus>(
				"check_rpa_permissions",
				{ required: JSON.parse(requiredKey) },
			);
			setHasPermissions(status.all_granted === true);
			return status.all_granted === true;
		} catch {
			setHasPermissions(false);
			return false;
		}
	}, [requiredKey]);
	useEffect(() => {
		void checkPermissions();
	}, [checkPermissions]);
	return { hasPermissions, checkPermissions };
}
