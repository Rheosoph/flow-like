"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import type { InstalledProject } from "../../../lib/device-management/deployment";
import { readDeviceInspection } from "../../../lib/device-management/inspection";
import {
	type InventoryWriter,
	createInventoryWriter,
} from "../../../lib/device-management/inventory";
import type { ReleaseConfig } from "../../../lib/device-management/package";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	acquireDeviceLock,
	addDeviceVault,
	controllerBackup,
	readDeviceVault,
	replaceEndpointVault,
	replaceRestoredVault,
} from "../../../lib/device-management/storage";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import { DeviceManagementConnection } from "../../../lib/device-management/transport";
import type {
	BrowserController,
	DeviceReceipt,
	Inspection,
	ManagementResponse,
	OnboardingManifest,
} from "../../../lib/device-management/types";
import type { DeviceStatus } from "../../../lib/devices";
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
import { Textarea } from "../../ui/textarea";
import { DeviceAccountRecovery } from "./device-account-recovery";
import { DeviceArchiveHistory } from "./device-archive-history";
import { DeviceDeploymentForm } from "./device-deployment-form";
import { DeviceGroupMetrics } from "./device-group-metrics";
import { DeviceHostOperations } from "./device-host-operations";
import { DeviceMessagesView } from "./device-messages-view";
import { DeviceMetricsView } from "./device-metrics-view";
import { DeviceOfflineQueue } from "./device-offline-queue";
import { DevicePasswordChange } from "./device-password-change";
import { DeviceProjectUpload } from "./device-project-upload";
import { DeviceReplicaControl } from "./device-replica-control";
import { DeviceSharingForm } from "./device-sharing-form";

export function DeviceManagementDialog({
	device,
	profile,
	scope,
	release,
	projectId,
	onInspection,
	onClose,
}: {
	device: DeviceStatus;
	profile: IProfile;
	scope: DeviceAccountScope;
	release?: ReleaseConfig;
	projectId?: string;
	onInspection?: (inspection: Inspection) => void;
	onClose: () => void;
}) {
	const backend = useBackend();
	const formId = useId();
	const [password, setPassword] = useState("");
	const [record, setRecord] = useState<LocalDeviceVault>();
	const [loaded, setLoaded] = useState(false);
	const [error, setError] = useState<string>();
	const [busy, setBusy] = useState(false);
	const [connected, setConnected] = useState(false);
	const [transport, setTransport] = useState<string>();
	const [inspection, setInspection] = useState<Inspection>();
	const [manifest, setManifest] = useState<OnboardingManifest>();
	const [deviceReceipt, setDeviceReceipt] = useState<DeviceReceipt>();
	const [audience, setAudience] = useState("");
	const [metrics, setMetrics] = useState<Record<string, unknown>>();
	const [messages, setMessages] = useState<Record<string, unknown>>();
	const [projectMetrics, setProjectMetrics] =
		useState<Record<string, unknown>>();
	const messagePosition = useRef(0);
	const [logs, setLogs] = useState<Record<string, unknown>[]>([]);
	const logPosition = useRef(0);
	const [deployment, setDeployment] = useState("");
	const [installedProject, setInstalledProject] = useState<InstalledProject>();
	const [installedGeneration, setInstalledGeneration] = useState(0);
	const [operation, setOperation] = useState("");
	const [lastResult, setLastResult] = useState<ManagementResponse>();
	const [sharing, setSharing] = useState(false);
	const [resetPassword, setResetPassword] = useState("");
	const [resetApproved, setResetApproved] = useState(false);
	const [pendingSecret, setPendingSecret] = useState<{
		placement: string;
		operationId: string;
	}>();
	const current = useRef(true);
	const controller = useRef<BrowserController | undefined>(undefined);
	const connection = useRef<DeviceManagementConnection | undefined>(undefined);
	const unlock = useRef<(() => void) | undefined>(undefined);
	const working = useRef(false);
	const abort = useRef(new AbortController());
	const path = `devices/${encodeURIComponent(device.device_id)}`;
	const visiblePlacements =
		inspection?.placements.filter(
			(row) => !projectId || row.project_id === projectId,
		) ?? [];
	useEffect(() => {
		if (
			projectId &&
			!inspection?.placements.some(
				(row) => row.project_id === projectId && row.id === audience,
			)
		)
			setAudience(
				inspection?.placements.find((row) => row.project_id === projectId)
					?.id ?? "",
			);
	}, [projectId, inspection, audience]);
	const inventoryWriter = useRef<InventoryWriter | undefined>(undefined);
	const inventoryWrites = useRef(Promise.resolve());
	function acceptInspection(value: Inspection) {
		setInspection(value);
		onInspection?.(value);
		const report = () => {
			if (current.current)
				setError(
					"Live inspection succeeded, but its encrypted inventory could not be retained. Reconnect to retry.",
				);
		};
		try {
			if (!inventoryWriter.current) {
				report();
				return;
			}
			inventoryWrites.current = inventoryWriter.current(value).catch(report);
		} catch {
			report();
		}
	}
	useEffect(() => {
		current.current = true;
		abort.current = new AbortController();
		void readDeviceVault(scope, device.device_id)
			.then((value) => {
				if (current.current) {
					setRecord(value);
					setLoaded(true);
				}
			})
			.catch(() => {
				if (current.current) {
					setLoaded(true);
					setError("Local encrypted device keys could not be read.");
				}
			});
		return () => {
			current.current = false;
			abort.current.abort();
			inventoryWriter.current = undefined;
			connection.current?.close();
			controller.current?.close();
			controller.current?.free();
			unlock.current?.();
		};
	}, [scope, device.device_id]);
	async function establish(
		active: BrowserController,
		stored: LocalDeviceVault,
	) {
		const module = await loadDeviceCrypto();
		const publicKey = active.publicBundle();
		if (
			publicKey.device_id !== device.device_id ||
			publicKey.controller_key.x !== stored.controllerPublic.controller_key.x
		)
			throw new Error("The unlocked controller does not match this device.");
		const receipt = await backend.apiState.fetch<DeviceReceipt>(
			profile,
			`${path}/identity`,
			{ signal: abort.current.signal },
		);
		const accepted = module.verifyDeviceReceipt(
			receipt,
			stored.manifestJws,
			stored.ownerControllerKey ?? publicKey.controller_key,
		);
		if (
			accepted.device_id !== device.device_id ||
			accepted.api_base_url !==
				`${scope.apiOrigin.replace(/\/$/u, "")}/api/v1` ||
			receipt.owner_id !== device.owner_id
		)
			throw new Error(
				"The signed device identity belongs to another device or hub.",
			);
		const opened = await DeviceManagementConnection.connect(
			backend.apiState,
			profile,
			active,
			receipt,
			stored.grantId,
			abort.current.signal,
		);
		if (!current.current) {
			opened.close();
			return;
		}
		connection.current = opened;
		setTransport(opened.transport);
		setManifest(accepted);
		setDeviceReceipt(receipt);
		setConnected(true);
		inventoryWriter.current = undefined;
		try {
			await inventoryWrites.current;
			if (!current.current || controller.current !== active) return;
			const writer = await createInventoryWriter(
				backend.apiState,
				profile,
				scope,
				active,
				stored.grantId,
				() => current.current && controller.current === active,
			);
			if (!current.current || controller.current !== active) return;
			inventoryWriter.current = writer;
		} catch {
			if (!current.current || controller.current !== active) return;
			setError(
				"Encrypted inventory could not be prepared. Reconnect to retry.",
			);
		}
		const status = await readDeviceInspection(
			(value) => opened.request(value),
			device.device_id,
		);
		if (current.current) acceptInspection(status);
	}
	async function connect() {
		if (working.current || !record) return;
		working.current = true;
		setBusy(true);
		setError(undefined);
		const secret = password;
		setPassword("");
		let stored = record;
		const alreadyUnlocked = Boolean(controller.current);
		try {
			if (!controller.current) {
				unlock.current = await acquireDeviceLock(scope, device.device_id);
				const module = await loadDeviceCrypto();
				await withPassword(secret, async (bytes) => {
					controller.current = module.unlockControllerVault(
						device.device_id,
						bytes,
						record.controllerVault,
					);
					if (record.requiresFreshEndpoint) {
						const fresh = controller.current.freshEndpointVault(bytes);
						stored = {
							...record,
							controllerPublic: fresh.public_bundle,
							controllerVault: Uint8Array.from(fresh.vault),
							requiresFreshEndpoint: false,
						};
						await replaceRestoredVault(scope, record, stored);
						if (current.current) setRecord(stored);
					}
				});
			}
			connection.current?.close();
			const active = controller.current;
			if (!active || !current.current)
				throw new Error("Management unlock cancelled.");
			await establish(active, stored);
		} catch (error) {
			connection.current?.close();
			connection.current = undefined;
			if (!alreadyUnlocked) {
				controller.current?.close();
				controller.current?.free();
				controller.current = undefined;
				unlock.current?.();
				unlock.current = undefined;
			}
			if (current.current) {
				setConnected(false);
				setError(
					error instanceof Error
						? error.message
						: "The device could not be connected.",
				);
			}
		} finally {
			working.current = false;
			if (current.current) setBusy(false);
		}
	}
	async function command(value: Record<string, unknown>, refresh = false) {
		if (working.current || !connection.current) return;
		working.current = true;
		setBusy(true);
		setError(undefined);
		try {
			const result = await connection.current.request(value);
			if (current.current) setLastResult(result);
			if (result.state === "rejected")
				throw new Error(
					"The device rejected this operation under its current permissions or revision.",
				);
			if (refresh) {
				const active = connection.current;
				const status = await readDeviceInspection(
					(value) => active.request(value),
					device.device_id,
				);
				if (current.current) acceptInspection(status);
			}
		} catch (error) {
			if (current.current)
				setError(
					error instanceof Error ? error.message : "Device operation failed.",
				);
		} finally {
			working.current = false;
			if (current.current) setBusy(false);
		}
	}
	async function resetReader() {
		if (working.current || !record || !resetApproved) return;
		working.current = true;
		setBusy(true);
		setError(undefined);
		const secret = resetPassword;
		setResetPassword("");
		try {
			const module = await loadDeviceCrypto();
			await withPassword(secret, async (bytes) => {
				const temporary = module.unlockControllerVault(
					device.device_id,
					bytes,
					record.controllerVault,
				);
				try {
					const fresh = temporary.freshEndpointVault(bytes);
					const replacement = {
						...record,
						controllerPublic: fresh.public_bundle,
						controllerVault: Uint8Array.from(fresh.vault),
						requiresFreshEndpoint: false,
					};
					if (!current.current) return;
					await replaceEndpointVault(scope, record, replacement);
					connection.current?.close();
					connection.current = undefined;
					controller.current?.close();
					controller.current?.free();
					controller.current = undefined;
					unlock.current?.();
					unlock.current = undefined;
					if (current.current) {
						setRecord(replacement);
						setResetApproved(false);
						setConnected(false);
						setLogs([]);
						setMetrics(undefined);
						setMessages(undefined);
						setProjectMetrics(undefined);
					}
				} finally {
					temporary.close();
					temporary.free();
				}
			});
		} catch (error) {
			if (current.current)
				setError(
					error instanceof Error
						? error.message
						: "The browser reader could not be reset.",
				);
		} finally {
			working.current = false;
			if (current.current) setBusy(false);
		}
	}
	async function runGroup<T>(
		operation: (call: ManagementCall) => Promise<T>,
	): Promise<T> {
		if (working.current || !connection.current)
			throw new Error("Wait for the current device operation to finish.");
		working.current = true;
		setBusy(true);
		const active = connection.current;
		try {
			return await operation((command, operationId) =>
				active.request(command, operationId),
			);
		} finally {
			working.current = false;
			if (current.current) setBusy(false);
		}
	}
	useEffect(() => {
		if (!connected) return;
		let cancelled = false;
		logPosition.current = 0;
		messagePosition.current = 0;
		setLogs([]);
		setMetrics(undefined);
		setMessages(undefined);
		setProjectMetrics(undefined);
		const poll = async () => {
			if (cancelled || working.current || !connection.current) return;
			working.current = true;
			setBusy(true);
			try {
				if (connection.current.expiresAt <= Date.now() / 1000 + 10) {
					connection.current.close();
					connection.current = undefined;
					if (current.current) setConnected(false);
					return;
				}
				const hasLiveScope = Boolean(audience) || !projectId;
				const sample = hasLiveScope
					? await connection.current.request({
							type: "metrics",
							placement_id: audience || null,
						})
					: undefined;
				const entries = hasLiveScope
					? await connection.current.request({
							type: "logs",
							placement_id: audience || null,
							after: logPosition.current,
							limit: 20,
						})
					: undefined;
				const transitions = await connection.current.request({
					type: "messages",
					placement_id: audience || null,
					project_id: !audience ? (projectId ?? null) : null,
					after: messagePosition.current,
					limit: 20,
				});
				const projectSample = projectId
					? await connection.current.request({
							type: "project_metrics",
							project_id: projectId,
						})
					: undefined;
				if (cancelled || !current.current) return;
				if (projectSample?.state === "completed")
					setProjectMetrics(projectSample.result);
				if (
					transitions.state === "completed" &&
					Array.isArray(transitions.result.records) &&
					Number.isSafeInteger(transitions.result.next) &&
					Number(transitions.result.next) >= messagePosition.current
				) {
					messagePosition.current = Number(transitions.result.next);
					setMessages((previous) => ({
						...transitions.result,
						records: [
							...(Array.isArray(previous?.records) ? previous.records : []),
							...(transitions.result.records as Record<string, unknown>[]),
						].slice(-100),
					}));
				}
				if (sample?.state === "completed") setMetrics(sample.result);
				if (
					entries?.state === "completed" &&
					Array.isArray(entries.result.records) &&
					typeof entries.result.next === "number"
				) {
					logPosition.current = entries.result.next;
					setLogs((previous) =>
						[
							...previous,
							...(entries.result.records as Record<string, unknown>[]),
						].slice(-200),
					);
				}
			} catch (error) {
				if (!cancelled && current.current) {
					setConnected(false);
					setError(
						error instanceof Error
							? error.message
							: "Live telemetry connection interrupted.",
					);
				}
			} finally {
				working.current = false;
				if (current.current) setBusy(false);
			}
		};
		void poll();
		const timer = setInterval(() => void poll(), 5000);
		return () => {
			cancelled = true;
			clearInterval(timer);
		};
	}, [connected, audience, projectId]);
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[90vh] max-w-4xl overflow-auto">
				<DialogHeader>
					<DialogTitle>Manage {device.name}</DialogTitle>
					<DialogDescription>
						Commands and telemetry are encrypted between this app and the
						device.
					</DialogDescription>
				</DialogHeader>
				{!loaded ? (
					<output>Loading encrypted local keys…</output>
				) : !record ? (
					<div className="space-y-3">
						<p>
							This app has no controller vault for this device. Import the
							encrypted backup created during setup.
						</p>
						<Input
							type="file"
							accept=".json,application/json"
							onChange={async (event) => {
								const file = event.target.files?.[0];
								event.target.value = "";
								if (!file) return;
								try {
									if (file.size > 1024 * 1024)
										throw new Error(
											"Controller backups must be smaller than 1 MiB.",
										);
									const stored = controllerBackup(
										await file.text(),
										scope,
										device.device_id,
									);
									if (!current.current) return;
									await addDeviceVault(scope, stored);
									if (current.current) {
										setRecord(stored);
										setError(undefined);
									}
								} catch (error) {
									if (current.current)
										setError(
											error instanceof Error
												? error.message
												: "The backup could not be imported.",
										);
								}
							}}
						/>
					</div>
				) : !connected ? (
					<form
						className="space-y-3"
						onSubmit={(event) => {
							event.preventDefault();
							void connect();
						}}
					>
						{!controller.current && (
							<label
								htmlFor={`${formId}-password`}
								className="block space-y-1 text-sm"
							>
								Management password
								<Input
									id={`${formId}-password`}
									type="password"
									autoComplete="current-password"
									value={password}
									onChange={(event) => setPassword(event.target.value)}
									required
									disabled={busy}
								/>
							</label>
						)}
						<Button type="submit" disabled={busy}>
							{busy
								? "Connecting…"
								: controller.current
									? "Reconnect"
									: "Unlock and connect"}
						</Button>
					</form>
				) : (
					<div className="space-y-5">
						<div className="flex items-center justify-between gap-2">
							<p className="text-sm">
								Connected through{" "}
								{transport === "webrtc"
									? "WebRTC"
									: "the encrypted WebSocket relay"}
							</p>
							<Button
								variant="outline"
								onClick={() => {
									connection.current?.close();
									connection.current = undefined;
									controller.current?.close();
									controller.current?.free();
									controller.current = undefined;
									unlock.current?.();
									unlock.current = undefined;
									setConnected(false);
									setLogs([]);
									setMetrics(undefined);
									setMessages(undefined);
									setProjectMetrics(undefined);
								}}
							>
								Lock
							</Button>
						</div>
						<div className="space-y-2">
							<div className="flex flex-wrap items-center justify-between gap-2">
								<h3 className="font-semibold">Project placements</h3>
								<Button
									variant="outline"
									size="sm"
									disabled={busy}
									onClick={() =>
										void runGroup((call) =>
											readDeviceInspection(call, device.device_id),
										)
											.then((value) => {
												if (current.current) acceptInspection(value);
											})
											.catch((error) => {
												if (current.current)
													setError(
														error instanceof Error
															? error.message
															: "Device status could not be read.",
													);
											})
									}
								>
									Refresh status
								</Button>
							</div>
							{inspection?.observed_at && (
								<p className="text-xs text-muted-foreground">
									Status read at{" "}
									{new Date(inspection.observed_at).toLocaleTimeString()}. Pages
									reflect device state as each page was received.
								</p>
							)}
							{visiblePlacements.length === 0 ? (
								<p className="text-sm text-muted-foreground">
									No project services are deployed on this device.
								</p>
							) : (
								visiblePlacements.map((placement) => (
									<div
										key={placement.id}
										className="flex flex-wrap items-center gap-2 rounded border p-3"
									>
										<div className="mr-auto">
											<p className="font-medium">{placement.project_id}</p>
											<p className="text-xs text-muted-foreground">
												{placement.id} · {placement.observed_state} · revision{" "}
												{placement.config_revision}
											</p>
										</div>
										{(["start", "stop", "restart"] as const).map((action) => (
											<Button
												key={action}
												variant="outline"
												size="sm"
												disabled={
													busy ||
													((action === "start" || action === "restart") &&
														pendingSecret?.placement === placement.id)
												}
												onClick={() =>
													void command(
														{
															type: action,
															placement_id: placement.id,
															expected_revision: placement.config_revision,
														},
														true,
													)
												}
											>
												{action[0].toUpperCase() + action.slice(1)}
											</Button>
										))}
										<DeviceReplicaControl
											placement={placement}
											busy={busy}
											request={(value) => command(value, true)}
										/>
										<DeviceOfflineQueue
											placement={placement.id}
											busy={busy}
											run={runGroup}
										/>
									</div>
								))
							)}
						</div>
						<details className="rounded border p-3">
							<summary className="cursor-pointer font-medium">
								Advanced placement JSON
							</summary>
							<form
								className="mt-3 space-y-3"
								onSubmit={(event) => {
									event.preventDefault();
									try {
										const config = JSON.parse(deployment) as Record<
											string,
											unknown
										>;
										if (projectId && config.project_id !== projectId)
											throw new Error(
												"This placement belongs to another project.",
											);
										if (
											!config.id ||
											!config.project_id ||
											!config.revision ||
											!Array.isArray(config.events)
										)
											throw new Error(
												"Provide a placement with project, revision and pinned events.",
											);
										const expected =
											inspection?.placements.find((row) => row.id === config.id)
												?.config_revision ?? 0;
										void command(
											{
												type: "apply",
												config,
												expected_revision: expected,
												start: false,
											},
											true,
										);
									} catch (error) {
										setError(
											error instanceof Error
												? error.message
												: "Invalid placement JSON.",
										);
									}
								}}
							>
								<p className="text-sm text-muted-foreground">
									The project files must already be installed at the path in
									this placement. Variables in this file apply only to this
									device.
								</p>
								<Textarea
									value={deployment}
									onChange={(event) => setDeployment(event.target.value)}
									maxLength={12_000}
									rows={7}
									placeholder="Paste the pinned placement JSON"
									required
								/>
								<Button disabled={busy} type="submit">
									Apply placement stopped
								</Button>
							</form>
						</details>
						<div className="space-y-2">
							<label className="block text-sm">
								Telemetry scope
								<select
									className="ml-2 rounded border bg-background p-2"
									value={audience}
									onChange={(event) => setAudience(event.target.value)}
								>
									{!projectId && <option value="">Device</option>}
									{visiblePlacements.map((row) => (
										<option key={row.id} value={row.id}>
											{row.project_id} / {row.id}
										</option>
									))}
								</select>
							</label>
							<p className="text-xs text-muted-foreground">
								Samples refresh every 5 seconds while this session is connected.
								Logs remain in this view until it is locked.
							</p>
							{(!projectId || audience) && (
								<>
									<h3 className="font-medium">Latest metrics</h3>
									<DeviceMetricsView
										sample={metrics}
										placement={Boolean(audience)}
										connected={connected}
									/>
								</>
							)}
							{projectMetrics && (
								<>
									<h3 className="font-medium">
										Project usage across placements
									</h3>
									<DeviceMetricsView
										sample={projectMetrics}
										placement
										connected={connected}
									/>
								</>
							)}
							<h3 className="font-medium">Device messages</h3>
							<DeviceMessagesView sample={messages} />
							<h3 className="font-medium">Logs</h3>
							<pre className="max-h-56 overflow-auto rounded bg-muted p-3 text-xs">
								{logs.length
									? logs.map((row) => JSON.stringify(row)).join("\n")
									: "No log entries received for this scope."}
							</pre>
						</div>
						<details className="rounded border p-3">
							<summary className="cursor-pointer font-medium">
								Check an unconfirmed operation
							</summary>
							<form
								className="mt-3 flex gap-2"
								onSubmit={(event) => {
									event.preventDefault();
									void command({ type: "operation", operation_id: operation });
								}}
							>
								<Input
									value={operation}
									onChange={(event) => setOperation(event.target.value)}
									placeholder="Operation ID"
									required
								/>
								<Button type="submit" disabled={busy}>
									Check status
								</Button>
							</form>
						</details>
						{manifest &&
							deviceReceipt &&
							controller.current &&
							(!projectId || audience) && (
								<DeviceGroupMetrics
									key={audience || "device"}
									controller={controller.current}
									account={scope}
									manifest={manifest}
									receipt={deviceReceipt}
									scope={audience || "device"}
									profile={profile}
									invitationVault={record.invitationVault}
									run={runGroup}
								/>
							)}
						{manifest &&
							deviceReceipt &&
							controller.current &&
							(!projectId || audience) && (
								<DeviceArchiveHistory
									key={`archive:${audience || "device"}`}
									controller={controller.current}
									account={scope.account}
									manifest={manifest}
									receipt={deviceReceipt}
									scope={audience || "device"}
									projectId={
										inspection?.placements.find((row) => row.id === audience)
											?.project_id
									}
									profile={profile}
									invitationVault={record.invitationVault}
									run={runGroup}
								/>
							)}
						{record.invitationVault &&
							manifest &&
							!projectId &&
							device.owner_id === scope.account && (
								<>
									<Button
										variant="outline"
										onClick={() => setSharing((value) => !value)}
									>
										Share device access
									</Button>
									{sharing && deviceReceipt && (
										<DeviceSharingForm
											profile={profile}
											manifest={manifest}
											receipt={deviceReceipt}
											invitationVault={record.invitationVault}
										/>
									)}
								</>
							)}
						{inspection && (
							<DeviceHostOperations
								bootId={inspection.boot_id}
								release={release}
								placements={visiblePlacements}
								run={runGroup}
								pending={pendingSecret}
								setPending={setPendingSecret}
							/>
						)}
						<details className="rounded border p-3">
							<summary className="cursor-pointer font-medium">
								Recover a group reader
							</summary>
							<div className="mt-3 space-y-3">
								<p className="text-sm text-muted-foreground">
									If this browser missed evicted MLS messages, generate a fresh
									browser endpoint. Every metrics group then needs a new owner
									admission. Your approved controller identity and
									retained-history key remain the same.
								</p>
								<label className="flex items-start gap-2 text-sm">
									<input
										type="checkbox"
										checked={resetApproved}
										onChange={(event) => setResetApproved(event.target.checked)}
										disabled={busy}
									/>
									Reset this browser's group readers and disconnect management.
								</label>
								<label
									htmlFor={`${formId}-reset-password`}
									className="block space-y-1 text-sm"
								>
									Management password
									<Input
										id={`${formId}-reset-password`}
										type="password"
										autoComplete="current-password"
										value={resetPassword}
										onChange={(event) => setResetPassword(event.target.value)}
										disabled={busy}
									/>
								</label>
								<Button
									variant="outline"
									disabled={busy || !resetApproved || !resetPassword}
									onClick={() => void resetReader()}
								>
									Create fresh browser endpoint
								</Button>
							</div>
						</details>
						{lastResult && (
							<output className="block text-sm">
								Operation {lastResult.operation_id}: {lastResult.state}
							</output>
						)}
					</div>
				)}
				{loaded && !connected && !controller.current && (
					<DeviceAccountRecovery
						profile={profile}
						scope={scope}
						deviceId={device.device_id}
						record={record}
						disabled={busy}
						onRestored={(value) => {
							setRecord(value);
							setPassword("");
							setError(undefined);
						}}
					/>
				)}
				{record && !connected && !controller.current && (
					<DevicePasswordChange
						scope={scope}
						record={record}
						disabled={busy}
						onChanged={(value) => {
							setRecord(value);
							setPassword("");
							setError(undefined);
						}}
					/>
				)}
				{controller.current && installedProject && (
					<DeviceDeploymentForm
						key={`${device.device_id}:${projectId ?? ""}:${installedGeneration}:${installedProject.project_id}:${installedProject.project_path}`}
						installed={installedProject}
						placements={visiblePlacements}
						connected={connected}
						deviceId={device.device_id}
						profile={profile}
						run={runGroup}
						onApplied={async () => {
							const value = await runGroup((call) =>
								readDeviceInspection(
									(command) => call(command),
									device.device_id,
								),
							);
							if (current.current) acceptInspection(value);
						}}
					/>
				)}
				{record && (
					<DeviceProjectUpload
						accountSubject={scope.account}
						connected={connected}
						projectId={projectId}
						run={runGroup}
						onInstalled={(identity) => {
							if (projectId && identity.project_id !== projectId) return;
							setInstalledProject(identity);
							setInstalledGeneration((previous) => previous + 1);
							setDeployment((previous) => {
								let value: Record<string, unknown> = {};
								try {
									if (previous)
										value = JSON.parse(previous) as Record<string, unknown>;
								} catch {
									return previous;
								}
								if (
									value.project_id &&
									value.project_id !== identity.project_id
								)
									return previous;
								return JSON.stringify(
									{
										...value,
										project_id: identity.project_id,
										project_path: identity.project_path,
										source: identity.source,
										...(identity.revision
											? { revision: identity.revision }
											: {}),
									},
									null,
									2,
								);
							});
						}}
					/>
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
