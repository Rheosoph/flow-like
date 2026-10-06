import { publicSource } from "../bit/bit-sources";
import type { AgentFeatures } from "./model/types";
import {
	MODEL_RUNTIME_MANIFEST_MAX_BYTES,
	type ModelsRequest,
	readModelsOverview,
} from "./models";
import type { ManagementCall } from "./telemetry";

type InstallRuntime = Extract<ModelsRequest, { kind: "install_runtime" }>;

/** The agent verifies the signature and sequence; the client only carries bounded bytes. */
async function downloadManifest(url: string, fetcher: typeof fetch) {
	if (!publicSource(url)) return undefined;
	const response = await fetcher(url, {
		credentials: "omit",
		redirect: "error",
		cache: "no-store",
		signal: AbortSignal.timeout(5_000),
	});
	if (!response.ok || !response.body) {
		await response.body?.cancel();
		return undefined;
	}
	const length = response.headers.get("content-length");
	if (length && Number(length) > MODEL_RUNTIME_MANIFEST_MAX_BYTES) {
		await response.body.cancel();
		return undefined;
	}
	const reader = response.body.getReader();
	const decoder = new TextDecoder("utf-8", { fatal: true });
	let size = 0;
	let compact = "";
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			size += value.byteLength;
			if (size > MODEL_RUNTIME_MANIFEST_MAX_BYTES) return undefined;
			compact += decoder.decode(value, { stream: true });
		}
		compact = (compact + decoder.decode()).trim();
		return /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/u.test(compact)
			? compact
			: undefined;
	} finally {
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}

/** A computer with CDN access can supply the manifest before the device creates its push job. */
export async function prepareRuntimeInstall(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	request: InstallRuntime,
	fetcher: typeof fetch = fetch,
): Promise<InstallRuntime> {
	if (features?.model_runtime_manifest !== 1) return request;
	const overview = await readModelsOverview(call, features);
	if (overview.kind !== "ok" || !overview.data.runtime_manifest_url)
		return request;
	try {
		const compact = await downloadManifest(
			overview.data.runtime_manifest_url,
			fetcher,
		);
		return compact ? { ...request, manifest_jws: compact } : request;
	} catch {
		// A browser's CDN failure must still let an online device fetch its own manifest.
		return request;
	}
}
