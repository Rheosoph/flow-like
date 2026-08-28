import { describe, expect, test } from "bun:test";
import {
	createPeerActivityTracker,
	peerPresenceEqual,
	readPeerPresence,
} from "./peer-presence";

const NODE_A = "nodeanchor0000000001";
const NODE_B = "nodeanchor0000000002";
const MODULE = "layeranchor000000001";

describe("readPeerPresence", () => {
	test("reads every surface of a session through the wire sanitizers", () => {
		const presence = readPeerPresence(
			{
				sub: "peer-a",
				layerPath: "root/abc",
				selection: { nodes: [NODE_B, 7, NODE_A] },
				activeNodeId: NODE_A,
				activeNodeTs: 9_000,
				flowscriptCursor: {
					anchor: { id: NODE_A, kind: "node" },
					dLine: 1,
					column: 4,
					sel: { endDLine: 0, endColumn: 1, anchorIds: [NODE_A, NODE_B] },
					ts: 1,
				},
				flowscriptClaims: { anchorIds: [NODE_B, "leaked text"], ts: 1 },
				flowscriptScope: { nodeIds: [NODE_A], ts: 1 },
				flowscriptView: { file: MODULE, ts: 1 },
				executionPresence: { executingNodes: [NODE_B, NODE_A], sub: "peer-a" },
			},
			4,
			10_000,
			8_000,
		);
		expect(presence).toEqual({
			clientId: 4,
			sub: "peer-a",
			layerPath: "root/abc",
			selection: { nodes: [NODE_A, NODE_B] },
			activeNodeId: NODE_A,
			activeNodeTs: 9_000,
			editor: {
				anchorId: NODE_A,
				anchorKind: "node",
				selectedAnchorIds: [NODE_A, NODE_B],
			},
			codeFile: MODULE,
			claimedAnchorIds: [NODE_B],
			scopeNodeIds: [NODE_A],
			executingNodeIds: [NODE_A, NODE_B],
		});
	});

	test("an empty or hostile state degrades to an idle root presence", () => {
		const presence = readPeerPresence(
			{
				layerPath: "",
				flowscriptView: { file: "main.flow" },
				activeNodeId: NODE_A,
				activeNodeTs: 1,
			},
			2,
			10_000,
		);
		expect(presence.layerPath).toBe("root");
		expect(presence.codeFile).toBeUndefined();
		expect(presence.activeNodeId).toBeUndefined();
		expect(presence.editor).toBeUndefined();
		expect(readPeerPresence(undefined, 3, 0).selection.nodes).toEqual([]);
	});

	test("click freshness runs on the local clock from first sight, never the peer's clock", () => {
		const state = { activeNodeId: NODE_A, activeNodeTs: 999_999_999_999 };
		// No local sighting → not fresh, however new the peer claims it is.
		expect(readPeerPresence(state, 2, 10_000).activeNodeId).toBeUndefined();
		expect(readPeerPresence(state, 2, 10_000, 9_000).activeNodeId).toBe(NODE_A);
		expect(
			readPeerPresence(state, 2, 10_000, 5_000).activeNodeId,
		).toBeUndefined();
	});
});

describe("peerPresenceEqual", () => {
	test("ignores what the board does not render, notices what it does", () => {
		const base = readPeerPresence(
			{ sub: "p", selection: { nodes: [NODE_A] }, cursor: { x: 1, y: 1 } },
			2,
			0,
		);
		const moved = readPeerPresence(
			{ sub: "p", selection: { nodes: [NODE_A] }, cursor: { x: 9, y: 9 } },
			2,
			0,
		);
		expect(peerPresenceEqual(base, moved)).toBe(true);
		const typing = readPeerPresence(
			{
				sub: "p",
				selection: { nodes: [NODE_A] },
				flowscriptCursor: {
					anchor: { id: NODE_B, kind: "node" },
					dLine: 0,
					column: 1,
					ts: 1,
				},
			},
			2,
			0,
		);
		expect(peerPresenceEqual(base, typing)).toBe(false);
		const typingLater = readPeerPresence(
			{
				sub: "p",
				selection: { nodes: [NODE_A] },
				flowscriptCursor: {
					anchor: { id: NODE_B, kind: "node" },
					dLine: 3,
					column: 8,
					ts: 2,
				},
			},
			2,
			0,
		);
		// Moving within the same statement is not a presence change.
		expect(peerPresenceEqual(typing, typingLater)).toBe(true);
	});
});

describe("peer activity tracker", () => {
	test("stamps the local clock on change and answers per user across sessions", () => {
		let clock = 1_000;
		const tracker = createPeerActivityTracker(() => clock);
		const states = new Map<number, Record<string, unknown>>([
			[1, { sub: "me", cursor: { x: 0, y: 0 } }],
			[2, { sub: "peer", cursor: { x: 0, y: 0 } }],
			[3, { sub: "peer", cursor: { x: 5, y: 5 } }],
		]);
		tracker.observe(states, 1);
		expect(tracker.lastActiveAt("peer")).toBe(1_000);
		expect(tracker.lastActiveAt("me")).toBeUndefined();
		clock = 5_000;
		tracker.observe(states, 1);
		expect(tracker.lastActiveAt("peer")).toBe(1_000);
		states.set(3, { sub: "peer", cursor: { x: 6, y: 5 } });
		tracker.observe(states, 1);
		expect(tracker.lastActiveAt("peer")).toBe(5_000);
		states.delete(3);
		states.delete(2);
		tracker.observe(states, 1);
		expect(tracker.lastActiveAt("peer")).toBeUndefined();
	});

	test("typing predicates fire only for CHANGES on sessions we already knew, and expire", () => {
		let clock = 1_000;
		const tracker = createPeerActivityTracker(() => clock);
		const cursor = (column: number, ts: number) => ({
			anchor: { id: NODE_A, kind: "node" },
			dLine: 0,
			column,
			ts,
		});
		const states = new Map<number, Record<string, unknown>>([
			[
				2,
				{ sub: "peer", flowscriptCursor: cursor(1, 1), chatTyping: { ts: 1 } },
			],
		]);
		tracker.observe(states, 1);
		// Present at first sight: history, not typing.
		expect(tracker.isTypingInEditor("peer")).toBe(false);
		expect(tracker.isTypingInChat("peer")).toBe(false);
		// A session that appears later with a cursor is history too.
		clock = 2_000;
		states.set(3, { sub: "late", flowscriptCursor: cursor(4, 1_900) });
		tracker.observe(states, 1);
		expect(tracker.isTypingInEditor("late")).toBe(false);
		// A change on a known session is typing, until the TTL lapses.
		clock = 3_000;
		states.set(2, {
			sub: "peer",
			flowscriptCursor: cursor(2, 2_900),
			chatTyping: { ts: 2_950 },
		});
		tracker.observe(states, 1);
		expect(tracker.isTypingInEditor("peer")).toBe(true);
		expect(tracker.isTypingInChat("peer")).toBe(true);
		clock = 3_000 + 3_500;
		expect(tracker.isTypingInEditor("peer")).toBe(false);
		expect(tracker.isTypingInChat("peer")).toBe(false);
	});

	test("away flips after five quiet minutes and resets on any session change", () => {
		let clock = 10_000;
		const tracker = createPeerActivityTracker(() => clock);
		const states = new Map<number, Record<string, unknown>>([
			[2, { sub: "peer", cursor: { x: 0, y: 0 } }],
		]);
		tracker.observe(states, 1);
		expect(tracker.isAway("peer")).toBe(false);
		clock += 5 * 60_000;
		expect(tracker.isAway("peer")).toBe(true);
		states.set(2, { sub: "peer", cursor: { x: 1, y: 0 } });
		tracker.observe(states, 1);
		expect(tracker.isAway("peer")).toBe(false);
		expect(tracker.isAway("nobody")).toBe(false);
	});

	test("a canvas click already on the wire at first sight is not fresh; a new one is", () => {
		let clock = 1_000;
		const tracker = createPeerActivityTracker(() => clock);
		const states = new Map<number, Record<string, unknown>>([
			[2, { sub: "peer", activeNodeId: NODE_A, activeNodeTs: 500 }],
		]);
		tracker.observe(states, 1);
		expect(tracker.activeClickSeenAt(2)).toBeUndefined();
		clock = 4_000;
		states.set(2, { sub: "peer", activeNodeId: NODE_B, activeNodeTs: 3_900 });
		tracker.observe(states, 1);
		expect(tracker.activeClickSeenAt(2)).toBe(4_000);
		clock = 6_000;
		tracker.observe(states, 1);
		expect(tracker.activeClickSeenAt(2)).toBe(4_000);
		states.set(2, { sub: "peer" });
		tracker.observe(states, 1);
		expect(tracker.activeClickSeenAt(2)).toBeUndefined();
	});
});
