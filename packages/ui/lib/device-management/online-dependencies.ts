import { blake3 } from "@noble/hashes/blake3";
import { sha256 } from "@noble/hashes/sha2";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import type { IBit } from "../schema/bit/bit";
import {
	type ArtifactInput,
	type ProjectArtifactAssets,
	prepareProjectArtifact,
} from "./artifacts";

import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "./online-metadata";

const MAX_FILE = 64 * 1024 * 1024;
const MAX_TOTAL = 256 * 1024 * 1024;
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

export async function prepareOnlineDependencies(
	app: IApp,
	backend: IBackendState,
	profile: IProfile,
	signal?: AbortSignal,
	approved?: ApprovedOnlineMetadata,
) {
	identifier(app.id);
	const metadata =
		approved ?? (await prepareOnlineMetadata(app.id, backend, profile, signal));
	check(metadata.app.id === app.id, "Approved project identity differs.");
	app = metadata.app;
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
		check(bit.id === id, "A model identity differs from this project.");
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
		const selected = new Map(
			[bit, ...dependencies].map((item) => [item.id, item]),
		);
		const artifacts = new Map<
			string,
			{ path: string; size: number; sha256: string }
		>();
		for (const item of selected.values()) {
			identifier(item.id);
			// Inline identities are derived by the native model implementation.
			// Reject them before downloading or stripping their source metadata.
			const parameters = JSON.stringify(item.parameters).toLowerCase();
			check(
				!parameters.includes('"projection"') && !parameters.includes('"mlx"'),
				"This model needs native asset resolution. Prepare its dependencies from the desktop app.",
			);
			check(
				item.dependencies.every((dependency) => selected.has(dependency)),
				"A model dependency is missing from its resolved inventory.",
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
			check(
				Number.isSafeInteger(item.size) &&
					(item.size ?? 0) > 0 &&
					(item.size ?? 0) <= MAX_FILE,
				"A model asset exceeds the 64 MiB browser export limit. Use the desktop app.",
			);
			const path = `bits/${item.hash}/${item.file_name}`;
			if (!files.has(path)) {
				check(
					item.download_link,
					"A model asset is unavailable for export. Download it in the desktop app first.",
				);
				const bytes = await download(
					item.download_link,
					Math.min(item.size as number, MAX_TOTAL - total),
					signal,
				);
				check(
					bytes.length === item.size,
					"Model asset size differs from its metadata.",
				);
				add(path, bytes);
			}
			const blob = files.get(path);
			check(
				blob && blob.size === item.size,
				"Models disagree about an asset's size.",
			);
			artifacts.set(path, {
				path,
				size: blob.size,
				sha256: hash(new Uint8Array(await blob.arrayBuffer())),
			});
		}
		const metadata = encoder.encode(
			JSON.stringify({
				bit: publicDependencyMetadata(bit),
				dependencies: [...selected.values()]
					.filter((item) => item.id !== id)
					.map(publicDependencyMetadata),
				artifacts: [...artifacts.values()],
			}),
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
