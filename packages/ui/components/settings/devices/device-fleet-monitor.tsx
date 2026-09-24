"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import {
	readFleet,
	registerFleetReader,
	type OpenFleet,
} from "../../../lib/device-management/fleet";
import { visibleInventory } from "../../../lib/device-management/inventory";
import {
	acquireDeviceLock,
	readDeviceVault,
	type DeviceAccountScope,
	type LocalDeviceVault,
} from "../../../lib/device-management/storage";
import type {
	BrowserController,
	DeviceReceipt,
} from "../../../lib/device-management/types";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { DeviceMetricsView } from "./device-metrics-view";

export function DeviceFleetMonitor({
	deviceId,
	profile,
	scope,
	projectId,
	onSnapshot,
}: {
	deviceId: string;
	profile: IProfile;
	scope: DeviceAccountScope;
	projectId?: string;
	onSnapshot: (device: string, snapshot: OpenFleet | undefined) => void;
}) {
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	const [snapshot, setSnapshot] = useState<OpenFleet>();
	const password = useRef<HTMLInputElement>(null);
	const unlocked = useRef<
		| {
				controller: BrowserController;
				vault: LocalDeviceVault;
				release: () => void;
		  }
		| undefined
	>(undefined);
	const generation = useRef(0);
	const polling = useRef<BrowserController | undefined>(undefined);
	const report = useRef(onSnapshot);
	report.current = onSnapshot;
	const lock = useCallback(() => {
		generation.current++;
		if (password.current) password.current.value = "";
		unlocked.current?.controller.close();
		unlocked.current?.controller.free();
		unlocked.current?.release();
		unlocked.current = undefined;
		setSnapshot(undefined);
		report.current(deviceId, undefined);
	}, [deviceId]);
	const refresh = useCallback(async () => {
		const current = unlocked.current;
		if (!current || polling.current === current.controller) return;
		polling.current = current.controller;
		try {
			const crypto = await loadDeviceCrypto();
			if (unlocked.current !== current) return;
			const value = await readFleet(
				backend.apiState,
				profile,
				scope,
				current.controller,
				current.vault,
				crypto,
				() => unlocked.current === current,
			);
			if (unlocked.current !== current) return;
			setSnapshot(value);
			report.current(deviceId, value);
			setError("");
		} finally {
			if (polling.current === current.controller) polling.current = undefined;
		}
	}, [backend.apiState, profile, scope, deviceId]);
	useEffect(() => {
		const timer = setInterval(() => {
			const version = generation.current;
			void refresh().catch((cause) => {
				if (version !== generation.current) return;
				lock();
				setError(
					cause instanceof Error
						? cause.message
						: "Fleet access could not be confirmed. Unlock again to retry.",
				);
			});
		}, 30_000);
		return () => {
			clearInterval(timer);
			lock();
		};
	}, [refresh, lock]);
	async function unlock() {
		let value = password.current?.value ?? "";
		if (password.current) password.current.value = "";
		const version = ++generation.current;
		setBusy(true);
		setError("");
		let release: (() => void) | undefined;
		try {
			release = await acquireDeviceLock(scope, deviceId);
			if (version !== generation.current) return;
			const vault = await readDeviceVault(scope, deviceId);
			if (!vault)
				throw new Error(
					"Restore this device's encrypted keys before opening fleet monitoring.",
				);
			const crypto = await loadDeviceCrypto();
			if (version !== generation.current) return;
			const controller = await withPassword(value, (bytes) =>
				crypto.unlockControllerVault(deviceId, bytes, vault.controllerVault),
			);
			if (version !== generation.current) {
				controller.close();
				controller.free();
				return;
			}
			unlocked.current = { controller, vault, release };
			release = undefined;
			const receipt = await backend.apiState.get<DeviceReceipt>(
				profile,
				`devices/${encodeURIComponent(deviceId)}/identity`,
			);
			if (version !== generation.current) return;
			await registerFleetReader(
				backend.apiState,
				profile,
				scope,
				controller,
				vault,
				receipt,
				() => version === generation.current,
			);
			if (version === generation.current) await refresh();
		} catch (cause) {
			if (version === generation.current) {
				lock();
				setError(
					cause instanceof Error
						? cause.message
						: "Fleet monitoring could not be opened.",
				);
			}
		} finally {
			value = "";
			release?.();
			if (version === generation.current || !unlocked.current) setBusy(false);
		}
	}
	const placements = visibleInventory(snapshot?.observations ?? [], projectId);
	const hasInventory =
		snapshot?.observations.some(
			(observation) =>
				!projectId ||
				observation.scope.kind === "device" ||
				observation.scope.project_id === projectId,
		) ?? false;
	const samples = (snapshot?.metrics ?? []).flatMap((metric) => {
		if (!projectId) return [{ ...metric, sample: metric.sample }];
		if (metric.scope.kind !== "device")
			return metric.scope.project_id === projectId ? [metric] : [];
		const projects = metric.sample.projects;
		if (!Array.isArray(projects)) return [];
		return projects
			.filter((row) => row.project_id === projectId)
			.map((row) => ({
				...metric,
				scope: { kind: "project" as const, project_id: projectId },
				sample: row.sample as Record<string, unknown>,
			}));
	});
	return (
		<div className="w-full space-y-3">
			<Button
				variant="outline"
				onClick={() => {
					if (open) lock();
					setOpen(!open);
				}}
			>
				{open ? "Close monitoring" : "Fleet inventory and metrics"}
			</Button>
			{open && (
				<div className="space-y-3 rounded border p-3">
					<p className="text-sm text-muted-foreground">
						The device publishes encrypted snapshots without an active
						management connection. Unlock each device to compare this project's
						deployments. Closing or locking clears this view. Lock monitoring
						before opening live management or saved inventory for this device.
					</p>
					{!unlocked.current ? (
						<form
							className="flex gap-2"
							onSubmit={(event) => {
								event.preventDefault();
								void unlock();
							}}
						>
							<Input
								ref={password}
								type="password"
								autoComplete="current-password"
								aria-label="Fleet device password"
								placeholder="Device password"
								required
								disabled={busy}
							/>
							<Button type="submit" disabled={busy}>
								{busy ? "Unlocking…" : "Unlock monitoring"}
							</Button>
						</form>
					) : (
						<Button variant="outline" onClick={lock}>
							Lock monitoring
						</Button>
					)}
					{snapshot && (
						<>
							{placements.length === 0 && (
								<p className="text-sm">
									{hasInventory
										? "No placements in the latest authorized inventory."
										: "No authorized inventory snapshot is available yet."}{" "}
									A newly enabled device may take a minute to publish its first
									snapshot.
								</p>
							)}
							{placements.map(({ row, at }) => (
								<div key={row.id} className="text-sm">
									<strong>
										{row.project_id} / {row.id}
									</strong>
									: {row.observed_state}, {row.ready_replicas ?? 0} ready /{" "}
									{row.desired_replicas ?? 0} requested. Revision {row.revision}
									.{" "}
									<span className="text-muted-foreground">
										Observed {new Date(at).toLocaleString()}.
									</span>
								</div>
							))}
							{samples.map((metric, index) => (
								<div
									key={`${metric.scope.kind}-${index}`}
									className="border-t pt-3"
								>
									<p className="mb-2 text-sm font-medium">
										{metric.scope.kind === "device"
											? "Device metrics"
											: metric.scope.kind === "project"
												? `Project metrics: ${metric.scope.project_id}`
												: `Placement metrics: ${metric.scope.project_id} / ${metric.scope.placement_id}`}
									</p>
									{samples.length > 1 && (
										<p className="mb-2 text-xs text-muted-foreground">
											These panels may cover overlapping scopes.
										</p>
									)}
									<DeviceMetricsView
										sample={metric.sample}
										placement={
											Boolean(projectId) || metric.scope.kind !== "device"
										}
										connected={false}
									/>
								</div>
							))}
						</>
					)}
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
				</div>
			)}
		</div>
	);
}
