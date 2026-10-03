import { expect, test } from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import {
	deviceMetadataPath,
	exportTypes,
	prepareOnlineMetadata,
} from "./online-metadata";

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
test("approved metadata carries the exact event pins and variables it will deploy", async () => {
	const approved = await prepareOnlineMetadata(
		"project",
		backend({
			version: 1,
			project_id: "project",
			documents: {
				app: { id: "project", visibility: "Private" },
				"events/api/versions/1/0/0": {
					id: "api",
					name: "API",
					event_type: "http",
					event_version: [1, 0, 0],
					board_id: "board",
					board_version: [2, 0, 0],
					active: true,
					config: Array.from(
						new TextEncoder().encode('{"path":"/api","method":"POST"}'),
					),
				},
				"boards/board/versions/2/0/0": {
					id: "board",
					version: [2, 0, 0],
					layers: {},
					variables: {
						token: {
							id: "token",
							name: "Token",
							data_type: "String",
							value_type: "Normal",
							secret: true,
							exposed: true,
						},
					},
				},
			},
		}),
		profile,
	);
	expect(
		approved.catalog.events.map((event) => [
			event.id,
			event.event_version,
			event.board_version,
			event.eligible,
		]),
	).toEqual([["api", [1, 0, 0], [2, 0, 0], true]]);
	expect(approved.catalog.variables.api).toEqual([
		{
			id: "token",
			name: "Token",
			data_type: "String",
			value_type: "Normal",
			secret: true,
		},
	]);
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

test("the export is asked for exactly the chosen Latest events, in one order", async () => {
	expect(deviceMetadataPath("project")).toBe("apps/project/device-metadata");
	expect(deviceMetadataPath("project", [])).toBe(
		"apps/project/device-metadata",
	);
	expect(deviceMetadataPath("project", ["evt_b", "evt_a", "evt_b"])).toBe(
		"apps/project/device-metadata?latest=evt_a,evt_b",
	);
	expect(deviceMetadataPath("project", ["evt.a-1", "evt b"])).toBe(
		"apps/project/device-metadata?latest=evt%20b,evt.a-1",
	);
	const routes: string[] = [];
	const hub = {
		apiState: {
			get: async (_: unknown, route: string) => {
				routes.push(route);
				return {
					version: 1,
					project_id: "project",
					documents: { app: { id: "project", visibility: "Private" } },
				};
			},
		},
	} as unknown as IBackendState;
	await prepareOnlineMetadata("project", hub, profile, undefined, [
		"evt_b",
		"evt_a",
	]);
	await prepareOnlineMetadata("project", hub, profile);
	expect(routes).toEqual([
		"apps/project/device-metadata?latest=evt_a,evt_b",
		"apps/project/device-metadata",
	]);
	const names = Array.from({ length: 65 }, (_, index) => `evt_${index}`);
	expect(deviceMetadataPath("project", names.slice(0, 64))).toContain(
		"?latest=evt_0,evt_1,",
	);
	expect(() => deviceMetadataPath("project", names)).toThrow(
		"One deploy can resolve 64 events that follow Latest, and this one names 65.",
	);
});

/* run-more-2-design §1.9 item 3: the hub exports these five types only when the request names them. */
const ROUND_TWO_HUB = [
	"http",
	"simple_chat",
	"rest",
	"mcp",
	"daemon",
	"cron",
	"api",
	"quick_action",
	"generic_form",
	"telegram",
	"discord",
];

test("the export names the new types of the deploy's events, sorted, once, and only to a hub that knows them", () => {
	const page = { event_type: "generic_form", default_page_id: "page" };
	// A deploy of Pages and chats names none: the bundle is round one's.
	expect(
		exportTypes(
			[{ event_type: "http" }, { event_type: "simple_chat" }, page],
			ROUND_TWO_HUB,
		),
	).toEqual([]);
	expect(
		exportTypes(
			[
				{ event_type: "telegram" },
				{ event_type: "generic_form" },
				{ event_type: "api" },
				{ event_type: "generic_form" },
				{ event_type: "cron" },
				{ event_type: "discord", default_page_id: null },
			],
			ROUND_TWO_HUB,
		),
	).toEqual(["api", "discord", "generic_form", "telegram"]);
	// A hub without `event_types` knows no `types`: never sent to it.
	expect(exportTypes([{ event_type: "generic_form" }], undefined)).toEqual([]);
	expect(deviceMetadataPath("project", [], ["generic_form"])).toBe(
		"apps/project/device-metadata?types=generic_form",
	);
	expect(
		deviceMetadataPath(
			"project",
			["evt_b", "evt_a"],
			["telegram", "api", "telegram"],
		),
	).toBe("apps/project/device-metadata?latest=evt_a,evt_b&types=api,telegram");
	expect(deviceMetadataPath("project", [], [])).toBe(
		"apps/project/device-metadata",
	);
	expect(() => deviceMetadataPath("project", [], ["cron"])).toThrow(
		'not "cron"',
	);
});

test("a bundle is asked for, and kept, by the types it names", async () => {
	const routes: string[] = [];
	const hub = {
		apiState: {
			get: async (_: unknown, route: string) => {
				routes.push(route);
				return {
					version: 1,
					project_id: "project",
					documents: { app: { id: "project", visibility: "Private" } },
				};
			},
		},
	} as unknown as IBackendState;
	await prepareOnlineMetadata(
		"project",
		hub,
		profile,
		undefined,
		[],
		["quick_action", "generic_form"],
	);
	await prepareOnlineMetadata("project", hub, profile, undefined, ["evt_a"]);
	expect(routes).toEqual([
		"apps/project/device-metadata?types=generic_form,quick_action",
		"apps/project/device-metadata?latest=evt_a",
	]);
	// The path is the cache key: another set of types is another bundle.
	expect(deviceMetadataPath("project", [], ["api"])).not.toBe(
		deviceMetadataPath("project", [], ["api", "discord"]),
	);
});
