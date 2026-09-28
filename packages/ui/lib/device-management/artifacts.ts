import { sha256 } from "@noble/hashes/sha2";
import { type ManagementRejection, managementRejection } from "./types";

export const ARTIFACT_CHUNK_BYTES = 8192;
const ARTIFACT_TRANSFER_TTL_SECONDS = 86_400;
const MAX_FILES = 8192;
const MAX_FILE_BYTES = 4 * 1024 ** 3;
const MAX_BYTES = 8 * 1024 ** 3;
const MAX_MANIFEST_BYTES = 2 * 1024 ** 2;
// Placement configurations accept at most these pins; larger selections could never deploy.
const MAX_BIT_PINS = 256;
const MAX_PACKAGE_PINS = 64;
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
function rejectionHint(
	rejection: ManagementRejection,
	transferId: string | undefined,
): string {
	if (rejection.code === "limit")
		return "Abort unfinished uploads on this device or wait for them to finish, then upload again.";
	return transferId
		? "Resuming cannot succeed. Abort this transfer, then prepare and upload the project again."
		: "Resolve the cause, then upload the project again.";
}
function uploadErrorMessage(
	transferId: string | undefined,
	rejection: ManagementRejection | undefined,
): string {
	if (!rejection)
		return "Project upload has no confirmed completion. Reconnect and resume this transfer, or abort it.";
	if (!rejection.retryable)
		return `The device rejected this project upload: ${rejection.error} ${rejectionHint(rejection, transferId)}`;
	return `The device did not complete this upload step: ${rejection.error} Resume this transfer to retry, or abort it.`;
}
/**
 * `transferId` is undefined when the device holds no transfer this upload could resume or abort.
 * `rejection` is the device's coded refusal; a retryable one keeps the resume path.
 */
export class ArtifactUploadError extends Error {
	constructor(
		readonly transferId: string | undefined,
		readonly rejection?: ManagementRejection,
	) {
		super(uploadErrorMessage(transferId, rejection));
		this.name = "ArtifactUploadError";
	}
}
export class ArtifactAbortError extends Error {
	/** Set only for a definitive refusal, which leaves nothing this browser can abort. */
	readonly rejection?: ManagementRejection;
	constructor(
		transferId: string,
		/** The device's coded refusal, retryable or not. */
		readonly response?: ManagementRejection,
		/** The device answered with a refusal; older agents send it without a code. */
		readonly refused = response !== undefined,
	) {
		const rejection = response && !response.retryable ? response : undefined;
		super(
			rejection
				? `The device rejected aborting transfer ${transferId}: ${rejection.error}`
				: `Aborting transfer ${transferId} is unconfirmed.${response ? ` Device response: ${response.error}` : ""} Reconnect and try again.`,
		);
		this.rejection = rejection;
		this.name = "ArtifactAbortError";
	}
}
/**
 * Whether a refused abort leaves nothing this browser should offer to abort again.
 * Devices answer an unknown transfer with a retryable `failed`, so every coded answer except `busy`
 * settles it; an older agent's uncoded refusal settles only a transfer the device never acknowledged.
 */
export function abortRefusalSettles(
	error: unknown,
	confirmed: boolean,
): boolean {
	if (!(error instanceof ArtifactAbortError) || !error.refused) return false;
	return error.response ? error.response.code !== "busy" : !confirmed;
}
const BUSY_RETRY_DELAYS = [250, 500, 1000, 2000, 4000, 8000];
function pause(milliseconds: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		const abort = () => {
			clearTimeout(timer);
			reject(signal?.reason ?? new Error("Artifact transfer cancelled."));
		};
		const timer = setTimeout(() => {
			signal?.removeEventListener("abort", abort);
			resolve();
		}, milliseconds);
		signal?.addEventListener("abort", abort, { once: true });
	});
}
/** The device answers `busy` while another artifact operation holds its lock. */
async function requestArtifact(
	request: ArtifactManagementCall,
	command: Record<string, unknown>,
	operationId?: string,
	signal?: AbortSignal,
): Promise<{ state: string; result: unknown }> {
	for (let attempt = 0; ; attempt++) {
		cancelled(signal);
		const response = await request(command, operationId);
		if (
			managementRejection(response)?.code !== "busy" ||
			attempt >= BUSY_RETRY_DELAYS.length
		)
			return response;
		await pause(BUSY_RETRY_DELAYS[attempt] ?? 0, signal);
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
/** A placement runs one version per node package, so pins with several can never deploy. */
export function assertOneVersionPerPackage(
	pins: readonly Pick<ProjectPackagePin, "package_id">[],
): void {
	const seen = new Set<string>();
	for (const { package_id } of pins) {
		check(
			!seen.has(package_id),
			`This project pins several versions of node package ${package_id}, but a placement runs one version per package. Keep one version in the asset pins and prepare the project again.`,
		);
		seen.add(package_id);
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
			bits.length <= MAX_BIT_PINS &&
			packages.length <= MAX_PACKAGE_PINS,
		`A deployable project pins at most ${MAX_BIT_PINS} Bits and ${MAX_PACKAGE_PINS} WASM packages; this selection has ${Array.isArray(bits) ? bits.length : "invalid"} Bits and ${Array.isArray(packages) ? packages.length : "invalid"} packages.`,
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
	assertOneVersionPerPackage(packages);
	// The device accepts only its canonical serde field order in the hashed manifest.
	return {
		bit_pins: bits.map((pin) => ({
			bit_id: pin.bit_id,
			metadata_sha256: pin.metadata_sha256,
		})),
		package_pins: packages.map((pin) => ({
			package_id: pin.package_id,
			version: pin.version,
			wasm_sha256: pin.wasm_sha256,
			manifest_sha256: pin.manifest_sha256,
		})),
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
				input.path === `apps/${project}/online-source.json` ||
				input.path === `apps/${project}/online-metadata.json`,
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
	/** False when the device never acknowledged `transferId`; a refused status then begins it under that id. */
	confirmed?: boolean;
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
	let started = false;
	const send = (payload: Record<string, unknown>, operationId?: string) =>
		requestArtifact(
			request,
			{ type: "artifact", request: payload },
			operationId,
			signal,
		);
	const accept = (
		response: { state: string; result: unknown },
		index: number | null,
		size: number,
	) => {
		const rejection = managementRejection(response);
		if (rejection)
			// A retryable refusal may follow partial work, so the transfer stays resumable and abortable.
			throw new ArtifactUploadError(
				started || rejection.retryable ? transferId : undefined,
				rejection,
			);
		check(
			["accepted", "completed"].includes(response.state),
			"Device rejected the artifact request.",
		);
		const status = transferStatus(
			response.result,
			descriptor,
			transferId,
			index,
			size,
		);
		started = true;
		return status;
	};
	const call = async (
		payload: Record<string, unknown>,
		index: number | null,
		size: number,
		operationId?: string,
	) => accept(await send(payload, operationId), index, size);
	const begin = () =>
		call(
			{ kind: "begin", descriptor },
			null,
			descriptor.manifest_size,
			transferId,
		);
	const resume = async () => {
		const response = await send({
			kind: "status",
			project_id: descriptor.project_id,
			transfer_id: transferId,
			file_index: null,
		});
		const code = managementRejection(response)?.code;
		// A begin that never reached the device leaves nothing to resume, so the same id begins it now.
		if (
			options.confirmed === false &&
			response.state === "rejected" &&
			(code === undefined || code === "failed")
		)
			return begin();
		return accept(response, null, descriptor.manifest_size);
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
		let current = options.transferId ? await resume() : await begin();
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
	} catch (error) {
		if (error instanceof ArtifactUploadError) throw error;
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
	const response = await requestArtifact(request, {
		type: "artifact",
		request: { kind: "abort", project_id: project, transfer_id: transferId },
	});
	if (
		!["accepted", "completed"].includes(response.state) ||
		!response.result ||
		typeof response.result !== "object" ||
		(response.result as { state?: unknown }).state !== "aborted"
	)
		throw new ArtifactAbortError(
			transferId,
			managementRejection(response),
			response.state === "rejected",
		);
}
export type PendingArtifactTransfer = {
	transfer_id: string;
	project_id: string;
	manifest_sha256: string;
	expires_at: number;
	/** Set once the device reported this transfer; absent for a begin whose reply never arrived. */
	confirmed?: boolean;
};
const MAX_PENDING_TRANSFERS = 32;
function pendingTransfersKey(deviceId: string): string {
	return `flow-like.device-artifact-transfers.${deviceId}`;
}
function readPendingTransfers(deviceId: string): PendingArtifactTransfer[] {
	try {
		const value: unknown = JSON.parse(
			globalThis.localStorage?.getItem(pendingTransfersKey(deviceId)) ?? "[]",
		);
		const now = Date.now() / 1000;
		return Array.isArray(value)
			? value.filter(
					(entry): entry is PendingArtifactTransfer =>
						entry &&
						typeof entry === "object" &&
						typeof entry.transfer_id === "string" &&
						typeof entry.project_id === "string" &&
						typeof entry.manifest_sha256 === "string" &&
						typeof entry.expires_at === "number" &&
						entry.expires_at > now &&
						(entry.confirmed === undefined ||
							typeof entry.confirmed === "boolean"),
				)
			: [];
	} catch {
		return [];
	}
}
function writePendingTransfers(
	deviceId: string,
	transfers: PendingArtifactTransfer[],
): void {
	try {
		const key = pendingTransfersKey(deviceId);
		if (transfers.length)
			globalThis.localStorage?.setItem(
				key,
				JSON.stringify(transfers.slice(-MAX_PENDING_TRANSFERS)),
			);
		else globalThis.localStorage?.removeItem(key);
	} catch {
		// Unavailable storage only loses the resume hint; the device keeps the transfer.
	}
}
/** Unfinished uploads this browser began on a device, kept so a later session can resume or abort them. */
export function pendingArtifactTransfers(
	deviceId: string,
	project?: string,
): PendingArtifactTransfer[] {
	return readPendingTransfers(deviceId).filter(
		(transfer) => project === undefined || transfer.project_id === project,
	);
}
/** A later confirmation upgrades an entry; an unconfirmed report never downgrades one. */
export function rememberArtifactTransfer(
	deviceId: string,
	transfer: Omit<PendingArtifactTransfer, "expires_at">,
): void {
	const transfers = readPendingTransfers(deviceId);
	const existing = transfers.find(
		(entry) => entry.transfer_id === transfer.transfer_id,
	);
	if (existing && (existing.confirmed || !transfer.confirmed)) return;
	writePendingTransfers(deviceId, [
		...transfers.filter((entry) => entry !== existing),
		{
			...transfer,
			expires_at:
				existing?.expires_at ??
				Date.now() / 1000 + ARTIFACT_TRANSFER_TTL_SECONDS,
		},
	]);
}
export function forgetArtifactTransfer(
	deviceId: string,
	transferId: string,
): void {
	writePendingTransfers(
		deviceId,
		readPendingTransfers(deviceId).filter(
			(entry) => entry.transfer_id !== transferId,
		),
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
