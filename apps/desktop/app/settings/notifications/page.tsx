"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Skeleton,
	Switch,
	cn,
	useBackend,
	useHub,
} from "@flow-like/flow-like-ui";
import type { IPushTargetStatus } from "@flow-like/flow-like-ui";
import {
	AlertTriangle,
	Bell,
	BellOff,
	CheckCircle2,
	RefreshCw,
	Server,
	ShieldCheck,
	Smartphone,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	canUseRemotePushForPlatform,
	detectPushPlatform,
	getPushDeviceId,
	isRemotePushPreferenceEnabled,
	loadRemotePushPlugin,
	setRemotePushPreference,
	type PushTargetPlatform,
} from "../../../lib/remote-push";

type PluginState = "loading" | "available" | "unavailable";
type PermissionState = "unknown" | "granted" | "denied" | "unavailable";

function platformLabel(platform: PushTargetPlatform | null): string {
	switch (platform) {
		case "IOS":
			return "iOS";
		case "ANDROID":
			return "Android";
		case "DESKTOP":
			return "Desktop";
		default:
			return "Unknown";
	}
}

function formatDate(value?: string | null): string {
	if (!value) return "Never";
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(date);
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function statusBadge(status: IPushTargetStatus | null, localEnabled: boolean) {
	if (!localEnabled) {
		return { label: "Off on this device", variant: "secondary" as const };
	}
	if (!status?.registered) {
		return { label: "Not registered", variant: "outline" as const };
	}
	if (status.push_enabled && !status.invalidated_at) {
		return { label: "Enabled", variant: "default" as const };
	}
	return { label: "Disabled", variant: "destructive" as const };
}

function StatusRow({
	label,
	value,
	muted,
}: Readonly<{ label: string; value: React.ReactNode; muted?: boolean }>) {
	return (
		<div className="flex min-h-9 items-center justify-between gap-4 border-b border-border/50 py-2 last:border-0">
			<div className="text-sm text-muted-foreground">{label}</div>
			<div
				className={cn(
					"min-w-0 text-right text-sm font-medium text-foreground",
					muted && "text-muted-foreground font-normal",
				)}
			>
				{value}
			</div>
		</div>
	);
}

export default function NotificationsSettingsPage() {
	const backend = useBackend();
	const hub = useHub();
	const pushConfig = hub.hub?.push_notifications;
	const [platform, setPlatform] = useState<PushTargetPlatform | null>(null);
	const [pluginState, setPluginState] = useState<PluginState>("loading");
	const [permissionState, setPermissionState] =
		useState<PermissionState>("unknown");
	const [deviceId, setDeviceId] = useState<string | null>(null);
	const [status, setStatus] = useState<IPushTargetStatus | null>(null);
	const [localEnabled, setLocalEnabled] = useState(true);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [loadError, setLoadError] = useState<string | null>(null);

	const canUseRemotePush = useMemo(
		() => canUseRemotePushForPlatform(pushConfig, platform),
		[pushConfig, platform],
	);
	const isMobile = platform === "IOS" || platform === "ANDROID";
	const badge = statusBadge(status, localEnabled);
	const deliveryEnabled =
		localEnabled &&
		Boolean(status?.registered && status.push_enabled && !status.invalidated_at);

	const refreshStatus = useCallback(async () => {
		setLoading(true);
		setLoadError(null);
		try {
			const nextPlatform = detectPushPlatform();
			setPlatform(nextPlatform);
			setLocalEnabled(isRemotePushPreferenceEnabled());

			const [nextDeviceId, remotePushApi] = await Promise.all([
				getPushDeviceId(),
				loadRemotePushPlugin(),
			]);
			setDeviceId(nextDeviceId);
			setPluginState(remotePushApi ? "available" : "unavailable");
			if (!remotePushApi) {
				setPermissionState("unavailable");
			}

			const nextStatus =
				await backend.userState.getPushTargetStatus(nextDeviceId);
			setStatus(nextStatus);
		} catch (error) {
			setLoadError(errorMessage(error));
			setStatus(null);
		} finally {
			setLoading(false);
		}
	}, [backend.userState]);

	useEffect(() => {
		void refreshStatus();
	}, [refreshStatus]);

	const setEnabled = useCallback(
		async (enabled: boolean) => {
			if (!deviceId) return;

			setSaving(true);
			try {
				if (enabled) {
					const nextPlatform = detectPushPlatform();
					if (!nextPlatform) {
						throw new Error("Push platform could not be detected.");
					}
					if (!canUseRemotePushForPlatform(pushConfig, nextPlatform)) {
						throw new Error("Remote push is not enabled for this platform.");
					}

					const remotePushApi = await loadRemotePushPlugin();
					if (!remotePushApi) {
						throw new Error("Remote push plugin is not available.");
					}

					const permission = await remotePushApi.requestPermission();
					if (!permission.granted) {
						setPermissionState("denied");
						throw new Error("Notification permission was denied.");
					}
					setPermissionState("granted");

					const token = await remotePushApi.getToken();
					if (!token) {
						throw new Error("Remote push returned an empty token.");
					}

					await backend.userState.registerPushTarget({
						device_id: deviceId,
						platform: nextPlatform,
						token,
						device_name:
							typeof navigator !== "undefined"
								? navigator.userAgent
								: undefined,
						channel_id: pushConfig?.channel_id ?? undefined,
						metadata: {
							source: "settings",
							platform: nextPlatform,
							provider: pushConfig?.provider,
						},
					});
				}

				const nextStatus = await backend.userState.setPushTargetEnabled(
					deviceId,
					enabled,
				);
				setRemotePushPreference(enabled);
				setLocalEnabled(enabled);
				setStatus(nextStatus);
				toast.success(
					enabled ? "Push notifications enabled" : "Push notifications disabled",
				);
			} catch (error) {
				toast.error(`Failed to update push notifications: ${errorMessage(error)}`);
			} finally {
				setSaving(false);
			}
		},
		[backend.userState, deviceId, pushConfig],
	);

	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="container mx-auto flex max-w-5xl flex-col gap-6 px-2 pb-4">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">Notifications</h1>
					<p className="text-muted-foreground">
						Mobile push delivery for this device and hub profile
					</p>
				</div>

				<div className="grid gap-4 lg:grid-cols-[1.15fr_0.85fr]">
					<Card>
						<CardHeader className="gap-3 sm:flex sm:flex-row sm:items-start sm:justify-between">
							<div className="space-y-1.5">
								<CardTitle className="flex items-center gap-2 text-xl">
									{deliveryEnabled ? (
										<Bell className="h-5 w-5 text-primary" />
									) : (
										<BellOff className="h-5 w-5 text-muted-foreground" />
									)}
									Push delivery
								</CardTitle>
								<CardDescription>
									{isMobile
										? "Current native push target state"
										: "Native mobile push is available on iOS and Android"}
								</CardDescription>
							</div>
							<Badge variant={badge.variant}>{badge.label}</Badge>
						</CardHeader>
						<CardContent className="flex flex-col gap-5">
							<div className="flex items-center justify-between gap-4 rounded-md border border-border/60 bg-muted/20 p-4">
								<div className="min-w-0">
									<div className="font-medium">Enable push notifications</div>
									<div className="text-sm text-muted-foreground">
										{status?.registered
											? "Registered target controls server delivery"
											: "Creates a server target after native permission"}
									</div>
								</div>
								<Switch
									checked={deliveryEnabled}
									disabled={
										saving ||
										loading ||
										!isMobile ||
										!canUseRemotePush ||
										pluginState !== "available" ||
										!deviceId
									}
									onCheckedChange={setEnabled}
								/>
							</div>

							{loadError && (
								<div className="flex gap-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
									<AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
									<span>{loadError}</span>
								</div>
							)}

							<div className="grid gap-3 sm:grid-cols-2">
								<StatusTile
									icon={Smartphone}
									label="Platform"
									value={platformLabel(platform)}
									ok={isMobile}
								/>
								<StatusTile
									icon={Server}
									label="Hub config"
									value={canUseRemotePush ? "Enabled" : "Unavailable"}
									ok={canUseRemotePush}
								/>
								<StatusTile
									icon={ShieldCheck}
									label="Native plugin"
									value={
										pluginState === "loading"
											? "Checking"
											: pluginState === "available"
												? "Available"
												: "Unavailable"
									}
									ok={pluginState === "available"}
								/>
								<StatusTile
									icon={CheckCircle2}
									label="Permission"
									value={
										permissionState === "granted"
											? "Granted"
											: permissionState === "denied"
												? "Denied"
												: permissionState === "unavailable"
													? "Unavailable"
													: "Not checked"
									}
									ok={permissionState === "granted"}
								/>
							</div>
						</CardContent>
					</Card>

					<Card>
						<CardHeader className="gap-3 sm:flex sm:flex-row sm:items-start sm:justify-between">
							<div className="space-y-1.5">
								<CardTitle className="text-xl">Target details</CardTitle>
								<CardDescription>Server row for this device</CardDescription>
							</div>
							<Button variant="outline" size="sm" onClick={refreshStatus}>
								<RefreshCw className="h-4 w-4" />
								Refresh
							</Button>
						</CardHeader>
						<CardContent>
							{loading ? (
								<div className="space-y-3">
									<Skeleton className="h-8 w-full" />
									<Skeleton className="h-8 w-full" />
									<Skeleton className="h-8 w-full" />
									<Skeleton className="h-8 w-full" />
								</div>
							) : (
								<div className="flex flex-col">
									<StatusRow
										label="Device ID"
										value={
											deviceId ? (
												<span className="break-all font-mono text-xs">
													{deviceId}
												</span>
											) : (
												"Unknown"
											)
										}
										muted={!deviceId}
									/>
									<StatusRow
										label="Registered"
										value={status?.registered ? "Yes" : "No"}
										muted={!status?.registered}
									/>
									<StatusRow
										label="Provider"
										value={status?.provider ?? pushConfig?.provider ?? "None"}
										muted={!status?.provider && !pushConfig?.provider}
									/>
									<StatusRow
										label="Server enabled"
										value={status?.push_enabled ? "Yes" : "No"}
										muted={!status?.push_enabled}
									/>
									<StatusRow
										label="Failure count"
										value={status?.failure_count ?? 0}
									/>
									<StatusRow
										label="Last registered"
										value={formatDate(status?.last_registered_at)}
										muted={!status?.last_registered_at}
									/>
									<StatusRow
										label="Last seen"
										value={formatDate(status?.last_seen_at)}
										muted={!status?.last_seen_at}
									/>
									<StatusRow
										label="Invalidated"
										value={formatDate(status?.invalidated_at)}
										muted={!status?.invalidated_at}
									/>
									<StatusRow
										label="Reason"
										value={status?.invalidation_reason ?? "None"}
										muted={!status?.invalidation_reason}
									/>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</div>
		</div>
	);
}

function StatusTile({
	icon: Icon,
	label,
	value,
	ok,
}: Readonly<{
	icon: typeof Smartphone;
	label: string;
	value: string;
	ok: boolean;
}>) {
	return (
		<div className="flex items-center gap-3 rounded-md border border-border/60 bg-background/60 p-3">
			<div
				className={cn(
					"flex h-9 w-9 shrink-0 items-center justify-center rounded-md",
					ok
						? "bg-primary/10 text-primary"
						: "bg-muted text-muted-foreground",
				)}
			>
				<Icon className="h-4 w-4" />
			</div>
			<div className="min-w-0">
				<div className="text-xs text-muted-foreground">{label}</div>
				<div className="truncate text-sm font-medium">{value}</div>
			</div>
		</div>
	);
}
