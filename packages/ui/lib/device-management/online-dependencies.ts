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
	hashBlob,
	prepareProjectArtifact,
} from "./artifacts";

import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "./online-metadata";

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

function metadataIdentity(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(metadataIdentity).join(",")}]`;
	if (value && typeof value === "object")
		return `{${Object.entries(value)
			.filter(([, entry]) => entry !== undefined)
			.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
			.map(
				([key, entry]) => `${JSON.stringify(key)}:${metadataIdentity(entry)}`,
			)
			.join(",")}}`;
	return JSON.stringify(value) ?? "null";
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

async function download(url: string, signal?: AbortSignal): Promise<Blob> {
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
			declared === null ||
				(/^\d+$/.test(declared) && Number.isSafeInteger(Number(declared))),
			"A dependency has an invalid content length.",
		);
		const file = await response.blob();
		check(
			Number.isSafeInteger(file.size) && file.size >= 0,
			"A dependency has an invalid size.",
		);
		return file;
	} catch {
		throw new Error(
			"A dependency download failed. Check access and try again.",
		);
	} finally {
		clearTimeout(timer);
		signal?.removeEventListener("abort", abort);
	}
}

interface Packaging {
	files: Map<string, Blob>;
	add(path: string, file: Blob | Uint8Array): void;
	signal?: AbortSignal;
}

/**
 * A Bit file with a digest known up front: the device fetches it from its
 * sources, no byte is downloaded here. A file without a public source
 * (a hub that signs its links) travels in the artifact instead.
 */
function storedAsset(item: IBit, root: IBit): PackagedBitAsset | undefined {
	const digest = bitContentDigest(item, root);
	if (!digest) return undefined;
	const { size, file_name: fileName } = item;
	check(
		typeof size === "number" && Number.isSafeInteger(size) && size > 0,
		`Model file ${fileName} has no valid size. Refresh the model's metadata and prepare again.`,
	);
	const sources = bitSources(item, root);
	if (!sources.length) return undefined;
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
		Number.isSafeInteger(item.size) && (item.size ?? 0) > 0,
		"A model asset has an invalid size. Refresh its metadata and prepare again.",
	);
	const path = `bits/${item.hash}/${item.file_name}`;
	if (!packaging.files.has(path)) {
		check(
			item.download_link,
			"A model asset is unavailable for export. Download it in the desktop app first.",
		);
		const file = await download(item.download_link, packaging.signal);
		check(
			file.size === item.size,
			"Model asset size differs from its metadata.",
		);
		packaging.add(path, file);
	}
	const blob = packaging.files.get(path);
	check(
		blob && blob.size === item.size,
		"Models disagree about an asset's size.",
	);
	return {
		path,
		size: blob.size,
		sha256: await hashBlob(blob, packaging.signal),
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
	function add(path: string, file: Blob | Uint8Array) {
		files.set(
			path,
			file instanceof Blob ? file : new Blob([new Uint8Array(file)]),
		);
	}
	const packaging: Packaging = {
		files,
		add,
		...(signal ? { signal } : {}),
	};
	const encoder = new TextEncoder();
	add(
		`apps/${app.id}/online-source.json`,
		encoder.encode(
			JSON.stringify({ version: 1, project_id: app.id, source: "online" }),
		),
	);
	const resolvedBits = new Map<string, string>();
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
		const identity = metadataIdentity(publicDependencyMetadata(bit));
		const previous = resolvedBits.get(id);
		check(
			previous === undefined || previous === identity,
			`Selected references disagree about model "${id}".`,
		);
		if (previous !== undefined) continue;
		resolvedBits.set(id, identity);
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
		let wasm: Blob;
		if (selected.download_url)
			wasm = await download(selected.download_url, signal);
		else {
			check(selected.wasm_base64, "Package bytes are missing.");
			try {
				wasm = new Blob([
					Uint8Array.from(atob(selected.wasm_base64), (value) =>
						value.charCodeAt(0),
					),
				]);
			} catch {
				throw new Error("Invalid package bytes.");
			}
		}
		const magic = new Uint8Array(await wasm.slice(0, 4).arrayBuffer());
		check(
			magic[0] === 0 && magic[1] === 97 && magic[2] === 115 && magic[3] === 109,
			"Packages must contain portable WASM bytes.",
		);
		if (selected.manifest.wasm_hash)
			check(
				selected.manifest.wasm_hash ===
					(await hashBlob(wasm, signal, blake3.create)),
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
			wasm_sha256: await hashBlob(wasm, signal),
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
