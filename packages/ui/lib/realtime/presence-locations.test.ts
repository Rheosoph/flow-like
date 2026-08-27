import { describe, expect, test } from "bun:test";
import type { PeerPresence } from "./peer-presence";
import {
	layerIdOfPath,
	mergePresenceMarks,
	nodeWatchers,
	presenceByFile,
	presenceByLayer,
	presenceMarksKey,
} from "./presence-locations";

const NODE_A = "nodeanchor0000000001";
const NODE_B = "nodeanchor0000000002";
const MODULE = "layeranchor000000001";
const LAYER = "layeranchor000000002";
const ME = "me";

function session(
	clientId: number,
	overrides: Partial<PeerPresence> = {},
): PeerPresence {
	return {
		clientId,
		layerPath: "root",
		selection: { nodes: [] },
		claimedAnchorIds: [],
		scopeNodeIds: [],
		executingNodeIds: [],
		...overrides,
	};
}

describe("presenceByFile", () => {
	test("groups sessions by code file, counting a user's windows once per user", () => {
		const peers = [
			session(1, { sub: "anna", codeFile: "main" }),
			session(2, { sub: "anna", codeFile: "main" }),
			session(3, { sub: "bob", codeFile: MODULE }),
			session(4, { sub: "carl" }),
			session(5, { codeFile: "main" }),
		];
		const byFile = presenceByFile(peers, ME);
		expect([...byFile.keys()].sort()).toEqual(["main", MODULE].sort());
		expect(byFile.get("main")).toEqual([
			{ sub: "anna", self: false, sessions: 2 },
		]);
		expect(byFile.get(MODULE)).toEqual([
			{ sub: "bob", self: false, sessions: 1 },
		]);
	});

	test("the local user's other windows are marked self and sorted last", () => {
		const peers = [
			session(1, { sub: ME, codeFile: "main" }),
			session(2, { sub: "zed", codeFile: "main" }),
			session(3, { sub: "anna", codeFile: "main" }),
		];
		expect(presenceByFile(peers, ME).get("main")).toEqual([
			{ sub: "anna", self: false, sessions: 1 },
			{ sub: "zed", self: false, sessions: 1 },
			{ sub: ME, self: true, sessions: 1 },
		]);
	});
});

describe("presenceByLayer", () => {
	test("keys by the innermost layer id and never by the root", () => {
		const peers = [
			session(1, { sub: "anna", layerPath: `root/${MODULE}/${LAYER}` }),
			session(2, { sub: "bob", layerPath: MODULE }),
			session(3, { sub: "carl", layerPath: "root" }),
			session(4, { sub: "dora", layerPath: "" }),
		];
		const byLayer = presenceByLayer(peers, ME);
		expect(byLayer.get(LAYER)).toEqual([
			{ sub: "anna", self: false, sessions: 1 },
		]);
		expect(byLayer.get(MODULE)).toEqual([
			{ sub: "bob", self: false, sessions: 1 },
		]);
		expect(byLayer.has("root")).toBe(false);
		expect(byLayer.has("")).toBe(false);
		expect(byLayer.size).toBe(2);
	});

	test("layerIdOfPath reads the last segment and treats root as no layer", () => {
		expect(layerIdOfPath("root")).toBeUndefined();
		expect(layerIdOfPath(undefined)).toBeUndefined();
		expect(layerIdOfPath(`root/${MODULE}`)).toBe(MODULE);
		expect(layerIdOfPath(MODULE)).toBe(MODULE);
	});
});

describe("nodeWatchers", () => {
	test("splits canvas selection from code editing, deduped per user", () => {
		const peers = [
			session(1, { sub: "anna", selection: { nodes: [NODE_A, NODE_B] } }),
			session(2, { sub: "anna", selection: { nodes: [NODE_A] } }),
			session(3, {
				sub: "bob",
				editor: { anchorId: NODE_A, anchorKind: "node", selectedAnchorIds: [] },
			}),
			session(4, {
				sub: "carl",
				editor: {
					anchorId: NODE_B,
					anchorKind: "node",
					selectedAnchorIds: [NODE_B, NODE_A],
				},
			}),
			session(5, { sub: "dora", claimedAnchorIds: [NODE_A] }),
			session(6, { sub: "eve", selection: { nodes: [NODE_B] } }),
			session(7, { selection: { nodes: [NODE_A] } }),
		];
		expect(nodeWatchers(peers, NODE_A, ME)).toEqual({
			selected: [{ sub: "anna", self: false, sessions: 2 }],
			editing: [
				{ sub: "bob", self: false, sessions: 1 },
				{ sub: "carl", self: false, sessions: 1 },
				{ sub: "dora", self: false, sessions: 1 },
			],
		});
	});

	test("a user can be in both lists, and an unwatched node yields empty lists", () => {
		const peers = [
			session(1, {
				sub: ME,
				selection: { nodes: [NODE_A] },
				claimedAnchorIds: [NODE_A],
			}),
		];
		expect(nodeWatchers(peers, NODE_A, ME)).toEqual({
			selected: [{ sub: ME, self: true, sessions: 1 }],
			editing: [{ sub: ME, self: true, sessions: 1 }],
		});
		expect(nodeWatchers(peers, NODE_B, ME)).toEqual({
			selected: [],
			editing: [],
		});
	});
});

describe("mergePresenceMarks", () => {
	test("dedupes a user across file and layer marks without double counting", () => {
		const merged = mergePresenceMarks(
			[
				{ sub: "anna", self: false, sessions: 2 },
				{ sub: ME, self: true, sessions: 1 },
			],
			undefined,
			[
				{ sub: "anna", self: false, sessions: 1 },
				{ sub: "bob", self: false, sessions: 1 },
			],
		);
		expect(merged).toEqual([
			{ sub: "anna", self: false, sessions: 2 },
			{ sub: "bob", self: false, sessions: 1 },
			{ sub: ME, self: true, sessions: 1 },
		]);
		expect(presenceMarksKey(merged)).toBe("anna:0:2|bob:0:1|me:1:1");
		expect(mergePresenceMarks()).toEqual([]);
	});
});
