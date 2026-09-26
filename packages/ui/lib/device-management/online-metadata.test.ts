import { expect, test } from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import { prepareOnlineMetadata } from "./online-metadata";

const profile = {} as IProfile;
function backend(payload: unknown) {
	return {
		apiState: {
			get: async (_: unknown, route: string) => {
				expect(route).toBe("apps/project/device-metadata");
				return payload;
			},
		},
	} as unknown as IBackendState;
}
test("controller approval binds exact local bytes including same-version executable changes", async () => {
	const bundle = {
		version: 1,
		project_id: "project",
		documents: {
			app: { id: "project", visibility: "Private" },
			"boards/board/versions/1/0/0": {
				id: "board",
				version: [1, 0, 0],
				nodes: {},
			},
		},
	};
	const first = await prepareOnlineMetadata(
		"project",
		backend(bundle),
		profile,
	);
	const bytes = new Uint8Array(await first.file.file.arrayBuffer());
	expect(first.file.path).toBe("apps/project/online-metadata.json");
	expect(first.sha256).toBe(
		Array.from(sha256(bytes), (byte) =>
			byte.toString(16).padStart(2, "0"),
		).join(""),
	);
	bundle.documents["boards/board/versions/1/0/0"].nodes = { injected: {} };
	const changed = await prepareOnlineMetadata(
		"project",
		backend(bundle),
		profile,
	);
	expect(changed.sha256).not.toBe(first.sha256);
	expect(
		JSON.parse(new TextDecoder().decode(bytes)).documents[
			"boards/board/versions/1/0/0"
		].nodes,
	).toEqual({});
});
test("foreign project metadata and server-provided approval digests are rejected", async () => {
	const valid = {
		version: 1,
		project_id: "project",
		documents: { app: { id: "project", visibility: "Private" } },
	};
	for (const payload of [
		{ ...valid, project_id: "other" },
		{ ...valid, sha256: "a".repeat(64) },
		{ ...valid, documents: { app: { id: "other" } } },
	]) {
		await expect(
			prepareOnlineMetadata("project", backend(payload), profile),
		).rejects.toThrow("invalid executable metadata");
	}
});
