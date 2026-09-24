import { sha256 } from "@noble/hashes/sha2";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import type { ArtifactInput } from "./artifacts";

export type ApprovedOnlineMetadata = {
	app: IApp;
	file: ArtifactInput;
	sha256: string;
};

/** Hash the exact locally held payload. Only the authenticated deployment approves it. */
export async function prepareOnlineMetadata(
	project: string,
	backend: IBackendState,
	profile: IProfile,
	signal?: AbortSignal,
): Promise<ApprovedOnlineMetadata> {
	signal?.throwIfAborted();
	const bundle = await backend.apiState.get<{
		version: number;
		project_id: string;
		documents: Record<string, unknown>;
	}>(profile, `apps/${project}/device-metadata`);
	signal?.throwIfAborted();
	const app = bundle?.documents?.app as IApp | undefined;
	if (
		bundle?.version !== 1 ||
		bundle.project_id !== project ||
		!app ||
		app.id !== project ||
		app.visibility === "Offline" ||
		Object.keys(bundle.documents).length > 1024 ||
		Object.keys(bundle).some(
			(key) => !["version", "project_id", "documents"].includes(key),
		)
	)
		throw new Error(
			"The server returned an invalid executable metadata snapshot. Update the server and prepare this project again.",
		);
	const bytes = new TextEncoder().encode(JSON.stringify(bundle));
	if (bytes.length > 32 * 1024 * 1024)
		throw new Error("Executable metadata exceeds the 32 MiB deployment limit.");
	return {
		app,
		file: {
			path: `apps/${project}/online-metadata.json`,
			file: new Blob([bytes]),
		},
		sha256: Array.from(sha256(bytes), (byte) =>
			byte.toString(16).padStart(2, "0"),
		).join(""),
	};
}
