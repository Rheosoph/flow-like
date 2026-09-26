"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import {
	type RetainedObservation,
	inventoryPath,
	openInventoryView,
	visibleInventory,
} from "../../../lib/device-management/inventory";
import {
	type DeviceAccountScope,
	acquireDeviceLock,
	readDeviceVault,
} from "../../../lib/device-management/storage";
import type {
	BrowserController,
	InventoryView,
} from "../../../lib/device-management/types";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";

export function DeviceRetainedInventory({
	deviceId,
	profile,
	scope,
	projectId,
}: {
	deviceId: string;
	profile: IProfile;
	scope: DeviceAccountScope;
	projectId?: string;
}) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button variant="outline" onClick={() => setOpen(true)}>
				Saved inventory
			</Button>
			{open && (
				<InventoryDialog
					deviceId={deviceId}
					profile={profile}
					scope={scope}
					projectId={projectId}
					onClose={() => setOpen(false)}
				/>
			)}
		</>
	);
}

function InventoryDialog({
	deviceId,
	profile,
	scope,
	projectId,
	onClose,
}: {
	deviceId: string;
	profile: IProfile;
	scope: DeviceAccountScope;
	projectId?: string;
	onClose: () => void;
}) {
	const backend = useBackend();
	const password = useRef<HTMLInputElement>(null);
	const controller = useRef<BrowserController | undefined>(undefined);
	const release = useRef<(() => void) | undefined>(undefined);
	const alive = useRef(true);
	const [observations, setObservations] = useState<
		RetainedObservation[] | null
	>(null);
	const [error, setError] = useState("");
	const [busy, setBusy] = useState(false);
	const lock = useCallback(() => {
		controller.current?.close();
		controller.current?.free();
		controller.current = undefined;
		release.current?.();
		release.current = undefined;
	}, []);
	const refresh = useCallback(
		async (active: BrowserController) => {
			const key = active.publicBundle().controller_key.x;
			const view = await backend.apiState.get<InventoryView>(
				profile,
				inventoryPath(deviceId, key),
			);
			const saved = await openInventoryView(scope, active, view);
			if (alive.current && controller.current === active)
				setObservations(saved);
		},
		[backend.apiState, profile, scope, deviceId],
	);
	useEffect(() => {
		alive.current = true;
		const timer = setInterval(() => {
			const active = controller.current;
			if (!active) return;
			void refresh(active).catch(() => {
				if (!alive.current || controller.current !== active) return;
				lock();
				setObservations(null);
				setError(
					"Current inventory access could not be confirmed. Unlock again when the hub is reachable.",
				);
			});
		}, 30_000);
		return () => {
			alive.current = false;
			clearInterval(timer);
			lock();
		};
	}, [lock, refresh]);
	async function unlock() {
		setBusy(true);
		setError("");
		try {
			release.current = await acquireDeviceLock(scope, deviceId);
			if (!alive.current) {
				lock();
				return;
			}
			const record = await readDeviceVault(scope, deviceId);
			if (!record)
				throw new Error(
					"Restore this device's encrypted keys before opening its saved inventory.",
				);
			const crypto = await loadDeviceCrypto();
			if (!alive.current) {
				lock();
				return;
			}
			const value = password.current?.value ?? "";
			if (password.current) password.current.value = "";
			const unlocked = await withPassword(value, (bytes) =>
				crypto.unlockControllerVault(deviceId, bytes, record.controllerVault),
			);
			if (!alive.current) {
				unlocked.close();
				unlocked.free();
				lock();
				return;
			}
			controller.current = unlocked;
			await refresh(unlocked);
		} catch (cause) {
			lock();
			if (alive.current) {
				setObservations(null);
				setError(
					cause instanceof Error
						? cause.message
						: "Saved inventory could not be opened.",
				);
			}
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	const latest = visibleInventory(observations ?? [], projectId);
	return (
		<Dialog
			open
			onOpenChange={(value) => {
				if (!value) onClose();
			}}
		>
			<DialogContent className="max-h-[85vh] overflow-auto">
				<DialogHeader>
					<DialogTitle>Saved device inventory</DialogTitle>
					<DialogDescription>
						Encrypted observations from your last successful inspection. The
						device can be offline. This view does not confirm current health or
						pending deployment plans.
					</DialogDescription>
				</DialogHeader>
				{observations === null ? (
					<form
						className="space-y-3"
						onSubmit={(event) => {
							event.preventDefault();
							void unlock();
						}}
					>
						<Input
							ref={password}
							type="password"
							autoComplete="current-password"
							aria-label="Device password"
							placeholder="Device password"
							required
							disabled={busy}
						/>
						<Button type="submit" disabled={busy}>
							{busy ? "Unlocking…" : "Unlock saved inventory"}
						</Button>
					</form>
				) : (
					<div className="space-y-3">
						{latest.length === 0 ? (
							<p>
								No retained placements are visible for this{" "}
								{projectId ? "project" : "device"}. Connect and inspect the
								device to record an observation.
							</p>
						) : (
							latest.map(({ row, at }) => (
								<div
									key={row.id}
									className="space-y-1 rounded border p-3 text-sm"
								>
									<p className="font-medium">
										{row.project_id} / {row.id}
									</p>
									<p>
										Last observed {new Date(at).toLocaleString()}:{" "}
										{row.observed_state}
									</p>
									<p>
										Revision {row.revision}; configuration {row.config_revision}
										; applied {row.applied_revision ?? "pending"}.
									</p>
									<p>
										At that time: {row.ready_replicas ?? 0} ready /{" "}
										{row.desired_replicas ?? 0} requested replicas; desired
										state {row.desired_state}.
									</p>
								</div>
							))
						)}
						<Button
							variant="outline"
							onClick={() => {
								lock();
								setObservations(null);
							}}
						>
							Lock inventory
						</Button>
					</div>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</DialogContent>
		</Dialog>
	);
}
