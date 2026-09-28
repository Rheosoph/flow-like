"use client";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import {
	type ArtifactProgress,
	ArtifactUploadError,
	type PendingArtifactTransfer,
	type PreparedProjectArtifact,
	type ProjectArtifactAssets,
	abortProjectArtifact,
	abortRefusalSettles,
	forgetArtifactTransfer,
	parseProjectArtifactAssets,
	pendingArtifactTransfers,
	prepareProjectArtifact,
	projectFilesFromSelection,
	rememberArtifactTransfer,
	selectedProjectAssetFiles,
	uploadProjectArtifact,
} from "../../../lib/device-management/artifacts";
import {
	DEPLOYMENT_CONFIG_BYTES,
	type InstalledProject,
	deploymentPinBytes,
} from "../../../lib/device-management/deployment";
import { prepareOnlineDependencies } from "../../../lib/device-management/online-dependencies";
import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "../../../lib/device-management/online-metadata";
import {
	type PreparedDesktopProject,
	desktopExportCommands,
	prepareDesktopProject,
} from "../../../lib/device-management/project-export";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import { isTauri } from "../../../lib/platform";
import { IAppVisibility } from "../../../lib/schema/app/app";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DeviceProjectUpload({
	connected,
	run,
	onInstalled,
	projectId,
	accountSubject,
	deviceId,
}: {
	connected: boolean;
	projectId?: string;
	accountSubject?: string;
	/** Enables resuming or aborting this browser's unfinished uploads after a reload. */
	deviceId?: string;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	onInstalled: (identity: InstalledProject) => void;
}) {
	const backend = useBackend();
	const id = useId();
	const [projects, setProjects] = useState<{ id: string; name: string }[]>([]);
	const [project, setProject] = useState(projectId ?? "");
	const [projectFiles, setProjectFiles] = useState<File[]>([]);
	const [assetFiles, setAssetFiles] = useState<File[]>([]);
	const [assets, setAssets] = useState<ProjectArtifactAssets>({
		bit_pins: [],
		package_pins: [],
	});
	const [prepared, setPrepared] = useState<PreparedProjectArtifact>();
	const [transferId, setTransferId] = useState<string>();
	const [progress, setProgress] = useState<ArtifactProgress>();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [path, setPath] = useState<string>();
	const [pending, setPending] = useState<PendingArtifactTransfer[]>([]);
	const alive = useRef(true);
	const abort = useRef(new AbortController());
	const inputVersion = useRef(0);
	const desktopSnapshot = useRef<PreparedDesktopProject | undefined>(undefined);
	const approvedMetadata = useRef(
		new WeakMap<PreparedProjectArtifact, ApprovedOnlineMetadata>(),
	);
	const confirmedTransfers = useRef(new Set<string>());
	const refreshPending = useCallback(() => {
		setPending(
			deviceId && project ? pendingArtifactTransfers(deviceId, project) : [],
		);
	}, [deviceId, project]);
	useEffect(() => refreshPending(), [refreshPending]);
	function adoptPrepared(
		artifact: PreparedProjectArtifact,
		approved?: ApprovedOnlineMetadata,
	) {
		if (approved) approvedMetadata.current.set(artifact, approved);
		setPrepared(artifact);
		setTransferId(
			deviceId
				? pendingArtifactTransfers(
						deviceId,
						artifact.descriptor.project_id,
					).find(
						(transfer) =>
							transfer.manifest_sha256 === artifact.descriptor.manifest_sha256,
					)?.transfer_id
				: undefined,
		);
	}
	useEffect(() => {
		if (projectId || !connected) return;
		let active = true;
		void backend.appState
			.getApps()
			.then((apps) => {
				if (active)
					setProjects(
						apps.map(([app, metadata]) => ({
							id: app.id,
							name: metadata?.name || app.id,
						})),
					);
			})
			.catch(() => {
				if (active)
					setError(
						"Projects could not be listed. Reconnect or use advanced import.",
					);
			});
		return () => {
			active = false;
		};
	}, [backend, connected, projectId]);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
			abort.current.abort();
			inputVersion.current++;
			void desktopSnapshot.current?.release().catch(() => {});
		};
	}, []);
	async function prepareCurrentProject() {
		if (busy || !connected || !project) return;
		setBusy(true);
		setError(undefined);
		setPrepared(undefined);
		setTransferId(undefined);
		setPath(undefined);
		abort.current = new AbortController();
		try {
			await desktopSnapshot.current?.release();
			desktopSnapshot.current = undefined;
			const app = await backend.appState.getAppAuthoritative(project);
			abort.current.signal.throwIfAborted();
			if (app.id !== project)
				throw new Error("The selected project identity changed.");
			if (app.visibility !== IAppVisibility.Offline) {
				const profile = await backend.userState.getProfile();
				if (!profile)
					throw new Error("Sign in to prepare executable metadata.");
				const metadata = await prepareOnlineMetadata(
					project,
					backend,
					profile,
					abort.current.signal,
				);
				if (
					isTauri() &&
					(metadata.app.bits.length ||
						Object.keys(metadata.app.packages ?? {}).length)
				) {
					const exported = await prepareDesktopProject(
						project,
						await desktopExportCommands(metadata.app),
						abort.current.signal,
						metadata,
					);
					if (!alive.current) {
						await exported.release();
						return;
					}
					desktopSnapshot.current = exported;
					setAssets(exported.assets);
					adoptPrepared(exported.artifact, metadata);
				} else {
					const exported = await prepareOnlineDependencies(
						metadata.app,
						backend,
						profile,
						abort.current.signal,
						metadata,
					);
					if (alive.current) {
						setAssets(exported.assets);
						adoptPrepared(exported.artifact, metadata);
					}
				}
			} else {
				if (!isTauri())
					throw new Error(
						"Open this local project in the desktop app to export its database and files, or use advanced import.",
					);
				const exported = await prepareDesktopProject(
					project,
					await desktopExportCommands(undefined, accountSubject),
					abort.current.signal,
				);
				if (!alive.current) {
					await exported.release();
					return;
				}
				desktopSnapshot.current = exported;
				setAssets(exported.assets);
				adoptPrepared(exported.artifact);
			}
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error
						? error.message
						: "The project could not be prepared for deployment.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function prepare() {
		if (busy || !projectFiles.length) return;
		const version = ++inputVersion.current;
		setBusy(true);
		setError(undefined);
		setPrepared(undefined);
		setTransferId(undefined);
		setPath(undefined);
		abort.current = new AbortController();
		try {
			await desktopSnapshot.current?.release();
			desktopSnapshot.current = undefined;
			const additional =
				assets.bit_pins.length || assets.package_pins.length
					? await selectedProjectAssetFiles(
							assetFiles,
							assets,
							abort.current.signal,
						)
					: [];
			const artifact = await prepareProjectArtifact(
				project,
				[...projectFilesFromSelection(project, projectFiles), ...additional],
				abort.current.signal,
				assets,
			);
			if (alive.current && version === inputVersion.current)
				adoptPrepared(artifact);
		} catch (error) {
			if (alive.current && version === inputVersion.current)
				setError(
					error instanceof Error
						? error.message
						: "Project files could not be prepared.",
				);
		} finally {
			if (alive.current && version === inputVersion.current) setBusy(false);
		}
	}
	/** `confirmed` means the device reported this transfer, so it certainly holds staging space. */
	function remember(transfer: string, confirmed: boolean) {
		if (confirmed) {
			if (confirmedTransfers.current.has(transfer)) return;
			confirmedTransfers.current.add(transfer);
		}
		if (!deviceId || !prepared) return;
		rememberArtifactTransfer(deviceId, {
			transfer_id: transfer,
			project_id: prepared.descriptor.project_id,
			manifest_sha256: prepared.descriptor.manifest_sha256,
			confirmed,
		});
		refreshPending();
	}
	function isConfirmed(transfer: string) {
		return (
			confirmedTransfers.current.has(transfer) ||
			(deviceId !== undefined &&
				pendingArtifactTransfers(deviceId).some(
					(entry) => entry.transfer_id === transfer && entry.confirmed,
				))
		);
	}
	function forget(id: string) {
		if (deviceId) forgetArtifactTransfer(deviceId, id);
	}
	async function upload() {
		if (!prepared || busy) return;
		const approved = approvedMetadata.current.get(prepared);
		if (prepared.descriptor.source === "online" && !approved) {
			setError("Prepare this online project again before uploading it.");
			return;
		}
		const resumed = transferId;
		setBusy(true);
		setError(undefined);
		abort.current = new AbortController();
		try {
			await run(async (call) => {
				const result = await uploadProjectArtifact({
					prepared,
					request: call,
					transferId,
					confirmed: resumed ? isConfirmed(resumed) : undefined,
					signal: abort.current.signal,
					onProgress: (value) => {
						remember(value.transferId, true);
						if (alive.current) {
							setProgress(value);
							setTransferId(value.transferId);
						}
					},
				});
				forget(result.transfer_id);
				if (alive.current && result.project_path) {
					setPath(result.project_path);
					setTransferId(undefined);
					onInstalled({
						project_id: prepared.descriptor.project_id,
						project_path: result.project_path,
						revision: prepared.descriptor.manifest_sha256,
						source: prepared.descriptor.source ?? "offline",
						assets,
						...(approved
							? {
									online_metadata_sha256: approved.sha256,
									online_catalog: approved.catalog,
								}
							: {}),
					});
					await desktopSnapshot.current?.release();
					desktopSnapshot.current = undefined;
				}
			});
		} catch (error) {
			// A begin whose reply was lost still holds device staging space until it is aborted.
			if (error instanceof ArtifactUploadError) {
				if (error.transferId) remember(error.transferId, false);
				else if (resumed) forget(resumed);
			}
			if (alive.current) {
				if (error instanceof ArtifactUploadError)
					setTransferId(error.transferId);
				setError(
					error instanceof Error ? error.message : "Project upload failed.",
				);
			}
		} finally {
			if (alive.current) {
				refreshPending();
				setBusy(false);
			}
		}
	}
	/** Frees device staging space; a settling refusal means nothing is left to abort. */
	async function discard(ids: string[]) {
		if (busy || !connected || !ids.length) return;
		setBusy(true);
		setError(undefined);
		const failures: string[] = [];
		try {
			await run(async (call) => {
				for (const id of ids) {
					try {
						await abortProjectArtifact(call, project, id);
					} catch (error) {
						if (!abortRefusalSettles(error, isConfirmed(id))) {
							failures.push(
								error instanceof Error
									? error.message
									: `Aborting transfer ${id} was not confirmed.`,
							);
							continue;
						}
					}
					forget(id);
					if (alive.current && id === transferId) {
						setTransferId(undefined);
						setProgress(undefined);
					}
				}
			});
		} catch (error) {
			failures.push(
				error instanceof Error ? error.message : "Transfer abort failed.",
			);
		} finally {
			if (alive.current) {
				refreshPending();
				if (failures.length) setError(failures.join(" "));
				setBusy(false);
			}
		}
	}
	const unfinished = pending.filter(
		(transfer) => transfer.transfer_id !== transferId,
	);
	const pinBytes = prepared ? deploymentPinBytes(assets) : 0;
	return (
		<details className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Install project files
			</summary>
			<div className="mt-3 space-y-3">
				<p className="text-sm text-muted-foreground">
					Prepare the selected project and its dependencies for this device.
					Local projects include a database snapshot and files; online projects
					keep their cloud data source.
				</p>
				{projectId ? (
					<p className="text-sm">Project: {projectId}</p>
				) : (
					<label
						className="block space-y-1 text-sm"
						htmlFor={`${id}-selected-project`}
					>
						Project
						<select
							id={`${id}-selected-project`}
							className="w-full rounded border bg-background p-2"
							value={project}
							disabled={busy}
							onChange={(event) => {
								setProject(event.target.value);
								setPrepared(undefined);
								setTransferId(undefined);
								setProgress(undefined);
								setPath(undefined);
								void desktopSnapshot.current?.release().catch(() => {});
								desktopSnapshot.current = undefined;
							}}
						>
							<option value="">Select a project</option>
							{projects.map((item) => (
								<option key={item.id} value={item.id}>
									{item.name}
								</option>
							))}
						</select>
					</label>
				)}
				<Button
					disabled={!connected || busy || !project}
					onClick={() => void prepareCurrentProject()}
				>
					{busy && !prepared
						? "Preparing project…"
						: "Prepare deployment from project"}
				</Button>
				<details className="rounded border p-3">
					<summary className="cursor-pointer text-sm">
						Advanced: import an existing project folder
					</summary>
					<div className="mt-3 space-y-3">
						<p className="text-sm text-muted-foreground">
							Select the project folder containing manifest.app. Files are
							hashed locally, sent over the encrypted management session, and
							published as one revision. Large uploads can resume after
							reconnecting while this dialog stays open.
						</p>
						<label
							htmlFor={`${id}-project`}
							className="block space-y-1 text-sm"
						>
							Project ID
							<Input
								id={`${id}-project`}
								value={project}
								onChange={(event) => {
									setProject(event.target.value);
									setPrepared(undefined);
									setTransferId(undefined);
									setProgress(undefined);
									setPath(undefined);
									inputVersion.current++;
								}}
								disabled={busy || Boolean(projectId)}
								maxLength={128}
							/>
						</label>
						<label htmlFor={`${id}-files`} className="block space-y-1 text-sm">
							Offline project folder
							<Input
								id={`${id}-files`}
								type="file"
								multiple
								{...{ webkitdirectory: "" }}
								disabled={busy || !project}
								onChange={(event) => {
									const files = Array.from(event.target.files ?? []);
									event.target.value = "";
									setProjectFiles(files);
									setPrepared(undefined);
									setTransferId(undefined);
									setPath(undefined);
								}}
							/>
						</label>

						<details>
							<summary className="cursor-pointer text-sm">
								Include offline model and node-package assets
							</summary>
							<div className="mt-3 space-y-3">
								<label
									htmlFor={`${id}-pins`}
									className="block space-y-1 text-sm"
								>
									Public asset pins JSON
									<Input
										id={`${id}-pins`}
										type="file"
										accept=".json,application/json"
										disabled={busy}
										onChange={async (event) => {
											const file = event.target.files?.[0];
											event.target.value = "";
											if (!file) return;
											const version = ++inputVersion.current;
											setBusy(true);
											setError(undefined);
											setPrepared(undefined);
											setTransferId(undefined);
											setPath(undefined);
											try {
												if (file.size > 65_536)
													throw new Error("Asset pins exceed 64 KiB.");
												const selected = parseProjectArtifactAssets(
													await file.text(),
												);
												if (alive.current && version === inputVersion.current)
													setAssets(selected);
											} catch (error) {
												if (alive.current && version === inputVersion.current)
													setError(
														error instanceof Error
															? error.message
															: "Invalid asset pins.",
													);
											} finally {
												if (alive.current && version === inputVersion.current)
													setBusy(false);
											}
										}}
									/>
								</label>
								<label
									htmlFor={`${id}-assets`}
									className="block space-y-1 text-sm"
								>
									Offline assets object-store folder
									<Input
										id={`${id}-assets`}
										type="file"
										multiple
										{...{ webkitdirectory: "" }}
										disabled={busy}
										onChange={(event) => {
											setAssetFiles(Array.from(event.target.files ?? []));
											event.target.value = "";
											setPrepared(undefined);
											setTransferId(undefined);
											setPath(undefined);
										}}
									/>
								</label>
								<p className="text-xs text-muted-foreground">
									{assets.bit_pins.length} pinned Bits ·{" "}
									{assets.package_pins.length} pinned node packages. Only the
									selected assets and their verified dependencies are included.
								</p>
							</div>
						</details>
						<Button
							variant="outline"
							disabled={busy || !project || !projectFiles.length}
							onClick={() => void prepare()}
						>
							Verify selected files and prepare revision
						</Button>
					</div>
				</details>
				{prepared && (
					<div>
						<p className="break-all text-xs">
							{prepared.descriptor.file_count} files ·{" "}
							{prepared.descriptor.total_bytes.toLocaleString()} bytes ·
							revision {prepared.descriptor.manifest_sha256}
						</p>
						{desktopSnapshot.current &&
							prepared.descriptor.source !== "online" && (
								<p className="mt-2 text-xs text-muted-foreground">
									The selected account’s project files and tables are included.
									Database snapshots contain current rows and schemas. Workflows
									that require history, named versions or full-text indexes are
									blocked during preparation. Configure device secrets before
									starting the service.
								</p>
							)}
						{pinBytes > DEPLOYMENT_CONFIG_BYTES - 2048 && (
							<p role="alert" className="mt-2 text-xs">
								Bit and node-package pins of this project need{" "}
								{pinBytes.toLocaleString()} of the{" "}
								{DEPLOYMENT_CONFIG_BYTES.toLocaleString()} bytes a remote
								deployment configuration can carry, so remote deployment will
								likely fail. Remove Bits or node packages the project no longer
								uses.
							</p>
						)}
					</div>
				)}
				<div className="flex flex-wrap gap-2">
					<Button
						disabled={!connected || busy || !prepared || Boolean(path)}
						onClick={() => void upload()}
					>
						{transferId ? "Resume encrypted upload" : "Upload project revision"}
					</Button>
					{busy && (
						<Button variant="outline" onClick={() => abort.current.abort()}>
							Pause local transfer
						</Button>
					)}
					{transferId && !busy && (
						<>
							<Button
								variant="outline"
								disabled={!connected}
								onClick={() => void discard([transferId])}
							>
								Abort device transfer
							</Button>
							<Button
								variant="outline"
								disabled={!connected}
								onClick={() => {
									setTransferId(undefined);
									setProgress(undefined);
									void discard([transferId]);
								}}
							>
								Abort and use a new transfer
							</Button>
						</>
					)}
				</div>
				{unfinished.length > 0 && !busy && (
					<div className="space-y-2 rounded border p-2 text-xs">
						<p>
							{unfinished.length} unfinished upload
							{unfinished.length === 1 ? "" : "s"} of this project from earlier
							sessions still reserve device staging space until they expire.
						</p>
						<Button
							variant="outline"
							size="sm"
							disabled={!connected}
							onClick={() =>
								void discard(unfinished.map((transfer) => transfer.transfer_id))
							}
						>
							Abort unfinished uploads
						</Button>
					</div>
				)}
				{progress && (
					<p className="text-xs">
						{progress.phase}: {progress.completedFiles}/{progress.totalFiles}{" "}
						files · {progress.uploadedBytes.toLocaleString()}/
						{progress.totalBytes.toLocaleString()} bytes
					</p>
				)}
				{transferId && (
					<p className="break-all font-mono text-xs">Transfer: {transferId}</p>
				)}
				<p className="text-xs text-muted-foreground">
					Online projects also need an owner-approved online resource grant.
					Preparing a deployment does not grant access to project files or
					models.
				</p>
				{!connected && (
					<p className="text-sm">
						Connect or reconnect above to transfer files.
					</p>
				)}
				{path && (
					<output className="block break-all text-xs">
						Installed project path: {path}. Use this path with the exact project
						revision in the stopped placement.
					</output>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
		</details>
	);
}
