"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Server, ShieldOff } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useHub } from "../../../hooks/use-hub";
import { useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../lib/api-error";
import { getApiOrigin } from "../../../lib/api-url";
import { DeviceFleetMonitor } from "./device-fleet-monitor";
import { DeviceFleetSummary } from "./device-fleet-summary";
import type { OpenFleet } from "../../../lib/device-management/fleet";
import type { ReleaseConfig } from "../../../lib/device-management/package";
import type {
	Inspection,
	PlacementStatus,
} from "../../../lib/device-management/types";
import {
	type DeviceStatus,
	hasRecentDeviceContact,
	listDevices,
	revokeDevice,
} from "../../../lib/devices";
import { asArray } from "../../../lib/response-shape";
import { useBackend, useBackendReady } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card, CardContent } from "../../ui/card";
import { DeviceAccessDialog } from "./device-access-dialog";
import { DeviceManagementDialog } from "./device-management-dialog";
import { DeviceResourcesDialog } from "./device-resources-dialog";
import { DeviceRetainedInventory } from "./device-retained-inventory";
import { DeviceSetupDialog } from "./device-setup-dialog";

export function DevicesPage({ projectId }: { projectId?: string } = {}) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const account = auth.user?.profile.sub;
	const issuer = auth.user?.profile.iss ?? "";
	const signedIn = auth.isAuthenticated && !!account;
	const { hub, refetch: refreshHub } = useHub([issuer, account ?? ""]);
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready && signedIn,
		[issuer, account],
	);

	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="container mx-auto flex max-w-6xl flex-col gap-6 px-2 pb-4">
				<div className="space-y-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">
						{projectId ? "Project devices" : t("devices", "Devices")}
					</h1>
					<p className="text-muted-foreground">
						{projectId
							? "Unlock a device to view this project's deployments, revisions and replica health."
							: t(
									"devicesDescription",
									"Standalone devices registered to your account.",
								)}
					</p>
				</div>
				{!ready || auth.isLoading ? (
					<output>{t("loading", "Loading…")}</output>
				) : !signedIn ? (
					<p>
						{t("devicesSignIn", "Sign in to view your registered devices.")}
					</p>
				) : !hub ? (
					<Card>
						<CardContent className="space-y-3 pt-6">
							<p>
								{t(
									"devicesHubUnavailable",
									"Device availability has not been confirmed by this hub.",
								)}
							</p>
							<Button variant="outline" onClick={() => void refreshHub()}>
								{t("retry", "Retry")}
							</Button>
						</CardContent>
					</Card>
				) : hub.standalone?.enabled !== true ? (
					<p>
						{t(
							"devicesDisabled",
							"Standalone devices are not enabled on this hub.",
						)}
					</p>
				) : profile.isError ? (
					<div role="alert" className="space-y-3">
						<p>
							{t(
								"devicesProfileError",
								"Your account profile could not be loaded.",
							)}
						</p>
						<Button variant="outline" onClick={() => void profile.refetch()}>
							{t("retry", "Retry")}
						</Button>
					</div>
				) : !profile.data ? (
					<output>{t("loading", "Loading…")}</output>
				) : (
					<DeviceInventory
						key={JSON.stringify([
							issuer,
							account,
							getApiOrigin(profile.data),
							profile.data.id,
							projectId,
						])}
						profile={profile.data}
						account={account}
						issuer={issuer}
						releaseTrust={hub.standalone.release_trust ?? undefined}
						projectId={projectId}
					/>
				)}
			</div>
		</div>
	);
}

function DeviceInventory({
	profile,
	account,
	issuer,
	releaseTrust,
	projectId,
}: {
	profile: IProfile;
	account: string;
	issuer: string;
	projectId?: string;
	releaseTrust?: {
		manifest_url: string;
		public_keys: string[];
		minimum_sequence: number;
	};
}) {
	const { t, i18n } = useTranslation("settings");
	const backend = useBackend();
	const client = useQueryClient();
	const [selected, setSelected] = useState<DeviceStatus | null>(null);
	const [resourceDevice, setResourceDevice] = useState<string | null>(null);
	const [managementDevice, setManagementDevice] = useState<string | null>(null);
	const [setup, setSetup] = useState(false);
	const [fleet, setFleet] = useState<Record<string, OpenFleet>>({});
	const [requestAccess, setRequestAccess] = useState(false);
	const [observations, setObservations] = useState<
		Record<string, { at: number; placements: PlacementStatus[] }>
	>({});
	const apiOrigin = getApiOrigin(profile);
	const scope = useMemo(
		() => ({
			issuer,
			account,
			apiOrigin,
			profileId: profile.id ?? "default",
		}),
		[issuer, account, apiOrigin, profile.id],
	);
	const release = useMemo<ReleaseConfig | undefined>(
		() =>
			releaseTrust
				? {
						manifestUrl: releaseTrust.manifest_url,
						publicKeys: releaseTrust.public_keys,
						minimumSequence: releaseTrust.minimum_sequence,
					}
				: undefined,
		[releaseTrust],
	);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const queryKey = [
		"devices",
		getApiOrigin(profile),
		issuer,
		account,
		profile.id,
	];
	const devices = useQuery({
		queryKey,
		queryFn: async () => asArray(await listDevices(backend.apiState, profile)),
		staleTime: 15_000,
		gcTime: 0,
		refetchInterval: (query) => (query.state.error ? false : 30_000),
		refetchIntervalInBackground: false,
		retry: false,
		meta: { persist: false },
	});
	const revoke = useMutation({
		mutationFn: (device: DeviceStatus) =>
			revokeDevice(backend.apiState, profile, device.device_id),
		onSuccess: async (_, device) => {
			if (!mounted.current) return;
			await client.cancelQueries({ queryKey, exact: true });
			if (!mounted.current) return;
			client.setQueryData<DeviceStatus[]>(queryKey, (previous) =>
				previous?.map((row) =>
					row.device_id === device.device_id
						? { ...row, status: "revoked" }
						: row,
				),
			);
			setSelected(null);
			await client.invalidateQueries({ queryKey, exact: true });
		},
	});
	const formatDate = (seconds: number) =>
		new Intl.DateTimeFormat(i18n.resolvedLanguage ?? i18n.language, {
			dateStyle: "medium",
			timeStyle: "short",
		}).format(new Date(seconds * 1_000));
	const now = Date.now();
	const selectedResourceDevice = devices.data?.find(
		(device) => device.device_id === resourceDevice,
	);
	const selectedManagementDevice = devices.data?.find(
		(device) =>
			device.device_id === managementDevice && device.status === "active",
	);
	function recordInspection(inspection: Inspection) {
		if (
			!projectId ||
			!mounted.current ||
			inspection.device_id !== managementDevice
		)
			return;
		setObservations((previous) => ({
			...previous,
			[inspection.device_id]: {
				at: Math.floor(Date.now() / 1000),
				placements: inspection.placements.filter(
					(placement) => placement.project_id === projectId,
				),
			},
		}));
	}

	return (
		<>
			{projectId && (
				<DeviceFleetSummary
					projectId={projectId}
					fleet={fleet}
					deviceCount={
						devices.data?.filter((d) => d.status === "active").length ?? 0
					}
				/>
			)}
			<div className="flex flex-wrap items-center justify-between gap-3">
				<Button onClick={() => setSetup(true)}>Set up a device</Button>
				<Button variant="outline" onClick={() => setRequestAccess(true)}>
					Request shared access
				</Button>
				<p className="text-sm text-muted-foreground">
					{t(
						"devicesPresenceExplanation",
						"Last contact is the latest heartbeat received by the hub. It does not confirm a live management connection.",
					)}
				</p>
				<Button
					variant="outline"
					disabled={devices.isFetching}
					onClick={() => void devices.refetch()}
				>
					<RefreshCw
						className={`mr-2 size-4 ${devices.isFetching ? "animate-spin" : ""}`}
						aria-hidden="true"
					/>
					{t("refresh", "Refresh")}
				</Button>
			</div>
			{devices.isPending ? (
				<output>{t("devicesLoading", "Loading devices…")}</output>
			) : devices.isError ? (
				<p role="alert" className="text-destructive">
					{apiErrorMessage(
						devices.error,
						t(
							"devicesLoadError",
							"Devices could not be loaded. Try refreshing.",
						),
					)}
				</p>
			) : devices.data.length === 0 ? (
				<Card>
					<CardContent className="flex flex-col items-center gap-3 py-12 text-center">
						<Server
							className="size-8 text-muted-foreground"
							aria-hidden="true"
						/>
						<h2 className="text-lg font-medium">
							{t("devicesEmpty", "No registered devices")}
						</h2>
						<p className="max-w-md text-sm text-muted-foreground">
							{t(
								"devicesEmptyDescription",
								"Devices appear here after standalone onboarding completes.",
							)}
						</p>
					</CardContent>
				</Card>
			) : (
				<ul className="space-y-3">
					{devices.data.map((device) => (
						<li key={device.device_id}>
							<Card>
								<CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
									<div className="min-w-0 space-y-2">
										<div className="flex flex-wrap items-center gap-2">
											<h2 className="break-all font-semibold">{device.name}</h2>
											<Badge
												variant={
													device.status === "revoked"
														? "destructive"
														: "secondary"
												}
											>
												{device.status === "revoked"
													? t("devicesRevoked", "Access revoked")
													: t("devicesRegistered", "Registered")}
											</Badge>
											{hasRecentDeviceContact(device, now) && (
												<Badge variant="outline">
													{t("devicesRecent", "Seen in the last 2 minutes")}
												</Badge>
											)}
										</div>
										<p className="break-all font-mono text-xs text-muted-foreground">
											{device.device_id}
										</p>
										<dl className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
											<div>
												<dt className="text-muted-foreground">
													{t("devicesLastContact", "Last contact")}
												</dt>
												<dd>
													{device.last_seen_at === null
														? t("devicesNeverSeen", "No heartbeat received")
														: formatDate(device.last_seen_at)}
												</dd>
											</div>
											<div>
												<dt className="text-muted-foreground">
													{t("devicesRegisteredAt", "Registered on")}
												</dt>
												<dd>{formatDate(device.registered_at)}</dd>
											</div>
										</dl>
										{projectId && device.status === "active" && (
											<div className="space-y-2 border-t pt-3 text-sm">
												{observations[device.device_id] ? (
													<>
														<p className="text-muted-foreground">
															Observed{" "}
															{formatDate(observations[device.device_id].at)}.
															Reconnect to refresh.
														</p>
														{observations[device.device_id].placements
															.length === 0 ? (
															<p>
																No placements for this project were visible at
																that time.
															</p>
														) : (
															observations[device.device_id].placements.map(
																(placement) => (
																	<div key={placement.id} className="space-y-1">
																		<p className="font-medium">
																			{placement.id}: {placement.observed_state}
																		</p>
																		<p>
																			Revision {placement.revision},
																			configuration {placement.config_revision},
																			applied{" "}
																			{placement.applied_revision ?? "pending"}
																		</p>
																		<p>
																			{placement.ready_replicas} ready /{" "}
																			{placement.desired_replicas} requested
																			replicas
																		</p>
																	</div>
																),
															)
														)}
													</>
												) : (
													<p className="text-muted-foreground">
														Unlock to check whether this project is deployed
														here.
													</p>
												)}
											</div>
										)}
									</div>
									{(device.status === "active" ||
										device.status === "revoked") && (
										<Button
											variant="outline"
											onClick={() => setResourceDevice(device.device_id)}
										>
											{t("deviceResourcesTitle", "Hosted resources")}
										</Button>
									)}
									{device.status === "active" && (
										<DeviceFleetMonitor
											deviceId={device.device_id}
											profile={profile}
											scope={scope}
											projectId={projectId}
											onSnapshot={(id, snapshot) =>
												setFleet((previous) => {
													const next = { ...previous };
													if (snapshot) next[id] = snapshot;
													else delete next[id];
													return next;
												})
											}
										/>
									)}
									{device.status === "active" && (
										<DeviceRetainedInventory
											deviceId={device.device_id}
											profile={profile}
											scope={scope}
											projectId={projectId}
										/>
									)}
									{device.status === "active" && (
										<Button
											variant="outline"
											onClick={() => setManagementDevice(device.device_id)}
										>
											Manage
										</Button>
									)}
									{device.status === "active" &&
										device.owner_id === account && (
											<Button
												variant="outline"
												className="shrink-0 text-destructive"
												disabled={revoke.isPending}
												onClick={() => {
													revoke.reset();
													setSelected(device);
												}}
											>
												<ShieldOff className="mr-2 size-4" aria-hidden="true" />
												{t("devicesRevoke", "Revoke access")}
											</Button>
										)}
								</CardContent>
							</Card>
						</li>
					))}
				</ul>
			)}
			{selectedResourceDevice && (
				<DeviceResourcesDialog
					device={selectedResourceDevice}
					deviceConfirmed={!devices.isError && !devices.isFetching}
					profile={profile}
					account={account}
					issuer={issuer}
					onClose={() => setResourceDevice(null)}
				/>
			)}
			{selectedManagementDevice && (
				<DeviceManagementDialog
					device={selectedManagementDevice}
					profile={profile}
					scope={scope}
					release={release}
					projectId={projectId}
					onInspection={recordInspection}
					onClose={() => setManagementDevice(null)}
				/>
			)}
			{setup && (
				<DeviceSetupDialog
					profile={profile}
					scope={scope}
					release={release}
					onClose={() => setSetup(false)}
				/>
			)}
			{requestAccess && (
				<DeviceAccessDialog
					scope={scope}
					onClose={() => setRequestAccess(false)}
				/>
			)}
			<AlertDialog
				open={selected !== null}
				onOpenChange={(open) => {
					if (!open && !revoke.isPending) setSelected(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("devicesRevokeTitle", "Revoke device access?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"devicesRevokeDescription",
								'This revokes "{{name}}" access to this Flow-Like account. Existing local services keep running. This does not power off the device or erase its local data.',
								{ name: selected?.name ?? "" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					{revoke.isError && (
						<p role="alert" className="text-sm text-destructive">
							{apiErrorMessage(
								revoke.error,
								t(
									"devicesRevokeError",
									"Access could not be revoked. Try again.",
								),
							)}
						</p>
					)}
					<AlertDialogFooter>
						<AlertDialogCancel disabled={revoke.isPending}>
							{t("cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							disabled={revoke.isPending || selected === null}
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							onClick={(event) => {
								event.preventDefault();
								if (selected && !revoke.isPending) revoke.mutate(selected);
							}}
						>
							{revoke.isPending
								? t("devicesRevoking", "Revoking…")
								: t("devicesRevoke", "Revoke access")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}
