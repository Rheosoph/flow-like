import { blake3 } from "@noble/hashes/blake3";
import { sha256 } from "@noble/hashes/sha2";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import { bitContentDigest, bitSources } from "../bit/bit-sources";
import type { IApp } from "../schema/app/app";
import type { IBit } from "../schema/bit/bit";
import {
	type ArtifactInput,
	type PackagedBitAsset,
	type ProjectArtifactAssets,
	type ProjectArtifactFile,
	prepareProjectArtifact,
} from "./artifacts";

import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "./online-metadata";

const MAX_FILE = 64 * 1024 * 1024;
const MAX_TOTAL = 256 * 1024 * 1024;
const MAX_MODEL_ASSET = 64 * 1024 ** 3;
function check(value: unknown, message: string): asserts value {
	if (!value) throw new Error(message);
}
function identifier(value: string) {
	check(
		/^[A-Za-z0-9_.-]{1,128}$/.test(value) && value !== "." && value !== "..",
		"Invalid project dependency identifier.",
	);
}
function hash(bytes: Uint8Array) {
	return Array.from(sha256(bytes), (value) =>
		value.toString(16).padStart(2, "0"),
	).join("");
}

/** Ephemeral download URLs and provider secrets must never enter an artifact. */
export function publicDependencyMetadata<T>(input: T): T {
	function privateUrl(value: unknown): boolean {
		if (typeof value !== "string") return false;
		try {
			const url = new URL(value);
			return Boolean(url.search || url.hash || url.username || url.password);
		} catch {
			return false;
		}
	}
	function visit(value: unknown): unknown {
		if (Array.isArray(value))
			return value.filter((entry) => !privateUrl(entry)).map(visit);
		if (!value || typeof value !== "object") return value;
		return Object.fromEntries(
			Object.entries(value).flatMap(([key, entry]) => {
				const name = key.toLowerCase();
				if (
					[
						"api_key",
						"apikey",
						"access_token",
						"refresh_token",
						"password",
						"secret",
						"authorization",
					].includes(name)
				)
					check(
						entry === null || entry === "",
						"A dependency contains embedded credentials. Configure device credentials separately.",
					);
				if (
					[
						"download_link",
						"download_url",
						"cwasm_download_url",
						"widget_bundle_download_url",
					].includes(name)
				)
					return [];
				if (privateUrl(entry)) return [];
				return [[key, visit(entry)]];
			}),
		);
	}
	return visit(input) as T;
}

async function download(
	url: string,
	limit: number,
	signal?: AbortSignal,
): Promise<Uint8Array> {
	const address = new URL(url);
	check(
		address.protocol === "https:" &&
			!address.username &&
			!address.password &&
			!address.hash,
		"Dependency downloads require a trusted HTTPS URL.",
	);
	const controller = new AbortController();
	const timer = setTimeout(() => controller.abort(), 120_000);
	const abort = () => controller.abort();
	signal?.addEventListener("abort", abort, { once: true });
	try {
		signal?.throwIfAborted();
		const response = await fetch(address, {
			credentials: "omit",
			redirect: "error",
			referrerPolicy: "no-referrer",
			signal: controller.signal,
		});
		check(
			response.ok && response.body,
			"A dependency download failed. Check access and try again.",
		);
		const declared = response.headers.get("content-length");
		check(
			!declared || Number(declared) <= limit,
			"A dependency exceeds the browser export limit. Prepare it from the desktop app.",
		);
		const reader = response.body.getReader();
		const chunks: Uint8Array[] = [];
		let size = 0;
		try {
			while (true) {
				const next = await reader.read();
				if (next.done) break;
				size += next.value.length;
				check(
					size <= limit,
					"A dependency exceeds the browser export limit. Prepare it from the desktop app.",
				);
				chunks.push(next.value);
			}
		} finally {
			await reader.cancel().catch(() => {});
			reader.releaseLock();
		}
		const bytes = new Uint8Array(size);
		let offset = 0;
		for (const chunk of chunks) {
			bytes.set(chunk, offset);
			offset += chunk.length;
		}
		return bytes;
	} catch {
		throw new Error(
			"A dependency download failed or exceeded its limit. Check access, or prepare it from the desktop app.",
		);
	} finally {
		clearTimeout(timer);
		signal?.removeEventListener("abort", abort);
	}
}

const NO_DIGEST =
	"A model file has no content digest, and this browser fingerprints files of up to 64 MiB only. Deploy it from the desktop app.";
const TOO_LARGE =
	"A model asset exceeds the 64 MiB browser export limit. Use the desktop app.";

interface Packaging {
	files: Map<string, Blob>;
	add(path: string, bytes: Uint8Array): void;
	remaining(): number;
	modelStore: boolean;
	signal?: AbortSignal;
}

/**
 * A Bit file with a digest known up front: the device fetches it from its
 * sources, no byte is downloaded here. A file of up to 64 MiB without a public
 * source (a hub that signs its links) travels in the artifact instead.
 */
function storedAsset(item: IBit, root: IBit): PackagedBitAsset | undefined {
	const digest = bitContentDigest(item, root);
	if (!digest) return undefined;
	const { size, file_name: fileName } = item;
	check(
		typeof size === "number" &&
			Number.isSafeInteger(size) &&
			size > 0 &&
			size <= MAX_MODEL_ASSET,
		`Model file ${fileName} has no valid size. Refresh the model's metadata and prepare again.`,
	);
	const sources = bitSources(item, root);
	if (!sources.length && size <= MAX_FILE) return undefined;
	check(
		sources.length > 0,
		`Model file ${fileName} has no public download source the device or this browser could use. Deploy it from the desktop app.`,
	);
	return {
		bit_id: item.id,
		descriptor: { digest, size, file_name: fileName as string, sources },
	};
}

/** A Bit file whose bytes travel in the artifact, downloaded once per location. */
async function artifactOf(
	item: IBit,
	packaging: Packaging,
): Promise<ProjectArtifactFile> {
	check(
		Number.isSafeInteger(item.size) &&
			(item.size ?? 0) > 0 &&
			(item.size ?? 0) <= MAX_FILE,
		packaging.modelStore ? NO_DIGEST : TOO_LARGE,
	);
	const path = `bits/${item.hash}/${item.file_name}`;
	if (!packaging.files.has(path)) {
		check(
			item.download_link,
			"A model asset is unavailable for export. Download it in the desktop app first.",
		);
		const bytes = await download(
			item.download_link,
			Math.min(item.size as number, packaging.remaining()),
			packaging.signal,
		);
		check(
			bytes.length === item.size,
			"Model asset size differs from its metadata.",
		);
		packaging.add(path, bytes);
	}
	const blob = packaging.files.get(path);
	check(
		blob && blob.size === item.size,
		"Models disagree about an asset's size.",
	);
	return {
		path,
		size: blob.size,
		sha256: hash(new Uint8Array(await blob.arrayBuffer())),
	};
}

/** v1 when every file travels in the artifact, v2 once the device fetches some of them itself. */
function packagedMetadata(
	packaged: readonly IBit[],
	artifacts: ProjectArtifactFile[],
	stored: PackagedBitAsset[],
) {
	const bit = publicDependencyMetadata(packaged[0]);
	const dependencies = packaged.slice(1).map(publicDependencyMetadata);
	if (!stored.length) return { bit, dependencies, artifacts };
	return {
		version: 2,
		bit,
		dependencies,
		assets: stored,
		...(artifacts.length ? { artifacts } : {}),
	};
}

/**
 * `modelStore`: every target device acquires model files into its model
 * store (`model_store`). Files with a digest known up front then become
 * Bit metadata v2 assets and are not downloaded here; the rest travel in the
 * artifact as before.
 */
export async function prepareOnlineDependencies(
	requested: IApp,
	backend: IBackendState,
	profile: IProfile,
	signal?: AbortSignal,
	approved?: ApprovedOnlineMetadata,
	modelStore = false,
) {
	identifier(requested.id);
	const metadata =
		approved ??
		(await prepareOnlineMetadata(requested.id, backend, profile, signal));
	check(metadata.app.id === requested.id, "Approved project identity differs.");
	const app = metadata.app;
	check(
		app.bits.length <= 256 && Object.keys(app.packages ?? {}).length <= 64,
		"Too many project dependencies.",
	);
	const assets: ProjectArtifactAssets = { bit_pins: [], package_pins: [] };
	const files = new Map<string, Blob>();
	let total = 0;
	function add(path: string, bytes: Uint8Array) {
		total += bytes.length;
		check(
			total <= MAX_TOTAL,
			"Project dependencies exceed the 256 MiB browser export limit. Use the desktop app.",
		);
		files.set(path, new Blob([new Uint8Array(bytes)]));
	}
	const packaging: Packaging = {
		files,
		add,
		remaining: () => MAX_TOTAL - total,
		modelStore,
		...(signal ? { signal } : {}),
	};
	const encoder = new TextEncoder();
	add(
		`apps/${app.id}/online-source.json`,
		encoder.encode(
			JSON.stringify({ version: 1, project_id: app.id, source: "online" }),
		),
	);
	for (const reference of [...new Set(app.bits)].sort()) {
		signal?.throwIfAborted();
		const split = reference.lastIndexOf(":");
		const id = reference.slice(split + 1);
		identifier(id);
		const bit = await backend.bitState.getBit(
			id,
			split >= 0 ? reference.slice(0, split) : undefined,
		);
		check(
			bit.id === id && (split < 0 || bit.hub === reference.slice(0, split)),
			"A model identity differs from this project.",
		);
		const resolved = bit.dependencies.length
			? await backend.apiState.get<IBit[] | { bits: IBit[] }>(
					profile,
					`bit/${id}/dependencies`,
				)
			: [];
		const dependencies = Array.isArray(resolved) ? resolved : resolved.bits;
		check(
			Array.isArray(dependencies) && dependencies.length <= 2048,
			"Invalid model dependency inventory.",
		);
		const selected = new Map<string, IBit>();
		for (const item of [bit, ...dependencies]) {
			identifier(item.id);
			const previous = selected.get(item.id);
			check(
				!previous || previous.hub === item.hub,
				`Model ID "${item.id}" appears on multiple hubs in its resolved inventory.`,
			);
			// The dependency endpoint can repeat the root with less metadata.
			if (!previous) selected.set(item.id, item);
		}
		const packaged = [...selected.values()].map((item) => ({
			...item,
			// Device packages use plain IDs after each source hub has been verified.
			dependencies: item.dependencies.map((dependency) => {
				const split = dependency.lastIndexOf(":");
				const id = dependency.slice(split + 1);
				const resolved = selected.get(id);
				check(
					resolved &&
						(split < 0 || resolved.hub === dependency.slice(0, split)),
					`Model "${item.hub}:${item.id}" is missing dependency "${dependency}" from its resolved inventory.`,
				);
				return id;
			}),
		}));
		const artifacts = new Map<string, ProjectArtifactFile>();
		const stored: PackagedBitAsset[] = [];
		for (const item of packaged) {
			// Inline identities are derived by the native model implementation.
			// Reject them before downloading or stripping their source metadata.
			const parameters = JSON.stringify(item.parameters).toLowerCase();
			check(
				!parameters.includes('"projection"') && !parameters.includes('"mlx"'),
				"This model needs native asset resolution. Prepare its dependencies from the desktop app.",
			);
			if (!item.file_name) continue;
			identifier(item.hash);
			check(
				item.hash !== "metadata" &&
					item.hash !== "deps-cache" &&
					!item.file_name.includes("\\") &&
					item.file_name
						.split("/")
						.every((part) => part && part !== "." && part !== ".."),
				"Invalid model asset path.",
			);
			const asset = modelStore ? storedAsset(item, packaged[0]) : undefined;
			if (asset) {
				stored.push(asset);
				continue;
			}
			const artifact = await artifactOf(item, packaging);
			artifacts.set(artifact.path, artifact);
		}
		const metadata = encoder.encode(
			JSON.stringify(
				packagedMetadata(packaged, [...artifacts.values()], stored),
			),
		);
		check(
			metadata.length <= 16 * 1024 * 1024,
			"Model metadata exceeds its bound.",
		);
		add(`bits/metadata/${id}.json`, metadata);
		assets.bit_pins.push({ bit_id: id, metadata_sha256: hash(metadata) });
	}
	for (const [id, version] of Object.entries(app.packages ?? {}).sort()) {
		signal?.throwIfAborted();
		identifier(id);
		check(typeof version === "string", "Package version is missing.");
		identifier(version);
		const selected = await backend.apiState.post<{
			package_id: string;
			version: string;
			manifest: { id: string; version: string; wasm_hash?: string };
			download_url?: string;
			wasm_base64?: string;
		}>(profile, "registry/download", { package_id: id, version });
		check(
			selected.package_id === id &&
				selected.version === version &&
				selected.manifest.id === id &&
				selected.manifest.version === version,
			"A package differs from this project's pinned version.",
		);
		let wasm: Uint8Array;
		if (selected.download_url)
			wasm = await download(
				selected.download_url,
				Math.min(MAX_FILE, MAX_TOTAL - total),
				signal,
			);
		else {
			check(
				selected.wasm_base64 &&
					selected.wasm_base64.length <= Math.ceil(MAX_FILE / 3) * 4,
				"Package bytes are missing or exceed the browser export limit.",
			);
			try {
				wasm = Uint8Array.from(atob(selected.wasm_base64), (value) =>
					value.charCodeAt(0),
				);
			} catch {
				throw new Error("Invalid package bytes.");
			}
		}
		check(
			wasm.length <= MAX_FILE &&
				wasm[0] === 0 &&
				wasm[1] === 97 &&
				wasm[2] === 115 &&
				wasm[3] === 109,
			"Packages must contain portable WASM bytes.",
		);
		if (selected.manifest.wasm_hash)
			check(
				selected.manifest.wasm_hash ===
					Array.from(blake3(wasm), (byte) =>
						byte.toString(16).padStart(2, "0"),
					).join(""),
				"Package bytes differ from their manifest digest.",
			);
		const manifest = encoder.encode(
			JSON.stringify(publicDependencyMetadata(selected.manifest)),
		);
		check(manifest.length <= 1024 * 1024, "Package manifest exceeds 1 MiB.");
		add(`packages/${id}/${version}/module.wasm`, wasm);
		add(`packages/${id}/${version}/manifest.json`, manifest);
		assets.package_pins.push({
			package_id: id,
			version,
			wasm_sha256: hash(wasm),
			manifest_sha256: hash(manifest),
		});
	}
	const inputs: ArtifactInput[] = [...files].map(([path, file]) => ({
		path,
		file,
	}));
	inputs.push(metadata.file);
	return {
		assets,
		online_metadata_sha256: metadata.sha256,
		artifact: await prepareProjectArtifact(
			app.id,
			inputs,
			signal,
			assets,
			"online",
		),
	};
}
