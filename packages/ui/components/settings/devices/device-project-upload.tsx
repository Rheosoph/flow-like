"use client";
import { useEffect, useId, useRef, useState } from "react";
import {
	type ArtifactProgress,
	ArtifactUploadError,
	type PreparedProjectArtifact,
	type ProjectArtifactAssets,
	abortProjectArtifact,
	parseProjectArtifactAssets,
	prepareOnlineProjectCache,
	prepareProjectArtifact,
	projectFilesFromSelection,
	selectedProjectAssetFiles,
	uploadProjectArtifact,
} from "../../../lib/device-management/artifacts";
import { prepareOnlineDependencies } from "../../../lib/device-management/online-dependencies";
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
}: {
	connected: boolean;
	projectId?: string;
	accountSubject?: string;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	onInstalled: (identity: {
		project_id: string;
		project_path: string;
		revision?: string;
		source: "offline" | "online";
		assets?: ProjectArtifactAssets;
	}) => void;
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
	const alive = useRef(true);
	const abort = useRef(new AbortController());
	const inputVersion = useRef(0);
	const desktopSnapshot = useRef<PreparedDesktopProject | undefined>(undefined);
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
				if (app.bits.length || Object.keys(app.packages ?? {}).length) {
					if (isTauri()) {
						const exported = await prepareDesktopProject(
							project,
							await desktopExportCommands(app),
							abort.current.signal,
						);
						if (!alive.current) {
							await exported.release();
							return;
						}
						desktopSnapshot.current = exported;
						setAssets(exported.assets);
						setPrepared(exported.artifact);
					} else {
						const profile = await backend.userState.getProfile();
						if (!profile)
							throw new Error(
								"Sign in to resolve this project's dependencies.",
							);
						const exported = await prepareOnlineDependencies(
							app,
							backend,
							profile,
							abort.current.signal,
						);
						if (alive.current) {
							setAssets(exported.assets);
							setPrepared(exported.artifact);
						}
					}
					return;
				}
				const projectPath = await run((call) =>
					prepareOnlineProjectCache(call, project),
				);
				if (alive.current) {
					setPath(projectPath);
					onInstalled({
						project_id: project,
						project_path: projectPath,
						source: "online",
					});
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
				setPrepared(exported.artifact);
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
				setPrepared(artifact);
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
	async function upload() {
		if (!prepared || busy) return;
		setBusy(true);
		setError(undefined);
		abort.current = new AbortController();
		try {
			await run(async (call) => {
				const result = await uploadProjectArtifact({
					prepared,
					request: call,
					transferId,
					signal: abort.current.signal,
					onProgress: (value) => {
						if (alive.current) {
							setProgress(value);
							setTransferId(value.transferId);
						}
					},
				});
				if (alive.current && result.project_path) {
					setPath(result.project_path);
					setTransferId(undefined);
					onInstalled({
						project_id: prepared.descriptor.project_id,
						project_path: result.project_path,
						revision: prepared.descriptor.manifest_sha256,
						source: prepared.descriptor.source ?? "offline",
						assets,
					});
					await desktopSnapshot.current?.release();
					desktopSnapshot.current = undefined;
				}
			});
		} catch (error) {
			if (alive.current) {
				if (error instanceof ArtifactUploadError)
					setTransferId(error.transferId);
				setError(
					error instanceof Error ? error.message : "Project upload failed.",
				);
			}
		} finally {
			if (alive.current) setBusy(false);
		}
	}
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
								onClick={() => {
									setBusy(true);
									void run((call) =>
										abortProjectArtifact(call, project, transferId),
									)
										.then(() => {
											if (alive.current) {
												setTransferId(undefined);
												setProgress(undefined);
											}
										})
										.catch((error) => {
											if (alive.current)
												setError(
													error instanceof Error
														? error.message
														: "Transfer abort was not confirmed.",
												);
										})
										.finally(() => {
											if (alive.current) setBusy(false);
										});
								}}
							>
								Abort device transfer
							</Button>
							<Button
								variant="outline"
								onClick={() => {
									setTransferId(undefined);
									setProgress(undefined);
									setError(undefined);
								}}
							>
								Use a new transfer
							</Button>
						</>
					)}
				</div>
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
