import { sha256 } from "@noble/hashes/sha2";
import type { IBit } from "../schema/bit/bit";
import type { ModelAssetDescriptor } from "./models";
import {
	type NativeArtifactUploadCode,
	NativeArtifactUploadError,
	type NativeArtifactUploadPhase,
} from "./native-errors";
import { type DeviceAccountScope, accountStorageKey } from "./storage";
import {
	type ManagementFailureDiagnostic,
	ManagementReadError,
	managementFailureDiagnostic,
} from "./transport";
import { DeviceTunnelError, type TunnelArtifactUpload } from "./tunnel";
import { type ManagementRejection, managementRejection } from "./types";
import { LiveCallError } from "./workspace/errors";

export const ARTIFACT_CHUNK_BYTES = 8192;
const ARTIFACT_TRANSFER_TTL_SECONDS = 86_400;
const MAX_FILES = 8192;
const MAX_MANIFEST_BYTES = 2 * 1024 ** 2;
// Placement configurations accept at most these pins; larger selections could never deploy.
const MAX_BIT_PINS = 256;
const MAX_PACKAGE_PINS = 64;
const MAX_PACKAGED_DEPENDENCIES = 2048;
/** Artifact files and model-store assets of one packaged Bit together. */
const MAX_PACKAGED_FILES = 2048;
const MAX_MODEL_ASSET_SOURCES = 8;
const MAX_MODEL_ASSET_SOURCE_LENGTH = 2048;
const METADATA_V1_FIELDS = ["bit", "dependencies", "artifacts"];
const METADATA_V2_FIELDS = [
	"version",
	"bit",
	"dependencies",
	"assets",
	"artifacts",
];
const DESCRIPTOR_FIELDS = ["digest", "size", "file_name", "sources"];
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
/** One model-store file of a packaged Bit; a Bit with several lists its parts in load order. */
export type PackagedBitAsset = {
	bit_id: string;
	descriptor: ModelAssetDescriptor;
};
/**
 * `bits/metadata/<bit_id>.json` (`PackagedBitMetadata` in
 * packages/device-protocol/src/artifact.rs). v1 has no `version` and carries
 * every Bit file as an artifact. v2 names model-store `assets` the device
 * acquires itself; small files may still travel as `artifacts`. Every Bit
 * file is listed exactly once.
 */
export type PackagedBitMetadata =
	| {
			bit: Record<string, unknown>;
			dependencies: Record<string, unknown>[];
			artifacts: ProjectArtifactFile[];
	  }
	| {
			version: 2;
			bit: Record<string, unknown>;
			dependencies?: Record<string, unknown>[];
			assets: PackagedBitAsset[];
			artifacts?: ProjectArtifactFile[];
	  };
/** A model asset the artifact names but does not carry: the device fetches it, or this computer sends it. */
export type PreparedModelAsset = {
	/** The pinned Bit whose metadata names it, and that Bit's hub. */
	pin: string;
	pinHub?: string;
	bitId: string;
	/** The owning Bit's file lives at `<bitHash>/<file_name>` in a Bit store. */
	bitHash: string;
	descriptor: ModelAssetDescriptor;
};
export type PreparedProjectArtifact = {
	descriptor: ProjectArtifactDescriptor;
	manifest: Uint8Array<ArrayBuffer>;
	files: readonly ArtifactInput[];
	/** Root Bit metadata verified against the deployment pins, including workflow references. */
	bits?: readonly IBit[];
	/** Model-store assets of v2 Bit metadata, one per digest, and the pins that name them; absent for v1. */
	models?: { pins: ProjectBitPin[]; assets: PreparedModelAsset[] };
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
const TUNNEL_UPLOAD_CAUSES = [
	"timeout",
	"invalid_reply",
	"open_timeout",
	"heartbeat_timeout",
	"renewal_timeout",
	"connection_closed",
	"invalid_frame",
	"capacity",
	"expired",
	"cancelled",
	"stream_failed",
	"unauthorized",
	"unsupported",
] as const;

/** Local upload position and fixed failure codes; excludes paths, credentials and raw errors. */
export interface ArtifactUploadDiagnostic {
	phase: "begin" | "resume" | "manifest" | "file_status" | "file" | "commit";
	cause:
		| (typeof TUNNEL_UPLOAD_CAUSES)[number]
		| NativeArtifactUploadCode
		| "tunnel_failed"
		| "management_failed"
		| "upload_failed"
		| "rejected";
	/** Null identifies the manifest; file indices are zero-based. */
	fileIndex?: number | null;
	/** The start of the attempted stream, not a claim that later bytes arrived. */
	offset?: number;
	nativePhase?: NativeArtifactUploadPhase;
}

function uploadFailureCause(
	error: unknown,
	diagnostic: ManagementFailureDiagnostic | undefined,
): ArtifactUploadDiagnostic["cause"] {
	if (error instanceof NativeArtifactUploadError) return error.code;
	if (error instanceof ManagementReadError) return error.code;
	if (error instanceof DeviceTunnelError)
		return (
			TUNNEL_UPLOAD_CAUSES.find((code) => code === error.code) ??
			"tunnel_failed"
		);
	if (error instanceof Error && error.name === "AbortError") return "cancelled";
	return diagnostic ? "management_failed" : "upload_failed";
}
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
		readonly diagnostic?: ManagementFailureDiagnostic,
		readonly uploadDiagnostic?: ArtifactUploadDiagnostic,
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
export async function hashBlob(
	file: ArtifactBlob,
	signal?: AbortSignal,
	createHash: () => {
		update(bytes: Uint8Array): void;
		digest(): Uint8Array;
		destroy(): void;
	} = sha256.create,
): Promise<string> {
	const hash = createHash();
	try {
		for (let offset = 0; offset < file.size; ) {
			cancelled(signal);
			const end = Math.min(file.size, offset + 1024 * 1024);
			const bytes = new Uint8Array(await file.slice(offset, end).arrayBuffer());
			hash.update(bytes);
			bytes.fill(0);
			offset = end;
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
/** Both versions of packaged Bit metadata, as one list of files and one of assets. */
type PackagedFiles = {
	bit: Record<string, unknown>;
	dependencies: unknown[];
	artifacts: ProjectArtifactFile[];
	assets: unknown[];
};
/** The one file location a Bit names, `bits/<hash>/<file_name>`. */
type BitFile = {
	id?: string;
	hash: string;
	path: string;
	fileName: string;
	size: number | null;
};
type StoredBit = { file: BitFile; parts: ModelAssetDescriptor[] };
function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}
function packagedFiles(value: unknown, bitId: string): PackagedFiles {
	const wrapper = isRecord(value) ? value : {};
	const v2 = Object.hasOwn(wrapper, "version");
	const fields = v2 ? METADATA_V2_FIELDS : METADATA_V1_FIELDS;
	const dependencies = wrapper.dependencies ?? (v2 ? [] : undefined);
	const artifacts = wrapper.artifacts ?? (v2 ? [] : undefined);
	const assets = v2 ? wrapper.assets : [];
	check(
		Object.keys(wrapper).every((key) => fields.includes(key)) &&
			(!v2 || wrapper.version === 2) &&
			isRecord(wrapper.bit) &&
			wrapper.bit.id === bitId &&
			Array.isArray(dependencies) &&
			dependencies.length <= MAX_PACKAGED_DEPENDENCIES &&
			Array.isArray(artifacts) &&
			Array.isArray(assets) &&
			artifacts.length + assets.length <= MAX_PACKAGED_FILES,
		"Selected Bit metadata shape differs.",
	);
	return {
		bit: wrapper.bit as Record<string, unknown>,
		dependencies: dependencies as unknown[],
		artifacts: artifacts as ProjectArtifactFile[],
		assets: assets as unknown[],
	};
}
function bitFile(bit: unknown): BitFile | undefined {
	if (!isRecord(bit) || bit.file_name === null || bit.file_name === undefined)
		return undefined;
	const { hash, file_name: fileName } = bit;
	check(
		typeof fileName === "string" &&
			typeof hash === "string" &&
			!hash.includes("/") &&
			hash !== "metadata" &&
			hash !== "deps-cache",
		"Invalid selected Bit asset path.",
	);
	const path = `bits/${hash}/${fileName}`;
	validateRelativePath(path);
	const size = bit.size ?? null;
	check(
		size === null ||
			(typeof size === "number" && Number.isSafeInteger(size) && size >= 0),
		"Invalid selected Bit asset size.",
	);
	return {
		...(typeof bit.id === "string" ? { id: bit.id } : {}),
		hash,
		path,
		fileName,
		size: size as number | null,
	};
}
function modelAssetSource(source: unknown): boolean {
	if (
		typeof source !== "string" ||
		source.length > MAX_MODEL_ASSET_SOURCE_LENGTH ||
		source.includes("#")
	)
		return false;
	try {
		const url = new URL(source);
		return (
			url.protocol === "https:" &&
			url.hostname !== "" &&
			!url.username &&
			!url.password &&
			(!source.includes("?") || url.search === "?download=true")
		);
	} catch {
		return false;
	}
}
/** `ModelAssetDescriptor::validate`: a pinned digest, a valid size, a safe file name, public HTTPS sources. */
function modelAssetDescriptor(value: unknown): ModelAssetDescriptor {
	const descriptor = isRecord(value) ? value : {};
	const digest = isRecord(descriptor.digest) ? descriptor.digest : {};
	const { size, file_name: fileName } = descriptor;
	const sources = descriptor.sources ?? [];
	check(
		Object.keys(descriptor).every((key) => DESCRIPTOR_FIELDS.includes(key)) &&
			Object.keys(digest).length === 2 &&
			(digest.algorithm === "sha256" || digest.algorithm === "blake3") &&
			typeof digest.hex === "string" &&
			/^[a-f0-9]{64}$/.test(digest.hex) &&
			typeof size === "number" &&
			Number.isSafeInteger(size) &&
			size > 0 &&
			typeof fileName === "string" &&
			Array.isArray(sources) &&
			sources.length <= MAX_MODEL_ASSET_SOURCES &&
			sources.every(modelAssetSource),
		"Invalid selected model asset.",
	);
	validateRelativePath(fileName as string);
	return {
		digest: {
			algorithm: digest.algorithm as "sha256" | "blake3",
			hex: digest.hex as string,
		},
		size: size as number,
		file_name: fileName as string,
		...(sources.length ? { sources: sources as string[] } : {}),
	};
}
/** The Bit each id names; an id two file-backed Bits share names none. */
function assetOwners(files: readonly BitFile[]): Map<string, BitFile | null> {
	const owners = new Map<string, BitFile | null>();
	for (const file of files)
		if (file.id !== undefined)
			owners.set(file.id, owners.has(file.id) ? null : file);
	return owners;
}
/** Parts in load order: the first carries the Bit's file name, no two share a name or digest. */
function checkParts(file: BitFile, parts: readonly ModelAssetDescriptor[]) {
	check(
		parts[0]?.file_name === file.fileName,
		"The first model asset of a selected Bit has another file name.",
	);
	parts.forEach((part, index) =>
		check(
			parts
				.slice(0, index)
				.every(
					(earlier) =>
						earlier.file_name !== part.file_name &&
						(earlier.digest.algorithm !== part.digest.algorithm ||
							earlier.digest.hex !== part.digest.hex),
				),
			"A selected Bit lists a model asset twice.",
		),
	);
	check(
		parts.length !== 1 || file.size === null || file.size === parts[0]?.size,
		"A model asset's size differs from its selected Bit.",
	);
}
/** The Bits whose bytes are model-store assets, with their parts. */
function storedBits(
	assets: readonly unknown[],
	files: readonly BitFile[],
): Map<string, StoredBit> {
	const owners = assetOwners(files);
	const stored = new Map<string, StoredBit>();
	for (const asset of assets) {
		check(
			isRecord(asset) &&
				Object.keys(asset).length === 2 &&
				typeof asset.bit_id === "string",
			"Invalid selected model asset.",
		);
		const descriptor = modelAssetDescriptor(asset.descriptor);
		const owner = owners.get(asset.bit_id as string);
		check(owner, "A model asset belongs to an unknown or ambiguous Bit.");
		const entry = stored.get(asset.bit_id as string) ?? {
			file: owner,
			parts: [],
		};
		entry.parts.push(descriptor);
		stored.set(asset.bit_id as string, entry);
	}
	for (const { file, parts } of stored.values()) checkParts(file, parts);
	return stored;
}
/** Locations whose bytes travel in the artifact, with the size their Bits declare. */
function artifactLocations(
	files: readonly BitFile[],
	stored: ReadonlyMap<string, StoredBit>,
): Map<string, number | null> {
	const expected = new Map<string, number | null>();
	const storePaths = new Set<string>();
	for (const file of files) {
		if (file.id !== undefined && stored.has(file.id)) {
			storePaths.add(file.path);
			continue;
		}
		check(
			!expected.has(file.path) || expected.get(file.path) === file.size,
			"Selected Bit assets disagree on size.",
		);
		expected.set(file.path, file.size);
	}
	check(
		[...expected.keys()].every((path) => !storePaths.has(path)),
		"A selected Bit file is listed as an artifact and as model assets.",
	);
	return expected;
}
type SelectedFiles = {
	selected: Map<string, SelectedAsset>;
	bits: IBit[];
	models: PreparedModelAsset[];
	modelPins: ProjectBitPin[];
};
async function selectedAssets(
	files: readonly ArtifactInput[],
	assets: ProjectArtifactAssets,
	signal?: AbortSignal,
): Promise<SelectedFiles> {
	const selected = new Map<string, SelectedAsset>();
	const bits: IBit[] = [];
	const models: PreparedModelAsset[] = [];
	const modelPins: ProjectBitPin[] = [];
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
		const wrapper = packagedFiles(
			parseAssetJson(new TextDecoder("utf-8", { fatal: true }).decode(bytes)),
			pin.bit_id,
		);
		bits.push(wrapper.bit as unknown as IBit);
		const bitFiles = [wrapper.bit, ...wrapper.dependencies].flatMap(
			(bit) => bitFile(bit) ?? [],
		);
		const stored = storedBits(wrapper.assets, bitFiles);
		const { hub } = wrapper.bit;
		for (const [bitId, { file, parts }] of stored)
			for (const descriptor of parts)
				models.push({
					pin: pin.bit_id,
					...(typeof hub === "string" && hub ? { pinHub: hub } : {}),
					bitId,
					bitHash: file.hash,
					descriptor,
				});
		if (stored.size) modelPins.push(pin);
		const expected = artifactLocations(bitFiles, stored);
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
	return { selected, bits, models: uniqueAssets(models), modelPins };
}
/** One entry per digest: Bits that share a file share its download. */
function uniqueAssets(models: readonly PreparedModelAsset[]) {
	const seen = new Set<string>();
	return models.filter(({ descriptor: { digest } }) => {
		const key = `${digest.algorithm}/${digest.hex}`;
		if (seen.has(key)) return false;
		seen.add(key);
		return true;
	});
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
	const { selected } = await selectedAssets(inputs, selection, signal);
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
	const { selected, bits, models, modelPins } = await selectedAssets(
		inputs,
		selection,
		signal,
	);
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
			Number.isSafeInteger(input.file.size) && input.file.size >= 0,
			"Project file has an invalid size.",
		);
		total += input.file.size;
		check(
			Number.isSafeInteger(total),
			"Project size cannot be represented exactly.",
		);
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
		bits,
		...(models.length ? { models: { pins: modelPins, assets: models } } : {}),
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
	upload: (input: TunnelArtifactUpload) => Promise<ArtifactTransferStatus>;
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
	let position: Omit<ArtifactUploadDiagnostic, "cause"> = { phase: "begin" };
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
				undefined,
				{ ...position, cause: "rejected" },
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
	const begin = () => {
		position = { phase: "begin" };
		return call(
			{ kind: "begin", descriptor },
			null,
			descriptor.manifest_size,
			transferId,
		);
	};
	const resume = async () => {
		position = { phase: "resume" };
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
			position = {
				phase: index === null ? "manifest" : "file",
				fileIndex: index,
				offset: status.offset,
			};
			if (status.complete) return status;
			cancelled(signal);
			// A full file without a verified hash needs its final bytes checked again.
			const offset =
				status.offset === file.size
					? Math.max(0, file.size - ARTIFACT_CHUNK_BYTES)
					: status.offset;
			position.offset = offset;
			const next = transferStatus(
				await options.upload({
					projectId: descriptor.project_id,
					transferId,
					fileIndex: index,
					offset,
					file,
					signal,
				}),
				descriptor,
				transferId,
				index,
				file.size,
			);
			check(
				next.state === "receiving" && next.complete,
				"Device did not verify the streamed artifact file.",
			);
			if (index !== null) {
				uploaded += next.offset - status.offset;
				progress("files");
			}
			return next;
		};
		progress("manifest");
		current = await send(null, new Blob([prepared.manifest]), current);
		check(
			current.manifest_ready,
			"Device did not verify the artifact manifest.",
		);
		for (let index = 0; index < prepared.files.length; index++) {
			position = { phase: "file_status", fileIndex: index };
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
		position = { phase: "commit" };
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
		const diagnostic =
			error instanceof LiveCallError
				? error.diagnostic
				: managementFailureDiagnostic(error);
		throw new ArtifactUploadError(transferId, undefined, diagnostic, {
			...position,
			cause: uploadFailureCause(error, diagnostic),
			...(error instanceof NativeArtifactUploadError && error.phase
				? { nativePhase: error.phase }
				: {}),
		});
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
/** Before C8 these hints were not account-scoped; the activity tray imports them once. */
const LEGACY_TRANSFERS_PREFIX = "flow-like.device-artifact-transfers.";
function pendingTransfersKey(
	deviceId: string,
	scope?: DeviceAccountScope,
): string {
	return scope
		? `flow-like.device-transfers.${accountStorageKey(scope)}.${deviceId}`
		: `${LEGACY_TRANSFERS_PREFIX}${deviceId}`;
}
function readPendingTransfers(
	deviceId: string,
	scope?: DeviceAccountScope,
): PendingArtifactTransfer[] {
	try {
		const value: unknown = JSON.parse(
			globalThis.localStorage?.getItem(pendingTransfersKey(deviceId, scope)) ??
				"[]",
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
	scope?: DeviceAccountScope,
): void {
	try {
		const key = pendingTransfersKey(deviceId, scope);
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
/**
 * Unfinished uploads this browser began on a device, kept so a later session can resume or abort them.
 * With `scope` the hints belong to one account and hub (C8); without it, to the legacy per-device key.
 */
export function pendingArtifactTransfers(
	deviceId: string,
	project?: string,
	scope?: DeviceAccountScope,
): PendingArtifactTransfer[] {
	return readPendingTransfers(deviceId, scope).filter(
		(transfer) => project === undefined || transfer.project_id === project,
	);
}
/** A later confirmation upgrades an entry; an unconfirmed report never downgrades one. */
export function rememberArtifactTransfer(
	deviceId: string,
	transfer: Omit<PendingArtifactTransfer, "expires_at">,
	scope?: DeviceAccountScope,
): void {
	const transfers = readPendingTransfers(deviceId, scope);
	const existing = transfers.find(
		(entry) => entry.transfer_id === transfer.transfer_id,
	);
	if (existing && (existing.confirmed || !transfer.confirmed)) return;
	writePendingTransfers(
		deviceId,
		[
			...transfers.filter((entry) => entry !== existing),
			{
				...transfer,
				expires_at:
					existing?.expires_at ??
					Date.now() / 1000 + ARTIFACT_TRANSFER_TTL_SECONDS,
			},
		],
		scope,
	);
}
export function forgetArtifactTransfer(
	deviceId: string,
	transferId: string,
	scope?: DeviceAccountScope,
): void {
	writePendingTransfers(
		deviceId,
		readPendingTransfers(deviceId, scope).filter(
			(entry) => entry.transfer_id !== transferId,
		),
		scope,
	);
}
/** Devices that still have pre-C8 upload hints in this browser. */
export function legacyArtifactTransferDevices(): string[] {
	try {
		const storage = globalThis.localStorage;
		if (!storage) return [];
		const devices: string[] = [];
		for (let index = 0; index < storage.length; index++) {
			const key = storage.key(index);
			if (key?.startsWith(LEGACY_TRANSFERS_PREFIX))
				devices.push(key.slice(LEGACY_TRANSFERS_PREFIX.length));
		}
		return devices;
	} catch {
		return [];
	}
}
/** Reads a device's pre-C8 upload hints once and deletes them; expired hints are dropped. */
export function takeLegacyArtifactTransfers(
	deviceId: string,
): PendingArtifactTransfer[] {
	const transfers = readPendingTransfers(deviceId);
	writePendingTransfers(deviceId, []);
	return transfers;
}
/**
 * The device's view of an upload this browser began, for resuming it after a reload.
 * Null when the device no longer holds the transfer (it answers an unknown one with a coded `failed`).
 */
export async function readArtifactTransfer(
	request: ArtifactManagementCall,
	transfer: Pick<
		PendingArtifactTransfer,
		"transfer_id" | "project_id" | "manifest_sha256"
	>,
	signal?: AbortSignal,
): Promise<ArtifactTransferStatus | null> {
	projectId(transfer.project_id);
	id(transfer.transfer_id);
	const response = await requestArtifact(
		request,
		{
			type: "artifact",
			request: {
				kind: "status",
				project_id: transfer.project_id,
				transfer_id: transfer.transfer_id,
				file_index: null,
			},
		},
		undefined,
		signal,
	);
	const rejection = managementRejection(response);
	if (rejection?.code === "failed") return null;
	if (rejection)
		throw new Error(
			`The device refused to report upload ${transfer.transfer_id}: ${rejection.error}`,
		);
	const status = response.result as Partial<ArtifactTransferStatus> | null;
	check(
		["accepted", "completed"].includes(response.state) &&
			status?.transfer_id === transfer.transfer_id &&
			status.descriptor?.project_id === transfer.project_id &&
			status.descriptor.manifest_sha256 === transfer.manifest_sha256 &&
			["receiving", "committed", "aborted"].includes(status.state ?? "") &&
			Number.isSafeInteger(status.expires_at),
		`The device's status of upload ${transfer.transfer_id} does not match this upload.`,
	);
	return status as ArtifactTransferStatus;
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
