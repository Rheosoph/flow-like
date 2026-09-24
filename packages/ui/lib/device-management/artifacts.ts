import { sha256 } from "@noble/hashes/sha2";

export const ARTIFACT_CHUNK_BYTES = 8192;
const MAX_FILES = 8192;
const MAX_FILE_BYTES = 4 * 1024 ** 3;
const MAX_BYTES = 8 * 1024 ** 3;
const MAX_MANIFEST_BYTES = 2 * 1024 ** 2;
const encoder = new TextEncoder();
export type ProjectArtifactFile = {
	path: string;
	size: number;
	sha256: string;
};
export type ProjectBitPin = { bit_id: string; metadata_sha256: string };
export type ProjectPackagePin = {
	package_id: string;
	version: string;
	wasm_sha256: string;
	manifest_sha256: string;
};
export type ProjectArtifactAssets = {
	bit_pins: ProjectBitPin[];
	package_pins: ProjectPackagePin[];
};
export type ProjectArtifactManifest = {
	version: 1;
	project_id: string;
	source?: "online";
	files: ProjectArtifactFile[];
	bit_pins?: ProjectBitPin[];
	package_pins?: ProjectPackagePin[];
};
export type ProjectArtifactDescriptor = {
	project_id: string;
	source?: "online";
	manifest_sha256: string;
	manifest_size: number;
	file_count: number;
	total_bytes: number;
};
export interface ArtifactBlob {
	readonly size: number;
	arrayBuffer(): Promise<ArrayBuffer>;
	slice(start?: number, end?: number): ArtifactBlob;
}
export type ArtifactInput = { path: string; file: ArtifactBlob };
export type PreparedProjectArtifact = {
	descriptor: ProjectArtifactDescriptor;
	manifest: Uint8Array<ArrayBuffer>;
	files: readonly ArtifactInput[];
};
export type ArtifactTransferStatus = {
	transfer_id: string;
	descriptor: ProjectArtifactDescriptor;
	state: "receiving" | "committed" | "aborted";
	expires_at: number;
	manifest_ready: boolean;
	file_index: number | null;
	offset: number;
	complete: boolean;
	project_path: string | null;
};
export type ArtifactManagementCall = (
	command: Record<string, unknown>,
	operationId?: string,
) => Promise<{ state: string; result: unknown }>;
export type ArtifactProgress = {
	transferId: string;
	phase: "manifest" | "files" | "commit";
	uploadedBytes: number;
	totalBytes: number;
	completedFiles: number;
	totalFiles: number;
};
export class ArtifactUploadError extends Error {
	constructor(readonly transferId: string) {
		super(
			"Project upload has no confirmed completion. Reconnect and resume this transfer, or abort it.",
		);
		this.name = "ArtifactUploadError";
	}
}
function check(value: unknown, message: string): asserts value {
	if (!value) throw new Error(message);
}
function projectId(value: string): void {
	check(
		/^[A-Za-z0-9_.-]{1,128}$/.test(value) && value !== "." && value !== "..",
		"Invalid project identifier.",
	);
}
function id(value: string): void {
	check(
		/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(
			value,
		),
		"Invalid artifact transfer identifier.",
	);
}
function cancelled(signal?: AbortSignal): void {
	signal?.throwIfAborted();
}
function hex(value: Uint8Array): string {
	return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}
function base64(bytes: Uint8Array): string {
	let value = "";
	for (const byte of bytes) value += String.fromCharCode(byte);
	return btoa(value)
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/, "");
}
export function validateProjectArtifactPath(
	project: string,
	path: string,
): void {
	projectId(project);
	check(
		path.startsWith(`apps/${project}/`),
		"File path is outside this project store.",
	);
	validateRelativePath(path);
}
function validateRelativePath(path: string): void {
	check(
		encoder.encode(path).length <= 1024 && path.normalize("NFC") === path,
		"Invalid project file path.",
	);
	const parts = path.split("/");
	check(parts.length <= 32, "Project file path exceeds its depth limit.");
	for (const part of parts) {
		const lower = part.toLowerCase();
		const stem = lower.split(".")[0] ?? "";
		check(
			part.length > 0 &&
				encoder.encode(part).length <= 255 &&
				part !== "." &&
				part !== ".." &&
				!/[. ]$/.test(part) &&
				!/[\p{Cc}\\:*?"<>|\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]/u.test(
					part,
				) &&
				!/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/.test(stem) &&
				lower !== ".secrets" &&
				lower !== ".env" &&
				!lower.endsWith(".secret") &&
				![
					"management.sqlite",
					"secrets.key",
					"device-keys.json",
					"onboarding.json",
					"release-trust.json",
				].includes(lower),
			"Project contains an unsupported or private file path.",
		);
	}
}
/** A directory picker selects the project's own folder, containing manifest.app. */
export function projectFilesFromSelection(
	project: string,
	files: readonly File[],
): ArtifactInput[] {
	projectId(project);
	check(
		files.length > 0 && files.length <= MAX_FILES,
		"Select a bounded project folder.",
	);
	const firstRoot = files[0]?.webkitRelativePath.split("/")[0];
	check(firstRoot, "Select the project folder rather than individual files.");
	return files.map((file) => {
		const parts = file.webkitRelativePath.split("/");
		check(
			parts[0] === firstRoot && parts.length >= 2,
			"Selected files must belong to one project folder.",
		);
		return {
			path: `apps/${project}/${parts.slice(1).join("/")}`.normalize("NFC"),
			file,
		};
	});
}
function byteCompare(left: string, right: string): number {
	const a = encoder.encode(left);
	const b = encoder.encode(right);
	for (let i = 0; i < Math.min(a.length, b.length); i++) {
		const diff = (a[i] ?? 0) - (b[i] ?? 0);
		if (diff) return diff;
	}
	return a.length - b.length;
}
async function hashBlob(
	file: ArtifactBlob,
	signal?: AbortSignal,
): Promise<string> {
	const hash = sha256.create();
	try {
		for (let offset = 0; offset < file.size; offset += 1024 * 1024) {
			cancelled(signal);
			const bytes = new Uint8Array(
				await file.slice(offset, offset + 1024 * 1024).arrayBuffer(),
			);
			hash.update(bytes);
			bytes.fill(0);
		}
		cancelled(signal);
		return hex(hash.digest());
	} finally {
		hash.destroy();
	}
}
function parseAssetJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		throw new Error("Selected asset JSON is invalid.");
	}
}
export function parseProjectArtifactAssets(
	text: string,
): ProjectArtifactAssets {
	check(
		encoder.encode(text).length <= 65536,
		"Asset selection exceeds its size limit.",
	);
	const value = parseAssetJson(text) as ProjectArtifactAssets;
	check(
		value &&
			typeof value === "object" &&
			!Array.isArray(value) &&
			Object.keys(value).every((k) => k === "bit_pins" || k === "package_pins"),
		"Invalid asset selection fields.",
	);
	const bits = value.bit_pins ?? [];
	const packages = value.package_pins ?? [];
	check(
		Array.isArray(bits) &&
			Array.isArray(packages) &&
			bits.length <= 256 &&
			packages.length <= 256,
		"Too many selected project assets.",
	);
	const ids = new Set<string>();
	const digests = /^[a-f0-9]{64}$/;
	for (const pin of bits) {
		check(
			pin &&
				Object.keys(pin).length === 2 &&
				typeof pin.bit_id === "string" &&
				typeof pin.metadata_sha256 === "string" &&
				digests.test(pin.metadata_sha256),
			"Invalid selected Bit pin.",
		);
		projectId(pin.bit_id);
		check(!ids.has(pin.bit_id), "Duplicate selected Bit.");
		ids.add(pin.bit_id);
	}
	ids.clear();
	for (const pin of packages) {
		check(
			pin &&
				Object.keys(pin).length === 4 &&
				typeof pin.package_id === "string" &&
				typeof pin.version === "string" &&
				typeof pin.wasm_sha256 === "string" &&
				typeof pin.manifest_sha256 === "string" &&
				digests.test(pin.wasm_sha256) &&
				digests.test(pin.manifest_sha256),
			"Invalid selected WASM package pin.",
		);
		projectId(pin.package_id);
		projectId(pin.version);
		const key = `${pin.package_id}/${pin.version}`;
		check(!ids.has(key), "Duplicate selected WASM package.");
		ids.add(key);
	}
	return {
		bit_pins: bits.map((p) => ({ ...p })),
		package_pins: packages.map((p) => ({ ...p })),
	};
}
type SelectedAsset = { size: number | null; sha256: string };
async function selectedAssets(
	files: readonly ArtifactInput[],
	assets: ProjectArtifactAssets,
	signal?: AbortSignal,
): Promise<Map<string, SelectedAsset>> {
	const selected = new Map<string, SelectedAsset>();
	for (const pin of assets.bit_pins) {
		cancelled(signal);
		const path = `bits/metadata/${pin.bit_id}.json`;
		const input = files.find((f) => f.path === path);
		check(
			input && input.file.size > 0 && input.file.size <= 16 * 1024 * 1024,
			"Selected Bit metadata is missing or exceeds its bound.",
		);
		const bytes = new Uint8Array(await input.file.arrayBuffer());
		check(
			hex(sha256(bytes)) === pin.metadata_sha256,
			"Selected Bit metadata digest differs.",
		);
		const wrapper = parseAssetJson(
			new TextDecoder("utf-8", { fatal: true }).decode(bytes),
		) as {
			bit: Record<string, unknown>;
			dependencies: Record<string, unknown>[];
			artifacts: ProjectArtifactFile[];
		};
		check(
			wrapper &&
				Object.keys(wrapper).length === 3 &&
				wrapper.bit?.id === pin.bit_id &&
				Array.isArray(wrapper.dependencies) &&
				wrapper.dependencies.length <= 2048 &&
				Array.isArray(wrapper.artifacts) &&
				wrapper.artifacts.length <= 2048,
			"Selected Bit metadata shape differs.",
		);
		const expected = new Map<string, number | null>();
		for (const bit of [wrapper.bit, ...wrapper.dependencies]) {
			if (bit.file_name === null || bit.file_name === undefined) continue;
			check(
				typeof bit.file_name === "string" &&
					typeof bit.hash === "string" &&
					!bit.hash.includes("/") &&
					bit.hash !== "metadata" &&
					bit.hash !== "deps-cache",
				"Invalid selected Bit asset path.",
			);
			const asset = `bits/${bit.hash}/${bit.file_name}`;
			validateRelativePath(asset);
			const size = bit.size ?? null;
			check(
				size === null ||
					(typeof size === "number" && Number.isSafeInteger(size) && size >= 0),
				"Invalid selected Bit asset size.",
			);
			check(
				!expected.has(asset) || expected.get(asset) === size,
				"Selected Bit assets disagree on size.",
			);
			expected.set(asset, size as number | null);
		}
		check(
			expected.size === wrapper.artifacts.length,
			"Selected Bit assets must cover exact metadata paths.",
		);
		const seen = new Set<string>();
		for (const artifact of wrapper.artifacts) {
			check(
				artifact &&
					Object.keys(artifact).length === 3 &&
					typeof artifact.path === "string" &&
					expected.has(artifact.path) &&
					!seen.has(artifact.path) &&
					Number.isSafeInteger(artifact.size) &&
					artifact.size >= 0 &&
					artifact.size <= MAX_FILE_BYTES &&
					typeof artifact.sha256 === "string" &&
					/^[a-f0-9]{64}$/.test(artifact.sha256),
				"Invalid selected Bit artifact digest.",
			);
			const size = expected.get(artifact.path);
			check(
				size === null || size === artifact.size,
				"Selected Bit artifact size differs.",
			);
			seen.add(artifact.path);
			const old = selected.get(artifact.path);
			check(
				!old || (old.size === artifact.size && old.sha256 === artifact.sha256),
				"Selected Bits disagree on an artifact digest.",
			);
			selected.set(artifact.path, {
				size: artifact.size,
				sha256: artifact.sha256,
			});
		}
		selected.set(path, { size: input.file.size, sha256: pin.metadata_sha256 });
		bytes.fill(0);
	}
	for (const pin of assets.package_pins) {
		for (const [name, digest] of [
			["manifest.json", pin.manifest_sha256],
			["module.wasm", pin.wasm_sha256],
		] as const) {
			const path = `packages/${pin.package_id}/${pin.version}/${name}`;
			validateRelativePath(path);
			selected.set(path, { size: null, sha256: digest });
		}
	}
	return selected;
}
/** Select only pinned assets from an object-store folder containing bits/ and packages/. */
export async function selectedProjectAssetFiles(
	files: readonly File[],
	assets: ProjectArtifactAssets,
	signal?: AbortSignal,
): Promise<ArtifactInput[]> {
	const selection = parseProjectArtifactAssets(JSON.stringify(assets));
	check(files.length <= 100000, "Asset source folder has too many entries.");
	const root = files[0]?.webkitRelativePath.split("/")[0];
	check(root, "Select the object-store folder containing bits and packages.");
	const inputs = files.map((file) => {
		const parts = file.webkitRelativePath.split("/");
		check(
			parts[0] === root && parts.length >= 2,
			"Asset files must belong to one selected folder.",
		);
		return { path: parts.slice(1).join("/").normalize("NFC"), file };
	});
	const selected = await selectedAssets(inputs, selection, signal);
	const result = inputs.filter((file) => selected.has(file.path));
	check(
		result.length === selected.size,
		"Selected offline assets are missing from this folder.",
	);
	return result;
}
export async function prepareProjectArtifact(
	project: string,
	inputs: readonly ArtifactInput[],
	signal?: AbortSignal,
	assets: ProjectArtifactAssets = { bit_pins: [], package_pins: [] },
	source: "offline" | "online" = "offline",
): Promise<PreparedProjectArtifact> {
	projectId(project);
	const selection = parseProjectArtifactAssets(JSON.stringify(assets));
	const selected = await selectedAssets(inputs, selection, signal);
	check(
		inputs.length > 0 && inputs.length <= MAX_FILES,
		"Project file count exceeds its limit.",
	);
	const files = inputs
		.map(({ path, file }) => ({ path, file }))
		.sort((a, b) => byteCompare(a.path, b.path));
	const seen = new Set<string>();
	let total = 0;
	for (const input of files) {
		validateRelativePath(input.path);
		check(
			source !== "online" ||
				!input.path.startsWith("apps/") ||
				input.path === `apps/${project}/online-source.json`,
			"Online dependency artifacts cannot contain offline project data.",
		);
		check(
			input.path.startsWith(`apps/${project}/`) || selected.has(input.path),
			"Project contains an unselected asset.",
		);
		if (selected.has(input.path)) {
			const size = selected.get(input.path)?.size;
			check(
				size === null || size === input.file.size,
				"Selected Bit asset size differs.",
			);
		}
		const folded = input.path.toLowerCase().normalize("NFC");
		check(!seen.has(folded), "Project has colliding file names.");
		seen.add(folded);
		check(
			Number.isSafeInteger(input.file.size) &&
				input.file.size >= 0 &&
				input.file.size <= MAX_FILE_BYTES,
			"Project file exceeds its size limit.",
		);
		total += input.file.size;
	}
	for (const path of seen) {
		const parts = path.split("/");
		parts.pop();
		while (parts.length) {
			check(
				!seen.has(parts.join("/")),
				"Project has a file and directory with the same name.",
			);
			parts.pop();
		}
	}
	check(total <= MAX_BYTES, "Project exceeds its upload size limit.");
	check(
		files.some(
			(file) =>
				file.path ===
					`apps/${project}/${source === "online" ? "online-source.json" : "manifest.app"}` &&
				file.file.size > 0,
		),
		"Select the project folder containing manifest.app.",
	);
	const manifest: ProjectArtifactManifest = {
		version: 1,
		project_id: project,
		...(source === "online" ? { source: "online" as const } : {}),
		files: [],
	};
	for (const input of files)
		manifest.files.push({
			path: input.path,
			size: input.file.size,
			sha256: await hashBlob(input.file, signal),
		});
	for (const [path, expected] of selected)
		check(
			manifest.files.some(
				(file) =>
					file.path === path &&
					file.sha256 === expected.sha256 &&
					(expected.size === null || file.size === expected.size),
			),
			"Selected asset digest differs.",
		);
	for (const path of selected.keys())
		check(
			manifest.files.some((file) => file.path === path),
			"A selected project asset is missing.",
		);
	for (const pin of selection.bit_pins)
		check(
			manifest.files.some(
				(file) =>
					file.path === `bits/metadata/${pin.bit_id}.json` &&
					file.sha256 === pin.metadata_sha256,
			),
			"Selected Bit metadata digest differs.",
		);
	for (const pin of selection.package_pins)
		for (const [name, digest] of [
			["manifest.json", pin.manifest_sha256],
			["module.wasm", pin.wasm_sha256],
		])
			check(
				manifest.files.some(
					(file) =>
						file.path === `packages/${pin.package_id}/${pin.version}/${name}` &&
						file.sha256 === digest &&
						file.size > 0,
				),
				"Selected WASM package digest differs.",
			);
	if (selection.bit_pins.length) manifest.bit_pins = selection.bit_pins;
	if (selection.package_pins.length)
		manifest.package_pins = selection.package_pins;
	const bytes = encoder.encode(JSON.stringify(manifest));
	check(
		bytes.length <= MAX_MANIFEST_BYTES,
		"Project manifest exceeds its size limit.",
	);
	return {
		descriptor: {
			project_id: project,
			...(source === "online" ? { source: "online" as const } : {}),
			manifest_sha256: hex(sha256(bytes)),
			manifest_size: bytes.length,
			file_count: files.length,
			total_bytes: total,
		},
		manifest: bytes,
		files,
	};
}
function sameDescriptor(
	value: unknown,
	expected: ProjectArtifactDescriptor,
): boolean {
	if (!value || typeof value !== "object") return false;
	const v = value as Record<string, unknown>;
	return (
		Object.keys(expected).every(
			(key) => v[key] === expected[key as keyof ProjectArtifactDescriptor],
		) && Object.keys(v).length === Object.keys(expected).length
	);
}
function transferStatus(
	value: unknown,
	descriptor: ProjectArtifactDescriptor,
	transferId: string,
	index: number | null,
	size: number,
): ArtifactTransferStatus {
	check(value && typeof value === "object", "Invalid artifact status.");
	const status = value as ArtifactTransferStatus;
	check(
		status.transfer_id === transferId &&
			sameDescriptor(status.descriptor, descriptor) &&
			["receiving", "committed", "aborted"].includes(status.state) &&
			Number.isSafeInteger(status.expires_at) &&
			status.expires_at > 0 &&
			typeof status.manifest_ready === "boolean" &&
			status.file_index === index &&
			Number.isSafeInteger(status.offset) &&
			status.offset >= 0 &&
			status.offset <= size &&
			typeof status.complete === "boolean" &&
			(!status.complete || status.offset === size) &&
			(status.project_path === null || typeof status.project_path === "string"),
		"Device artifact status does not match this upload.",
	);
	if (status.state === "committed")
		check(
			typeof status.project_path === "string" &&
				status.project_path.startsWith("/") &&
				status.project_path.endsWith(
					`/projects/${descriptor.project_id}/revisions/${descriptor.manifest_sha256}`,
				),
			"Committed project path differs from this revision.",
		);
	return status;
}
export async function uploadProjectArtifact(options: {
	prepared: PreparedProjectArtifact;
	request: ArtifactManagementCall;
	transferId?: string;
	signal?: AbortSignal;
	onProgress?: (progress: ArtifactProgress) => void;
}): Promise<ArtifactTransferStatus> {
	const { prepared, request, signal, onProgress } = options;
	const transferId = options.transferId ?? crypto.randomUUID();
	id(transferId);
	const { descriptor } = prepared;
	projectId(descriptor.project_id);
	check(
		hex(sha256(prepared.manifest)) === descriptor.manifest_sha256 &&
			prepared.manifest.length === descriptor.manifest_size &&
			prepared.files.length === descriptor.file_count,
		"Prepared artifact has changed.",
	);
	const call = async (
		payload: Record<string, unknown>,
		index: number | null,
		size: number,
		operationId?: string,
	) => {
		cancelled(signal);
		const response = await request(
			{ type: "artifact", request: payload },
			operationId,
		);
		check(
			["accepted", "completed"].includes(response.state),
			"Device rejected the artifact request.",
		);
		return transferStatus(response.result, descriptor, transferId, index, size);
	};
	let uploaded = 0;
	let completed = 0;
	const progress = (phase: ArtifactProgress["phase"]) =>
		onProgress?.({
			transferId,
			phase,
			uploadedBytes: uploaded,
			totalBytes: descriptor.total_bytes,
			completedFiles: completed,
			totalFiles: descriptor.file_count,
		});
	try {
		let current = options.transferId
			? await call(
					{
						kind: "status",
						project_id: descriptor.project_id,
						transfer_id: transferId,
						file_index: null,
					},
					null,
					descriptor.manifest_size,
				)
			: await call(
					{ kind: "begin", descriptor },
					null,
					descriptor.manifest_size,
					transferId,
				);
		if (current.state === "committed") return current;
		check(
			current.state === "receiving",
			"Artifact transfer is no longer receiving files.",
		);
		const send = async (
			index: number | null,
			file: ArtifactBlob,
			status: ArtifactTransferStatus,
		) => {
			let value = status;
			while (!value.complete) {
				cancelled(signal);
				const offset =
					value.offset === file.size
						? Math.max(0, file.size - ARTIFACT_CHUNK_BYTES)
						: value.offset;
				const bytes = new Uint8Array(
					await file
						.slice(offset, Math.min(file.size, offset + ARTIFACT_CHUNK_BYTES))
						.arrayBuffer(),
				);
				const next = await call(
					{
						kind: "chunk",
						project_id: descriptor.project_id,
						transfer_id: transferId,
						file_index: index,
						offset,
						data: base64(bytes),
					},
					index,
					file.size,
				);
				bytes.fill(0);
				check(
					next.offset > value.offset || next.complete,
					"Device did not advance the artifact upload.",
				);
				if (index !== null) {
					uploaded += next.offset - value.offset;
					progress("files");
				}
				value = next;
			}
			return value;
		};
		progress("manifest");
		current = await send(null, new Blob([prepared.manifest]), current);
		check(
			current.manifest_ready,
			"Device did not verify the artifact manifest.",
		);
		for (let index = 0; index < prepared.files.length; index++) {
			const input = prepared.files[index];
			check(input, "Missing prepared project file.");
			const status = await call(
				{
					kind: "status",
					project_id: descriptor.project_id,
					transfer_id: transferId,
					file_index: index,
				},
				index,
				input.file.size,
			);
			uploaded += status.offset;
			await send(index, input.file, status);
			completed++;
			progress("files");
		}
		progress("commit");
		const result = await call(
			{
				kind: "commit",
				project_id: descriptor.project_id,
				transfer_id: transferId,
			},
			null,
			descriptor.manifest_size,
		);
		check(
			result.state === "committed",
			"Device has not committed this project revision.",
		);
		return result;
	} catch {
		throw new ArtifactUploadError(transferId);
	}
}
export async function abortProjectArtifact(
	request: ArtifactManagementCall,
	project: string,
	transferId: string,
): Promise<void> {
	projectId(project);
	id(transferId);
	const response = await request({
		type: "artifact",
		request: { kind: "abort", project_id: project, transfer_id: transferId },
	});
	check(
		["accepted", "completed"].includes(response.state) &&
			response.result &&
			typeof response.result === "object" &&
			(response.result as { state?: unknown }).state === "aborted",
		"Project upload abort is unconfirmed.",
	);
}
export async function prepareOnlineProjectCache(
	request: ArtifactManagementCall,
	project: string,
): Promise<string> {
	projectId(project);
	const response = await request({
		type: "artifact",
		request: { kind: "prepare_online", project_id: project },
	});
	const value = response.result as { project_path?: unknown };
	check(
		["accepted", "completed"].includes(response.state) &&
			value &&
			typeof value.project_path === "string" &&
			value.project_path.startsWith("/") &&
			value.project_path.endsWith(`/projects/${project}/online-cache`),
		"Online project cache creation is unconfirmed.",
	);
	return value.project_path;
}
