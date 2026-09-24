import type { ApprovedOnlineMetadata } from "./online-metadata";
import type { IApp } from "../schema/app/app";
import {
	type ArtifactBlob,
	type PreparedProjectArtifact,
	type ProjectArtifactAssets,
	parseProjectArtifactAssets,
	prepareProjectArtifact,
} from "./artifacts";

export type DesktopExport = {
	export_id: string;
	project_id: string;
	source?: "online" | "offline";
	files: { path: string; size: number }[];
	assets: ProjectArtifactAssets;
};
export type ExportCommands = {
	prepare(projectId: string): Promise<DesktopExport>;
	read(
		exportId: string,
		path: string,
		offset: number,
		length: number,
	): Promise<ArrayBuffer>;
	release(exportId: string): Promise<void>;
};
export type PreparedDesktopProject = {
	artifact: PreparedProjectArtifact;
	assets: ProjectArtifactAssets;
	release(): Promise<void>;
};

export async function desktopExportCommands(
	onlineApp?: IApp,
	userSub?: string,
): Promise<ExportCommands> {
	const { invoke } = await import("@tauri-apps/api/core");
	return {
		prepare: (appId) =>
			invoke("prepare_device_project_export", { appId, onlineApp, userSub }),
		read: (exportId, path, offset, length) =>
			invoke("read_device_project_export_chunk", {
				exportId,
				path,
				offset,
				length,
			}),
		release: (exportId) =>
			invoke("release_device_project_export", { exportId }),
	};
}

class SnapshotBlob implements ArtifactBlob {
	constructor(
		readonly size: number,
		private readonly read: (
			offset: number,
			length: number,
		) => Promise<ArrayBuffer>,
		private readonly offset = 0,
	) {}
	slice(start = 0, end = this.size): ArtifactBlob {
		const clamp = (value: number) =>
			Math.max(0, Math.min(this.size, value < 0 ? this.size + value : value));
		const from = clamp(start);
		return new SnapshotBlob(
			Math.max(0, clamp(end) - from),
			this.read,
			this.offset + from,
		);
	}
	async arrayBuffer(): Promise<ArrayBuffer> {
		if (this.size > 16 * 1024 * 1024)
			throw new Error("Read the project snapshot in bounded chunks.");
		const bytes = new Uint8Array(this.size);
		for (let offset = 0; offset < this.size; offset += 1024 * 1024) {
			const length = Math.min(1024 * 1024, this.size - offset);
			const chunk = await this.read(this.offset + offset, length);
			if (!(chunk instanceof ArrayBuffer) || chunk.byteLength !== length)
				throw new Error("The prepared project returned an incomplete chunk.");
			bytes.set(new Uint8Array(chunk), offset);
		}
		return bytes.buffer;
	}
}

/** Native owns the private snapshot; the browser only reads bounded chunks. */
export async function prepareDesktopProject(
	projectId: string,
	commands: ExportCommands,
	signal?: AbortSignal,
	approved?: ApprovedOnlineMetadata,
): Promise<PreparedDesktopProject> {
	signal?.throwIfAborted();
	const exported = await commands.prepare(projectId);
	let released = false;
	const release = async () => {
		if (released) return;
		released = true;
		await commands.release(exported.export_id);
	};
	try {
		signal?.throwIfAborted();
		if (
			exported.project_id !== projectId ||
			!/^[a-f0-9-]{36}$/.test(exported.export_id) ||
			!Array.isArray(exported.files) ||
			exported.files.length > 8192
		)
			throw new Error(
				"The prepared export belongs to another project or exceeds its limits.",
			);
		const assets = parseProjectArtifactAssets(JSON.stringify(exported.assets));
		const inputs: import("./artifacts").ArtifactInput[] = exported.files.map(
			({ path, size }) => ({
				path,
				file: new SnapshotBlob(size, async (offset, length) => {
					signal?.throwIfAborted();
					if (released)
						throw new Error(
							"The prepared project has been released. Prepare it again.",
						);
					return commands.read(exported.export_id, path, offset, length);
				}),
			}),
		);
		if (exported.source === "online") {
			if (!approved || approved.app.id !== projectId)
				throw new Error(
					"Prepare controller-approved executable metadata before exporting this online project.",
				);
			inputs.push(approved.file);
		}
		const artifact = await prepareProjectArtifact(
			projectId,
			inputs,
			signal,
			assets,
			exported.source ?? "offline",
		);
		return { artifact, assets, release };
	} catch (error) {
		await release().catch(() => {});
		throw error;
	}
}
