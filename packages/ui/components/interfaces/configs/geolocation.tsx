"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../../lib/api-url";
import { executeDeviceCommand } from "../../../lib/device-bridge";
import {
	geofenceCenter,
	geofenceCenterFields,
	validateGeolocationEvent,
} from "../../../lib/geolocation-event";
import { geometryMarker } from "../../../lib/geometry";
import type { GeofencePermissionStatus } from "../../../lib/location";
import { useBackend } from "../../../state/backend-state";
import { GeometryEditor } from "../../flow/variables/geometry-editor";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import type { IConfigInterfaceProps } from "../interfaces";

export function GeolocationConfig({
	config,
	onConfigUpdate,
	isEditing,
	appId,
	eventId,
	eventExecutionMode,
}: IConfigInterfaceProps) {
	const backend = useBackend();
	const auth = useAuth();
	const [requesting, setRequesting] = useState(false);
	const [error, setError] = useState<string>();
	const [pollUntil, setPollUntil] = useState(0);
	const scope = JSON.stringify([
		getApiOrigin(backend.profile),
		backend.profile?.id,
		auth.isAuthenticated ? auth.user?.profile.sub : "local",
		appId,
		eventId,
	]);
	const currentScope = useRef(scope);
	currentScope.current = scope;
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const permission = useQuery({
		queryKey: ["geofence-permission", scope],
		queryFn: async () =>
			executeDeviceCommand(
				"location.geofencePermission",
				{ mode: "status" },
				{ appId, executionTarget: "local" },
			) as Promise<GeofencePermissionStatus>,
		retry: false,
		staleTime: 0,
		refetchInterval: () => (pollUntil > Date.now() ? 1000 : false),
	});
	useEffect(() => {
		setError(undefined);
		setRequesting(false);
		setPollUntil(0);
	}, [scope]);
	useEffect(() => {
		const refresh = () => {
			if (document.visibilityState === "visible") void permission.refetch();
		};
		window.addEventListener("focus", refresh);
		document.addEventListener("visibilitychange", refresh);
		return () => {
			window.removeEventListener("focus", refresh);
			document.removeEventListener("visibilitychange", refresh);
		};
	}, [permission.refetch]);
	const requestPermission = async (mode: "foreground" | "background") => {
		setRequesting(true);
		setError(undefined);
		setPollUntil(Date.now() + 20_000);
		try {
			await executeDeviceCommand(
				"location.geofencePermission",
				{ mode },
				{ appId, executionTarget: "local", userInitiated: true },
			);
			if (mounted.current && currentScope.current === scope)
				await permission.refetch();
		} catch (error) {
			if (mounted.current && currentScope.current === scope)
				setError(
					error instanceof Error
						? error.message
						: "Location permissions could not be updated.",
				);
		} finally {
			if (mounted.current && currentScope.current === scope)
				setRequesting(false);
		}
	};
	const update = (values: Record<string, unknown>) =>
		onConfigUpdate({ ...config, sink_type: "geolocation", ...values });
	const status = permission.data;
	const invalid = validateGeolocationEvent(config);
	return (
		<div className="space-y-5">
			<div className="space-y-2">
				<Label>Region center · Geometry Point</Label>
				<p className="text-xs text-muted-foreground">
					Choose the center of the circular region. Coordinates use WGS84
					longitude, then latitude.
				</p>
				<GeometryEditor
					value={geofenceCenter(config)}
					schema={geometryMarker("Point")}
					disabled={!isEditing}
					allowUnset={false}
					onChange={(value, valid) => {
						try {
							update(
								valid
									? geofenceCenterFields(value)
									: { latitude: null, longitude: null },
							);
						} catch {
							update({ latitude: null, longitude: null });
						}
					}}
				/>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<label className="space-y-2 text-sm">
					Radius (meters)
					<Input
						type="number"
						min={100}
						max={100000}
						step={10}
						disabled={!isEditing}
						value={typeof config.radius === "number" ? config.radius : ""}
						onChange={(event) =>
							update({
								radius:
									event.target.value === "" ? null : Number(event.target.value),
							})
						}
					/>
				</label>
				<label className="flex flex-col gap-2 text-sm">
					Fire when
					<select
						className="h-9 rounded-md border bg-background px-3"
						disabled={!isEditing}
						value={config.trigger_on ?? "Both"}
						onChange={(event) => update({ trigger_on: event.target.value })}
					>
						<option value="Enter">Entering the region</option>
						<option value="Exit">Leaving the region</option>
						<option value="Both">Entering or leaving</option>
					</select>
				</label>
			</div>
			<label className="flex flex-col gap-2 text-sm">
				Monitor on this device
				<select
					className="h-9 rounded-md border bg-background px-3"
					disabled={!isEditing}
					value={config.background === true ? "background" : "foreground"}
					onChange={(event) =>
						update({ background: event.target.value === "background" })
					}
				>
					<option value="foreground">While the app is open</option>
					<option value="background">Also in the background</option>
				</select>
			</label>
			<p className="text-xs text-muted-foreground">
				{config.background === true
					? "The operating system can deliver region crossings while the app is in the background. Delivery can be delayed or unavailable after force-quitting the app. Precise continuous tracking is not used."
					: "Monitoring stops when Flow-Like leaves the foreground. Crossings while the app is inactive are not delivered later."}
			</p>
			{eventExecutionMode === "Remote" && (
				<p className="text-sm text-muted-foreground">
					This device monitors the region. A crossing sends the region center,
					radius and transition to this Event's server workflow.
				</p>
			)}
			{invalid && (
				<p role="alert" className="text-sm text-destructive">
					{invalid}
				</p>
			)}
			<div className="space-y-3 rounded-lg border p-4">
				<div className="flex items-center justify-between gap-3">
					<h3 className="text-sm font-medium">Device location permission</h3>
					<Button
						variant="ghost"
						size="sm"
						onClick={() => void permission.refetch()}
						disabled={permission.isFetching || requesting}
					>
						Refresh
					</Button>
				</div>
				{permission.isLoading && (
					<p className="text-sm text-muted-foreground">Checking this device…</p>
				)}
				{permission.isError && (
					<p role="alert" className="text-sm text-destructive">
						{permission.error instanceof Error
							? permission.error.message
							: "Permission status could not load."}
					</p>
				)}
				{status && (
					<>
						<p className="text-sm">
							{status.error?.code === "unsupported"
								? "Unavailable on this platform"
								: {
										not_determined: "Not requested",
										denied: "Denied in system settings",
										restricted: "Restricted by system policy",
										when_in_use: "Allowed while using the app",
										always: "Always allowed",
									}[status.authorization]}
						</p>
						{["when_in_use", "always"].includes(status.authorization) && (
							<p className="text-xs text-muted-foreground">
								{status.accuracyAuthorization === "reduced"
									? "The device currently provides approximate location."
									: "The device allows precise location."}
							</p>
						)}
						{status.error?.code !== "unsupported" && (
							<p className="text-xs text-muted-foreground">
								{status.monitoredRegionCount} of {status.maxMonitoredRegions}{" "}
								region slots are in use on this device.
							</p>
						)}
						{status.monitoredRegionCount >= status.maxMonitoredRegions && (
							<p role="alert" className="text-sm text-destructive">
								All region slots are in use. Disable another geolocation Event
								before adding a new region.
							</p>
						)}
						{status.error && (
							<p role="alert" className="text-sm text-destructive">
								{status.error.message}
							</p>
						)}
						{status.authorization === "denied" && (
							<p className="text-xs text-muted-foreground">
								Change Flow-Like's Location permission in system settings. The
								operating system may not show another permission prompt.
							</p>
						)}
						{!status.backgroundSupported &&
							status.error?.code !== "unsupported" && (
								<p className="text-sm text-muted-foreground">
									Background region monitoring is unavailable on this device.
								</p>
							)}
						{status.backgroundDelivery === "app_running" && (
							<p className="text-xs text-muted-foreground">
								On this Mac, Flow-Like must stay running and the computer must
								be awake for region monitoring. Closing the app stops delivery.
							</p>
						)}
						<div className="flex flex-wrap gap-2">
							<Button
								variant="outline"
								disabled={
									requesting ||
									status.error?.code === "unsupported" ||
									status.authorization === "restricted" ||
									["when_in_use", "always"].includes(status.authorization)
								}
								onClick={() => void requestPermission("foreground")}
							>
								Allow while using app
							</Button>
							<Button
								variant="outline"
								disabled={
									requesting ||
									!status.backgroundSupported ||
									status.authorization === "restricted" ||
									status.authorization === "always" ||
									(status.backgroundDelivery === "app_running" &&
										status.authorization === "when_in_use")
								}
								onClick={() => void requestPermission("background")}
							>
								Allow background geofencing
							</Button>
						</div>
						{config.background === true &&
							status.authorization !== "always" &&
							status.backgroundDelivery !== "app_running" &&
							status.backgroundSupported && (
								<p className="text-xs text-muted-foreground">
									On iOS, choose Always in system settings after granting
									initial access. Enabling this Event does not request
									permission automatically.
								</p>
							)}
					</>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
			<p className="text-xs text-muted-foreground">
				Save and activate the Event to register this region. Its Geometry output
				is the monitored center. Use Get Current Location separately for a
				current device fix while the app is open.
			</p>
		</div>
	);
}
