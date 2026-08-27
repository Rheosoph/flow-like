import { describe, expect, test } from "bun:test";
import {
	AWAY_AFTER_MS,
	type PeerPresence,
} from "../../lib/realtime/peer-presence";
import { ICommandType } from "../../lib/schema/flow/board/commands/generic-command";
import {
	PRESENCE_AGO_CAP_MINUTES,
	PRESENCE_EVENT_TTL_MS,
	PRESENCE_IDLE_AFTER_MS,
	agoMinutes,
	describeActivity,
	mergeCollaborators,
	presenceEventRemainingMs,
	presenceHighlightIds,
	presenceStats,
	sortCollaborators,
} from "./flow-presence-bar-model";

let nextClientId = 1;
const session = (
	sub: string | undefined,
	overrides: Partial<PeerPresence> = {},
): PeerPresence => ({
	clientId: nextClientId++,
	sub,
	layerPath: "root",
	selection: { nodes: [] },
	claimedAnchorIds: [],
	scopeNodeIds: [],
	executingNodeIds: [],
	...overrides,
});

describe("mergeCollaborators", () => {
	test("folds several sessions of one user into one row and unions their ids", () => {
		const rows = mergeCollaborators(
			[
				session("alice", {
					selection: { nodes: ["n2", "n1"] },
					claimedAnchorIds: ["c1"],
					scopeNodeIds: ["s1"],
				}),
				session("alice", {
					selection: { nodes: ["n3", "n1"] },
					claimedAnchorIds: ["c2"],
					executingNodeIds: ["x1"],
				}),
				session(undefined),
			],
			"me",
		);

		expect(rows).toHaveLength(1);
		expect(rows[0]).toMatchObject({
			sub: "alice",
			self: false,
			sessions: 2,
			selectionNodeIds: ["n1", "n2", "n3"],
			claimedAnchorIds: ["c1", "c2"],
			scopeNodeIds: ["s1"],
			executingNodeIds: ["x1"],
		});
		expect(rows[0]).not.toHaveProperty("primaryTs");
	});

	test("keeps the location, editor and active node of the most recent session", () => {
		const [alice] = mergeCollaborators(
			[
				session("alice", {
					layerPath: "a",
					activeNodeId: "old",
					activeNodeTs: 10,
					codeFile: "main",
					editor: {
						anchorId: "e-old",
						anchorKind: "node",
						selectedAnchorIds: [],
					},
				}),
				session("alice", {
					layerPath: "a/b",
					activeNodeId: "new",
					activeNodeTs: 20,
					codeFile: "mod",
					editor: {
						anchorId: "e-new",
						anchorKind: "node",
						selectedAnchorIds: [],
					},
				}),
			],
			"me",
		);

		expect(alice.layerPath).toBe("a/b");
		expect(alice.activeNodeId).toBe("new");
		expect(alice.activeNodeTs).toBe(20);
		expect(alice.editor?.anchorId).toBe("e-new");
		expect(alice.codeFile).toBe("mod");
	});

	test("flags the local user's other windows as self and normalises an empty layer path", () => {
		const rows = mergeCollaborators(
			[session("me", { layerPath: "" }), session("bob")],
			"me",
		);
		expect(rows.find((r) => r.sub === "me")).toMatchObject({
			self: true,
			layerPath: "root",
		});
		expect(rows.find((r) => r.sub === "bob")?.self).toBe(false);
	});

	test("keeps the most recent last edit and last run across sessions", () => {
		const [alice] = mergeCollaborators(
			[
				session("alice", {
					lastEdit: { kinds: [ICommandType.MoveNode], count: 3, ts: 50 },
					lastRun: {
						runId: "run-aaaaaaaaaa",
						status: "ok",
						executed: 4,
						ts: 90,
					},
				}),
				session("alice", {
					lastEdit: { kinds: [ICommandType.AddNode], count: 1, ts: 70 },
				}),
				session("alice", {
					lastRun: {
						runId: "run-bbbbbbbbbb",
						status: "error",
						executed: 2,
						ts: 40,
					},
				}),
			],
			"me",
		);

		expect(alice.lastEdit).toEqual({
			kinds: [ICommandType.AddNode],
			count: 1,
			ts: 70,
		});
		expect(alice.lastRun).toMatchObject({ runId: "run-aaaaaaaaaa", ts: 90 });

		const [bob] = mergeCollaborators([session("bob")], "me");
		expect(bob.lastEdit).toBeUndefined();
		expect(bob.lastRun).toBeUndefined();
	});
});

describe("sortCollaborators", () => {
	test("orders same-layer peers first, then by name, with self sessions last", () => {
		const rows = mergeCollaborators(
			[
				session("me", { layerPath: "here" }),
				session("zoe", { layerPath: "here" }),
				session("bob", { layerPath: "elsewhere" }),
				session("amy", { layerPath: "here" }),
			],
			"me",
		);
		const names: Record<string, string> = {
			me: "Aaron",
			zoe: "Zoe",
			bob: "Bob",
			amy: "Amy",
		};
		const sorted = sortCollaborators(rows, "here", (sub) => names[sub]);
		expect(sorted.map((r) => r.sub)).toEqual(["amy", "zoe", "bob", "me"]);
	});
});

describe("describeActivity", () => {
	const ctx = {
		currentLayerPath: "root",
		layerNames: new Map([["layer-1", "Checkout"]]),
		fileLabels: new Map([
			["main", "main.flow"],
			["mod-1", "checkout/payments.flow"],
		]),
		nodeName: (id: string) => (id === "n1" ? "HTTP Request" : undefined),
		now: 1_000_000,
	};

	test("resolves layer, file and node names and flags the current layer as here", () => {
		const [collab] = mergeCollaborators(
			[
				session("alice", {
					layerPath: "root/layer-1",
					codeFile: "mod-1",
					selection: { nodes: ["n9", "n1"] },
					executingNodeIds: ["n1"],
					editor: { anchorId: "n1", anchorKind: "node", selectedAnchorIds: [] },
				}),
			],
			"me",
		);
		const activity = describeActivity(collab, ctx);
		expect(activity).toMatchObject({
			sameLayer: false,
			layerPath: "root/layer-1",
			layerLabel: "Checkout",
			codeFileLabel: "checkout/payments.flow",
			editing: { anchorId: "n1", kind: "node", label: "HTTP Request" },
			selectedCount: 2,
			firstSelectedNodeId: "n1",
			running: true,
			idleMinutes: undefined,
		});

		const [here] = mergeCollaborators([session("bob")], "me");
		const hereActivity = describeActivity(here, ctx);
		expect(hereActivity.sameLayer).toBe(true);
		expect(hereActivity.layerLabel).toBeUndefined();
		expect(hereActivity.running).toBe(false);
	});

	test("skips variable anchors, names layer anchors and falls back to the raw id", () => {
		const build = (kind: "node" | "layer" | "variable", anchorId: string) =>
			mergeCollaborators(
				[
					session("alice", {
						editor: { anchorId, anchorKind: kind, selectedAnchorIds: [] },
					}),
				],
				"me",
			)[0];
		expect(
			describeActivity(build("variable", "v1"), ctx).editing,
		).toBeUndefined();
		expect(
			describeActivity(build("layer", "layer-1"), ctx).editing?.label,
		).toBe("Checkout");
		expect(
			describeActivity(build("node", "unknown-node-id-xyz"), ctx).editing
				?.label,
		).toBe("unknown-no");
	});

	test("reports idle minutes only past the threshold, on the local clock", () => {
		const [collab] = mergeCollaborators([session("alice")], "me");
		const at = (ago: number) =>
			describeActivity(collab, { ...ctx, lastActiveAt: ctx.now - ago })
				.idleMinutes;
		expect(at(PRESENCE_IDLE_AFTER_MS - 1)).toBeUndefined();
		expect(at(PRESENCE_IDLE_AFTER_MS)).toBe(1);
		expect(at(4 * 60_000 + 30_000)).toBe(4);
		expect(describeActivity(collab, ctx).idleMinutes).toBeUndefined();
	});

	test("passes the live predicates through, defaulting to false", () => {
		const [collab] = mergeCollaborators([session("alice")], "me");
		expect(describeActivity(collab, ctx)).toMatchObject({
			typingInEditor: false,
			typingInChat: false,
			away: false,
		});
		expect(
			describeActivity(collab, {
				...ctx,
				typingInEditor: true,
				typingInChat: true,
			}),
		).toMatchObject({ typingInEditor: true, typingInChat: true, away: false });
	});

	test("an away user always has idle minutes, floored at the away threshold", () => {
		const [collab] = mergeCollaborators([session("alice")], "me");
		expect(describeActivity(collab, { ...ctx, away: true })).toMatchObject({
			away: true,
			idleMinutes: Math.floor(AWAY_AFTER_MS / 60_000),
		});
		expect(
			describeActivity(collab, {
				...ctx,
				away: true,
				lastActiveAt: ctx.now - 12 * 60_000,
			}).idleMinutes,
		).toBe(12);
	});

	test("groups the last edit into verbs with a clamped ago label", () => {
		const build = (ts: number) =>
			mergeCollaborators(
				[
					session("alice", {
						lastEdit: {
							kinds: [
								ICommandType.MoveNode,
								ICommandType.MoveToLayer,
								ICommandType.ConnectPin,
							],
							count: 5,
							ts,
						},
					}),
				],
				"me",
			)[0];
		expect(
			describeActivity(build(ctx.now - 2 * 60_000 - 5000), ctx).lastEdit,
		).toEqual({ verbs: ["moved", "connected"], count: 5, agoMinutes: 2 });
		expect(
			describeActivity(build(ctx.now + 90_000), ctx).lastEdit?.agoMinutes,
		).toBe(0);
		expect(
			describeActivity(build(ctx.now - 3 * 24 * 60 * 60_000), ctx).lastEdit
				?.agoMinutes,
		).toBe(PRESENCE_AGO_CAP_MINUTES);
		expect(
			describeActivity(mergeCollaborators([session("bob")], "me")[0], ctx)
				.lastEdit,
		).toBeUndefined();
	});

	test("summarises the last run with its status, node count and age", () => {
		const [collab] = mergeCollaborators(
			[
				session("alice", {
					lastRun: {
						runId: "run-aaaaaaaaaa",
						status: "error",
						executed: 12,
						ts: ctx.now - 7 * 60_000,
					},
				}),
			],
			"me",
		);
		expect(describeActivity(collab, ctx).lastRun).toEqual({
			status: "error",
			executed: 12,
			agoMinutes: 7,
		});
	});
});

describe("agoMinutes", () => {
	test("floors to whole minutes and clamps to [0, 24h]", () => {
		expect(agoMinutes(1000, 1000 + 59_999)).toBe(0);
		expect(agoMinutes(1000, 1000 + 60_000)).toBe(1);
		expect(agoMinutes(5000, 1000)).toBe(0);
		expect(agoMinutes(0, 48 * 60 * 60_000)).toBe(PRESENCE_AGO_CAP_MINUTES);
	});
});

describe("presenceEventRemainingMs", () => {
	test("counts down from the TTL and never goes negative", () => {
		const event = { sub: "alice", kind: "joined" as const, at: 10_000 };
		expect(presenceEventRemainingMs(undefined, 10_000)).toBe(0);
		expect(presenceEventRemainingMs(event, 10_000)).toBe(PRESENCE_EVENT_TTL_MS);
		expect(presenceEventRemainingMs(event, 11_500)).toBe(
			PRESENCE_EVENT_TTL_MS - 1500,
		);
		expect(
			presenceEventRemainingMs(event, 10_000 + PRESENCE_EVENT_TTL_MS),
		).toBe(0);
		expect(presenceEventRemainingMs(event, 99_999)).toBe(0);
		expect(presenceEventRemainingMs(event, 5000)).toBe(PRESENCE_EVENT_TTL_MS);
	});
});

describe("presenceHighlightIds", () => {
	test("unions the canvas selection with the editor's node anchors only", () => {
		const [collab] = mergeCollaborators(
			[
				session("alice", {
					selection: { nodes: ["n2"] },
					editor: {
						anchorId: "n1",
						anchorKind: "node",
						selectedAnchorIds: ["n3", "n2"],
					},
				}),
			],
			"me",
		);
		expect(presenceHighlightIds(collab)).toEqual(["n1", "n2", "n3"]);

		const [onLayer] = mergeCollaborators(
			[
				session("bob", {
					editor: {
						anchorId: "layer-1",
						anchorKind: "layer",
						selectedAnchorIds: [],
					},
				}),
			],
			"me",
		);
		expect(presenceHighlightIds(onLayer)).toEqual([]);
	});
});

describe("presenceStats", () => {
	test("counts distinct people including you, and who is in the code editor", () => {
		const rows = mergeCollaborators(
			[
				session("me", { codeFile: "main" }),
				session("me"),
				session("alice", {
					editor: { anchorId: "n1", anchorKind: "node", selectedAnchorIds: [] },
				}),
				session("bob"),
			],
			"me",
		);
		expect(presenceStats(rows)).toEqual({ onlineCount: 3, inCodeEditor: 2 });
		expect(presenceStats([])).toEqual({ onlineCount: 1, inCodeEditor: 0 });
	});
});
