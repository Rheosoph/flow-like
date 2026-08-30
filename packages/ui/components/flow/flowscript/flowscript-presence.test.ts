import { describe, expect, test } from "bun:test";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	collectCommandEntityIds,
	createFlowScriptPresencePublisher,
	createFlowScriptPresenceStore,
	createFlowScriptViewportPublisher,
	cursorToWire,
	deriveClaimedAnchorIds,
	deriveRemoteEditorsByNode,
	deriveScopesBySub,
	findClaimCollision,
	peersSharingFlowScriptScope,
	readPeerFlowScriptClaims,
	resolveWireCursor,
	resolveWireViewport,
	viewportToWire,
} from "./flowscript-presence";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	FLOWSCRIPT_SCOPE_FIELD,
	FLOWSCRIPT_VIEWPORT_FIELD,
	MAX_WIRE_DLINE,
	wireSafetyViolations,
} from "./flowscript-presence-protocol";

const LAYER_ID = "layeranchor000000001";
const CONST_ID = "nodeanchor0000000001";
const IF_ID = "nodeanchor0000000002";
const LOG_ID = "nodeanchor0000000003";

const SHARED_BODY = [
	`function main(): () {   //@l:${LAYER_ID}`,
	`	const path = createPath({ value: "x" })   //@n:${CONST_ID}`,
	`	if (exists({ path })) { // exec_out_exists   //@n:${IF_ID}`,
	`		log(path)   //@n:${LOG_ID}`,
	"	}",
	"}",
];

/** Two renders of the SAME board: identical anchors, different `use` blocks —
 *  absolute line numbers differ by 2 between the clients. */
const TEXT_A = ["use files::*", "", ...SHARED_BODY].join("\n");
const TEXT_B = [
	"use files::*",
	"use http::*",
	"use json::*",
	"",
	...SHARED_BODY,
].join("\n");

const caret = (lineNumber: number, column: number) => ({
	positionLineNumber: lineNumber,
	positionColumn: column,
	selectionStartLineNumber: lineNumber,
	selectionStartColumn: column,
});

function wireCursorOrThrow(
	index: ReturnType<typeof parseFlowScriptAnchors>,
	selection: Parameters<typeof cursorToWire>[1],
) {
	const payload = cursorToWire(index, selection, 1_000);
	if (!payload) throw new Error("expected a wire cursor payload");
	return payload;
}

describe("anchor-relative cursor round-trip", () => {
	test("a cursor survives the wire onto a render with a different use block", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		// TEXT_A: log(path) sits on line 6; TEXT_B renders it on line 8.
		const payload = wireCursorOrThrow(indexA, caret(6, 7));
		expect(payload.anchor).toEqual({ id: LOG_ID, kind: "node" });
		expect(payload.dLine).toBe(0);
		const resolved = resolveWireCursor(indexB, payload);
		expect(resolved).toEqual({ lineNumber: 8, column: 7 });
	});

	test("an un-anchored line travels as an offset below its owning anchor", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		// The `}` closing the if-block (line 7 in A) has no anchor of its own.
		const payload = wireCursorOrThrow(indexA, caret(7, 2));
		expect(payload.anchor.id).toBe(LOG_ID);
		expect(payload.dLine).toBe(1);
		expect(resolveWireCursor(indexB, payload)?.lineNumber).toBe(9);
	});

	test("a selection spanning two anchors resolves to a normalized range", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		const payload = wireCursorOrThrow(indexA, {
			positionLineNumber: 6,
			positionColumn: 5,
			selectionStartLineNumber: 4,
			selectionStartColumn: 2,
		});
		expect(payload.sel?.endAnchorId).toBe(CONST_ID);
		const resolved = resolveWireCursor(indexB, payload);
		expect(resolved?.selection).toEqual({
			startLineNumber: 6,
			startColumn: 2,
			endLineNumber: 8,
			endColumn: 5,
		});
	});

	test("above the first anchor there is nothing to publish", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		expect(cursorToWire(indexA, caret(1, 4), 1_000)).toBeUndefined();
	});

	test("an anchor unknown to the local render resolves to nothing", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const payload = wireCursorOrThrow(indexA, caret(6, 7));
		const foreignIndex = parseFlowScriptAnchors(
			"const y = 1   //@n:someotheranchor00001",
		);
		expect(resolveWireCursor(foreignIndex, payload)).toBeUndefined();
	});
});

describe("claim derivation from baseline/current pairs", () => {
	test("a clean buffer claims nothing", () => {
		expect(deriveClaimedAnchorIds(TEXT_A, TEXT_A)).toEqual([]);
	});

	test("editing one statement claims exactly its anchor", () => {
		const edited = TEXT_A.replace("log(path)", "log(path, path)");
		expect(deriveClaimedAnchorIds(TEXT_A, edited)).toEqual([LOG_ID]);
	});

	test("inserting an un-anchored line claims the statement above the seam", () => {
		const lines = TEXT_A.split("\n");
		lines.splice(4, 0, "	const extra = 1");
		expect(deriveClaimedAnchorIds(TEXT_A, lines.join("\n"))).toContain(
			CONST_ID,
		);
	});

	test("deleting a statement claims the anchor that vanished", () => {
		const lines = TEXT_A.split("\n");
		lines.splice(5, 1); // remove the log(path) line
		expect(deriveClaimedAnchorIds(TEXT_A, lines.join("\n"))).toContain(LOG_ID);
	});

	test("the claim set is bounded", () => {
		const bigBaseline = Array.from(
			{ length: 300 },
			(_, i) =>
				`const v${i} = 1   //@n:bulkanchor${String(i).padStart(9, "0")}`,
		).join("\n");
		const bigEdited = bigBaseline.replaceAll("= 1", "= 2");
		expect(
			deriveClaimedAnchorIds(bigBaseline, bigEdited).length,
		).toBeLessThanOrEqual(64);
	});
});

class FakeAwareness {
	clientID = 1;
	states = new Map<number, Record<string, unknown>>();
	published: [string, unknown][] = [];
	private listeners = new Set<() => void>();
	getStates() {
		return this.states;
	}
	on(_event: "change", cb: () => void) {
		this.listeners.add(cb);
	}
	off(_event: "change", cb: () => void) {
		this.listeners.delete(cb);
	}
	emitChange() {
		for (const cb of this.listeners) cb();
	}
	setLocalStateField(field: string, value: unknown) {
		this.published.push([field, value]);
	}
}

function fakeRaf() {
	const frames: (() => void)[] = [];
	return {
		raf: (cb: () => void) => {
			frames.push(cb);
			return frames.length;
		},
		caf: () => {},
		flushFrame: () => {
			const pending = frames.splice(0);
			for (const cb of pending) cb();
		},
	};
}

const peerCursor = (dLine: number, ts = 1) => ({
	anchor: { id: LOG_ID, kind: "node" },
	dLine,
	column: 3,
	ts,
});

describe("presence store coalescing", () => {
	test("N awareness bursts inside one frame emit exactly one store change", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, { sub: "peer-a", flowscriptCursor: peerCursor(0) });
		const { raf, caf, flushFrame } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		let emissions = 0;
		store.subscribe(() => emissions++);
		expect(store.getSnapshot().cursors.length).toBe(1);

		for (let i = 1; i <= 5; i++) {
			awareness.states.set(2, {
				sub: "peer-a",
				flowscriptCursor: peerCursor(i),
			});
			awareness.emitChange();
		}
		expect(emissions).toBe(0);
		flushFrame();
		expect(emissions).toBe(1);
		expect(store.getSnapshot().cursors[0]?.cursor.dLine).toBe(5);
		store.dispose();
	});

	test("heartbeats (ts-only changes) short-circuit without notifying", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, {
			sub: "peer-a",
			flowscriptCursor: peerCursor(2, 100),
		});
		const { raf, caf, flushFrame } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		let emissions = 0;
		store.subscribe(() => emissions++);
		awareness.states.set(2, {
			sub: "peer-a",
			flowscriptCursor: peerCursor(2, 200),
		});
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(0);
		store.dispose();
	});

	test("filters only this client; own other sessions stay and are flagged self", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(1, {
			sub: "me",
			flowscriptCursor: peerCursor(0),
		});
		awareness.states.set(3, {
			sub: "me",
			flowscriptCursor: peerCursor(1),
		});
		awareness.states.set(4, {
			sub: "peer-b",
			flowscriptClaims: { anchorIds: [CONST_ID, IF_ID], ts: 1 },
			selection: {
				nodes: ["nodeanchor0000000009", 42, "nodeanchor0000000008"],
			},
		});
		const { raf, caf } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, {
			raf,
			caf,
			selfSub: "me",
		});
		const snapshot = store.getSnapshot();
		expect(snapshot.cursors.map((c) => [c.clientId, c.sub, c.self])).toEqual([
			[3, "me", true],
		]);
		expect(snapshot.claims).toEqual([
			{ clientId: 4, sub: "peer-b", self: false, anchorIds: [CONST_ID, IF_ID] },
		]);
		expect(snapshot.canvasSelections[0]?.nodeIds).toEqual([
			"nodeanchor0000000008",
			"nodeanchor0000000009",
		]);
		expect(snapshot.canvasSelections[0]?.activeNodeId).toBeUndefined();
		store.dispose();
	});

	test("a canvas click is fresh from when THIS client first saw it, then times out on its own", () => {
		const awareness = new FakeAwareness();
		// Already on the wire when the store comes up: old news, no flash — and
		// the peer's own clock (far ahead of ours) is never consulted.
		awareness.states.set(2, {
			sub: "peer-a",
			selection: { nodes: [IF_ID, LOG_ID] },
			activeNodeId: LOG_ID,
			activeNodeTs: 999_999_999,
		});
		const { raf, caf, flushFrame } = fakeRaf();
		const scheduler = fakeScheduler();
		let clock = 11_000;
		const store = createFlowScriptPresenceStore(awareness, {
			raf,
			caf,
			now: () => clock,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		expect(store.getSnapshot().canvasSelections[0]).toEqual({
			clientId: 2,
			sub: "peer-a",
			self: false,
			nodeIds: [IF_ID, LOG_ID],
		});

		// A new click (new timestamp) is fresh for ACTIVE_NODE_FRESH_MS local ms.
		awareness.states.set(2, {
			sub: "peer-a",
			selection: { nodes: [IF_ID, LOG_ID] },
			activeNodeId: IF_ID,
			activeNodeTs: 1_000_000_000,
		});
		awareness.emitChange();
		flushFrame();
		expect(store.getSnapshot().canvasSelections[0]?.activeNodeId).toBe(IF_ID);

		// Nothing else happens on the wire; the store's own expiry timer clears it.
		clock = 11_000 + 3_500;
		scheduler.flush();
		flushFrame();
		expect(
			store.getSnapshot().canvasSelections[0]?.activeNodeId,
		).toBeUndefined();
		store.dispose();
	});
});

describe("text selections name the nodes they span", () => {
	test("a range covers the statement it starts in through its last anchored line", () => {
		const index = parseFlowScriptAnchors(TEXT_A);
		// TEXT_A lines: 3 function, 4 const, 5 if, 6 log, 7 }, 8 }
		const payload = wireCursorOrThrow(index, {
			selectionStartLineNumber: 4,
			selectionStartColumn: 10,
			positionLineNumber: 7,
			positionColumn: 2,
		});
		expect(payload.sel?.anchorIds).toEqual([CONST_ID, IF_ID, LOG_ID]);
		// Reverse direction (cursor at the top) yields the same set.
		const reversed = wireCursorOrThrow(index, {
			selectionStartLineNumber: 7,
			selectionStartColumn: 2,
			positionLineNumber: 4,
			positionColumn: 10,
		});
		expect(reversed.sel?.anchorIds).toEqual([CONST_ID, IF_ID, LOG_ID]);
	});

	test("a whole-line selection ending at column 1 does not claim the next statement", () => {
		const index = parseFlowScriptAnchors(TEXT_A);
		const payload = wireCursorOrThrow(index, {
			selectionStartLineNumber: 4,
			selectionStartColumn: 1,
			positionLineNumber: 5,
			positionColumn: 1,
		});
		expect(payload.sel?.anchorIds).toEqual([CONST_ID]);
	});

	test("a selection inside one statement names just that statement; a caret names none", () => {
		const index = parseFlowScriptAnchors(TEXT_A);
		const inside = wireCursorOrThrow(index, {
			selectionStartLineNumber: 6,
			selectionStartColumn: 3,
			positionLineNumber: 6,
			positionColumn: 9,
		});
		expect(inside.sel?.anchorIds).toEqual([LOG_ID]);
		expect(wireCursorOrThrow(index, caret(6, 3)).sel).toBeUndefined();
	});
});

describe("remote editors by node (canvas projection)", () => {
	test("cursors mark nodes active; claims attach passively; variables are skipped", () => {
		const byNode = deriveRemoteEditorsByNode({
			cursors: [
				{
					clientId: 2,
					sub: "peer-a",
					cursor: {
						anchor: { id: LOG_ID, kind: "node" },
						dLine: 0,
						column: 1,
						ts: 1,
					},
				},
				{
					clientId: 5,
					sub: "peer-c",
					cursor: {
						anchor: { id: CONST_ID, kind: "variable" },
						dLine: 0,
						column: 1,
						ts: 1,
					},
				},
			],
			claims: [{ clientId: 2, sub: "peer-a", anchorIds: [LOG_ID, IF_ID] }],
			canvasSelections: [],
			scopes: new Map(),
			viewports: new Map(),
		});
		expect(byNode.get(LOG_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", active: true, selected: true },
		]);
		expect(byNode.get(IF_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", active: false, selected: false },
		]);
		expect(byNode.has(CONST_ID)).toBe(false);
	});

	test("a text selection marks every spanned node selected; the caret node stays active", () => {
		const byNode = deriveRemoteEditorsByNode({
			cursors: [
				{
					clientId: 2,
					sub: "peer-a",
					self: true,
					cursor: {
						anchor: { id: LOG_ID, kind: "node" },
						dLine: 0,
						column: 1,
						sel: {
							endAnchorId: CONST_ID,
							endDLine: 0,
							endColumn: 1,
							anchorIds: [CONST_ID, IF_ID, LOG_ID],
						},
						ts: 1,
					},
				},
			],
			claims: [{ clientId: 2, sub: "peer-a", anchorIds: [CONST_ID] }],
			canvasSelections: [],
			scopes: new Map(),
			viewports: new Map(),
		});
		expect(byNode.get(LOG_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", self: true, active: true, selected: true },
		]);
		expect(byNode.get(IF_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", self: true, active: false, selected: true },
		]);
		// Selection outranks the claim on the same node for the same user.
		expect(byNode.get(CONST_ID)?.[0]?.selected).toBe(true);
	});
});

function fakeScheduler() {
	let nextId = 1;
	const tasks = new Map<number, () => void>();
	return {
		schedule: (cb: () => void, _ms: number) => {
			const id = nextId++;
			tasks.set(id, cb);
			return id;
		},
		cancel: (handle: unknown) => {
			tasks.delete(handle as number);
		},
		flush: () => {
			const pending = [...tasks.values()];
			tasks.clear();
			for (const cb of pending) cb();
		},
	};
}

function makePublisher() {
	const awareness = new FakeAwareness();
	const scheduler = fakeScheduler();
	const index = parseFlowScriptAnchors(TEXT_A);
	let nowMs = 10_000;
	const publisher = createFlowScriptPresencePublisher({
		awareness,
		getAnchorIndex: () => index,
		now: () => nowMs,
		schedule: scheduler.schedule,
		cancel: scheduler.cancel,
	});
	return {
		awareness,
		scheduler,
		publisher,
		advance: (ms: number) => {
			nowMs += ms;
		},
	};
}

describe("presence publisher gating", () => {
	test("a clean buffer publishes no claims at all", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		publisher.scheduleClaims(TEXT_A, TEXT_A);
		scheduler.flush();
		expect(awareness.published).toEqual([]);
		publisher.dispose();
		expect(awareness.published).toEqual([]);
	});

	test("a dirty buffer publishes its claimed anchors, and going clean clears them", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		const edited = TEXT_A.replace("log(path)", "log(path, path)");
		publisher.scheduleClaims(TEXT_A, edited);
		scheduler.flush();
		expect(awareness.published).toEqual([
			[FLOWSCRIPT_CLAIMS_FIELD, { anchorIds: [LOG_ID], ts: 10_000 }],
		]);
		publisher.scheduleClaims(TEXT_A, TEXT_A);
		scheduler.flush();
		expect(awareness.published[1]).toEqual([
			FLOWSCRIPT_CLAIMS_FIELD,
			undefined,
		]);
	});

	test("rapid claim edits collapse onto the trailing debounce", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		publisher.scheduleClaims(TEXT_A, TEXT_A.replace("log(path)", "log(1)"));
		publisher.scheduleClaims(TEXT_A, TEXT_A.replace("log(path)", "log(2)"));
		scheduler.flush();
		expect(awareness.published.length).toBe(1);
	});

	test("cursor publishes only on change and clears on demand", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		publisher.publishCursor(caret(6, 7));
		// Never synchronous: Monaco fires the selection change before React has
		// committed the edited text, so the flush waits for the next tick.
		expect(awareness.published.length).toBe(0);
		scheduler.flush();
		expect(awareness.published.length).toBe(1);
		expect(awareness.published[0][0]).toBe(FLOWSCRIPT_CURSOR_FIELD);

		// Same position again: throttled to a trailing tick, then key-deduped.
		publisher.publishCursor(caret(6, 7));
		scheduler.flush();
		expect(awareness.published.length).toBe(1);

		publisher.clearCursor();
		expect(awareness.published[1]).toEqual([
			FLOWSCRIPT_CURSOR_FIELD,
			undefined,
		]);

		// Cleared twice stays a single clear.
		publisher.clearCursor();
		expect(awareness.published.length).toBe(2);
	});

	test("cursor moves are throttled to the trailing edge (≤ 20Hz)", () => {
		const { awareness, scheduler, publisher, advance } = makePublisher();
		publisher.publishCursor(caret(4, 2));
		scheduler.flush();
		expect(awareness.published.length).toBe(1);
		advance(10);
		publisher.publishCursor(caret(6, 7)); // within min interval — deferred
		publisher.publishCursor(caret(6, 8)); // coalesced onto the same tick
		expect(awareness.published.length).toBe(1);
		scheduler.flush();
		expect(awareness.published.length).toBe(2);
		const payload = awareness.published[1][1] as {
			anchor: { id: string };
			column: number;
		};
		expect(payload.anchor.id).toBe(LOG_ID);
		expect(payload.column).toBe(8);
	});

	test("dispose withdraws published presence from the wire", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		publisher.publishCursor(caret(6, 7));
		publisher.scheduleClaims(TEXT_A, TEXT_A.replace("log(path)", "log(2)"));
		scheduler.flush();
		publisher.dispose();
		const cleared = awareness.published
			.slice(-2)
			.map(([field, value]) => [field, value]);
		expect(cleared).toEqual([
			[FLOWSCRIPT_CURSOR_FIELD, undefined],
			[FLOWSCRIPT_CLAIMS_FIELD, undefined],
		]);
	});
});

describe("undo/apply claim collisions (rule 3)", () => {
	const NODE_A = "nodeanchorentity0001";
	const NODE_B = "nodeanchorentity0002";
	const VAR_A = "varanchorentity00001";

	test("collects every entity id shape an undo batch can carry", () => {
		const ids = collectCommandEntityIds([
			{ command_type: "RemoveNode", node: { id: NODE_A } },
			{
				command_type: "ConnectPin",
				from_node: NODE_B,
				to_node: "nodeanchorentity0003",
			},
			{ command_type: "UpsertVariable", variable: { id: VAR_A } },
			{ command_type: "MoveNode", node_id: "nodeanchorentity0004" },
			{
				command_type: "CopyPaste",
				new_nodes: [{ id: "nodeanchorentity0005" }],
				original_nodes: [{ id: "nodeanchorentity0006" }],
			},
			{ command_type: "UpsertLayer", layer: { id: "layeranchorentity001" } },
		]);
		for (const id of [
			NODE_A,
			NODE_B,
			"nodeanchorentity0003",
			VAR_A,
			"nodeanchorentity0004",
			"nodeanchorentity0005",
			"nodeanchorentity0006",
			"layeranchorentity001",
		]) {
			expect(ids.has(id)).toBe(true);
		}
	});

	test("finds the first peer whose claims intersect the batch", () => {
		const claims = [
			{ clientId: 2, sub: "peer-a", anchorIds: [NODE_B] },
			{ clientId: 3, sub: "peer-b", anchorIds: [NODE_A, VAR_A] },
		];
		expect(findClaimCollision(claims, new Set([VAR_A]))?.sub).toBe("peer-b");
		expect(findClaimCollision(claims, new Set([NODE_B]))?.sub).toBe("peer-a");
		expect(
			findClaimCollision(claims, new Set(["nodeanchorunclaimed1"])),
		).toBeUndefined();
		expect(findClaimCollision(claims, new Set())).toBeUndefined();
	});

	test("readPeerFlowScriptClaims sanitizes awareness state and skips self", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(1, {
			sub: "me",
			[FLOWSCRIPT_CLAIMS_FIELD]: { anchorIds: [NODE_A], ts: 1 },
		});
		awareness.states.set(2, {
			sub: "peer-a",
			[FLOWSCRIPT_CLAIMS_FIELD]: { anchorIds: [NODE_B, "x"], ts: 1 },
		});
		awareness.states.set(3, {
			sub: "me-elsewhere",
			[FLOWSCRIPT_CLAIMS_FIELD]: { anchorIds: "not-a-list" },
		});
		const claims = readPeerFlowScriptClaims(awareness);
		expect(claims).toEqual([
			{ clientId: 2, sub: "peer-a", anchorIds: [NODE_B] },
		]);
		// Filtering by sub drops the local user's other sessions too.
		awareness.states.set(4, {
			sub: "same-user",
			[FLOWSCRIPT_CLAIMS_FIELD]: { anchorIds: [NODE_A], ts: 1 },
		});
		expect(readPeerFlowScriptClaims(awareness, "same-user")).toEqual([
			{ clientId: 2, sub: "peer-a", anchorIds: [NODE_B] },
		]);
	});
});

describe("shared scoped sessions (presence store + helpers)", () => {
	const scopeState = (nodeIds: string[], ts = 1) => ({
		[FLOWSCRIPT_SCOPE_FIELD]: { nodeIds, ts },
	});

	test("collects peers' scopes into a clientId-keyed map, sanitized, this client filtered", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(1, { sub: "me", ...scopeState([CONST_ID]) });
		awareness.states.set(2, { sub: "me", ...scopeState([CONST_ID]) });
		awareness.states.set(3, {
			sub: "peer-a",
			...scopeState([CONST_ID, "free text!", IF_ID]),
		});
		awareness.states.set(4, {
			sub: "peer-b",
			[FLOWSCRIPT_SCOPE_FIELD]: { nodeIds: [], ts: 1 },
		});
		const { raf, caf } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, {
			raf,
			caf,
			selfSub: "me",
		});
		const scopes = store.getSnapshot().scopes;
		// Our own second window (client 2) is a joinable peer like any other.
		expect([...scopes.keys()]).toEqual([2, 3]);
		expect(scopes.get(2)).toEqual({
			sub: "me",
			self: true,
			nodeIds: [CONST_ID],
		});
		expect(scopes.get(3)).toEqual({
			sub: "peer-a",
			self: false,
			nodeIds: [CONST_ID, IF_ID],
		});
		store.dispose();
	});

	test("scope heartbeats (ts-only changes) short-circuit without notifying", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, { sub: "peer-a", ...scopeState([CONST_ID], 100) });
		const { raf, caf, flushFrame } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		let emissions = 0;
		store.subscribe(() => emissions++);
		awareness.states.set(2, { sub: "peer-a", ...scopeState([CONST_ID], 200) });
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(0);

		awareness.states.set(2, {
			sub: "peer-a",
			...scopeState([CONST_ID, IF_ID], 300),
		});
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(1);
		expect(store.getSnapshot().scopes.get(2)?.nodeIds).toEqual([
			CONST_ID,
			IF_ID,
		]);
		store.dispose();
	});

	test("a withdrawn scope leaves the map", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, { sub: "peer-a", ...scopeState([CONST_ID]) });
		const { raf, caf, flushFrame } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		expect(store.getSnapshot().scopes.size).toBe(1);
		awareness.states.set(2, { sub: "peer-a" });
		awareness.emitChange();
		flushFrame();
		expect(store.getSnapshot().scopes.size).toBe(0);
		store.dispose();
	});

	test("peersSharingFlowScriptScope matches on SET equality, not render order", () => {
		const scopes = new Map([
			[2, { sub: "peer-a", nodeIds: [IF_ID, CONST_ID] }],
			[3, { sub: "peer-b", nodeIds: [CONST_ID] }],
			[4, { sub: "peer-c", nodeIds: [CONST_ID, IF_ID, LOG_ID] }],
		]);
		expect(peersSharingFlowScriptScope(scopes, [CONST_ID, IF_ID])).toEqual([
			{ clientId: 2, sub: "peer-a" },
		]);
		expect(peersSharingFlowScriptScope(scopes, [CONST_ID])).toEqual([
			{ clientId: 3, sub: "peer-b" },
		]);
		expect(peersSharingFlowScriptScope(scopes, [LOG_ID])).toEqual([]);
		expect(peersSharingFlowScriptScope(scopes, [])).toEqual([]);
	});

	test("peersSharingFlowScriptScope lists one entry per user across sessions", () => {
		const scopes = new Map([
			[2, { sub: "peer-a", nodeIds: [CONST_ID] }],
			[5, { sub: "peer-a", nodeIds: [CONST_ID] }],
			[7, { sub: undefined, nodeIds: [CONST_ID] }],
			[8, { sub: undefined, nodeIds: [CONST_ID] }],
		]);
		expect(peersSharingFlowScriptScope(scopes, [CONST_ID])).toEqual([
			{ clientId: 2, sub: "peer-a" },
			{ clientId: 7, sub: undefined },
			{ clientId: 8, sub: undefined },
		]);
	});

	test("deriveScopesBySub keys by sub, first session wins, sub-less dropped", () => {
		const scopes = new Map([
			[2, { sub: "peer-a", nodeIds: [CONST_ID] }],
			[5, { sub: "peer-a", nodeIds: [IF_ID] }],
			[6, { sub: undefined, nodeIds: [LOG_ID] }],
			[7, { sub: "peer-b", nodeIds: [LOG_ID] }],
		]);
		const bySub = deriveScopesBySub(scopes);
		expect([...bySub.keys()]).toEqual(["peer-a", "peer-b"]);
		expect(bySub.get("peer-a")).toEqual([CONST_ID]);
		expect(bySub.get("peer-b")).toEqual([LOG_ID]);
	});
});

function wireViewportOrThrow(
	index: ReturnType<typeof parseFlowScriptAnchors>,
	firstVisibleLine: number,
) {
	const payload = viewportToWire(index, firstVisibleLine, 1_000);
	if (!payload) throw new Error("expected a wire viewport payload");
	return payload;
}

describe("anchor-relative viewport round-trip (scroll-follow)", () => {
	test("the first visible line survives the wire onto a render with a different use block", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		// TEXT_A shows the `if` on line 5; TEXT_B renders it on line 7.
		const payload = wireViewportOrThrow(indexA, 5);
		expect(payload).toEqual({
			anchor: { id: IF_ID, kind: "node" },
			dLine: 0,
			ts: 1_000,
		});
		expect(resolveWireViewport(indexB, payload)).toBe(7);
	});

	test("an un-anchored top line travels as an offset below its owning anchor", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		// The `}` closing the if-block: line 7 in A, line 9 in B.
		const payload = wireViewportOrThrow(indexA, 7);
		expect(payload.anchor.id).toBe(LOG_ID);
		expect(payload.dLine).toBe(1);
		expect(resolveWireViewport(indexB, payload)).toBe(9);
	});

	test("above the first anchor there is nothing to publish", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		expect(viewportToWire(indexA, 1, 1_000)).toBeUndefined();
	});

	test("an unknown anchor resolves to nothing; a known one clamps to the buffer", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const indexB = parseFlowScriptAnchors(TEXT_B);
		const payload = wireViewportOrThrow(indexA, 7);
		const foreignIndex = parseFlowScriptAnchors(
			"const y = 1   //@n:someotheranchor00001",
		);
		expect(resolveWireViewport(foreignIndex, payload)).toBeUndefined();
		expect(resolveWireViewport(indexB, payload, 8)).toBe(8);
	});

	test("a published viewport is metadata-only (rule 2)", () => {
		const indexA = parseFlowScriptAnchors(TEXT_A);
		const payload = wireViewportOrThrow(indexA, 7);
		expect(wireSafetyViolations(payload)).toEqual([]);
		expect(Object.keys(payload).sort()).toEqual(["anchor", "dLine", "ts"]);
	});
});

function makeViewportPublisher() {
	const awareness = new FakeAwareness();
	const index = parseFlowScriptAnchors(TEXT_A);
	let nowMs = 10_000;
	const delays: number[] = [];
	const tasks = new Map<number, () => void>();
	let nextId = 1;
	const publisher = createFlowScriptViewportPublisher({
		awareness,
		getAnchorIndex: () => index,
		now: () => nowMs,
		schedule: (cb, ms) => {
			delays.push(ms);
			const id = nextId++;
			tasks.set(id, cb);
			return id;
		},
		cancel: (handle) => {
			tasks.delete(handle as number);
		},
	});
	return {
		awareness,
		publisher,
		delays,
		flush: () => {
			const pending = [...tasks.values()];
			tasks.clear();
			for (const cb of pending) cb();
		},
		advance: (ms: number) => {
			nowMs += ms;
		},
	};
}

describe("viewport publisher (≤ 5Hz, change-gated)", () => {
	test("publishes on the trailing tick, then only when the anchor-relative top changed", () => {
		const { awareness, publisher, delays, flush } = makeViewportPublisher();
		publisher.publish(5);
		expect(awareness.published.length).toBe(0);
		expect(delays).toEqual([0]);
		flush();
		expect(awareness.published).toEqual([
			[
				FLOWSCRIPT_VIEWPORT_FIELD,
				{ anchor: { id: IF_ID, kind: "node" }, dLine: 0, ts: 10_000 },
			],
		]);
		// Same top line again: throttled to a tick, then key-deduped.
		publisher.publish(5);
		flush();
		expect(awareness.published.length).toBe(1);
	});

	test("scroll bursts inside the interval collapse onto one ≥200ms-spaced publish", () => {
		const { awareness, publisher, delays, flush, advance } =
			makeViewportPublisher();
		publisher.publish(4);
		flush();
		expect(awareness.published.length).toBe(1);
		advance(50);
		publisher.publish(6); // within min interval — deferred
		publisher.publish(7); // coalesced onto the same tick
		expect(delays).toEqual([0, 150]);
		expect(awareness.published.length).toBe(1);
		flush();
		expect(awareness.published.length).toBe(2);
		expect(awareness.published[1][1]).toEqual({
			anchor: { id: LOG_ID, kind: "node" },
			dLine: 1,
			ts: 10_050,
		});
	});

	test("scrolling above the first anchor clears the field; dispose withdraws a live one", () => {
		const { awareness, publisher, flush } = makeViewportPublisher();
		publisher.publish(6);
		flush();
		publisher.publish(1);
		flush();
		expect(awareness.published[1]).toEqual([
			FLOWSCRIPT_VIEWPORT_FIELD,
			undefined,
		]);
		// Already cleared: dispose stays silent.
		publisher.dispose();
		expect(awareness.published.length).toBe(2);

		const live = makeViewportPublisher();
		live.publisher.publish(6);
		live.flush();
		live.publisher.publish(7); // pending tick is cancelled, never published
		live.publisher.dispose();
		expect(live.awareness.published).toEqual([
			[
				FLOWSCRIPT_VIEWPORT_FIELD,
				{ anchor: { id: LOG_ID, kind: "node" }, dLine: 0, ts: 10_000 },
			],
			[FLOWSCRIPT_VIEWPORT_FIELD, undefined],
		]);
	});
});

describe("presence store viewports", () => {
	const viewportState = (dLine: number, ts = 1, id: string = IF_ID) => ({
		[FLOWSCRIPT_VIEWPORT_FIELD]: { anchor: { id, kind: "node" }, dLine, ts },
	});

	test("collects peers' viewports keyed by clientId; this client filtered, own sessions flagged self", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(1, { sub: "me", ...viewportState(0) });
		awareness.states.set(2, { sub: "me", ...viewportState(1) });
		awareness.states.set(3, { sub: "peer-a", ...viewportState(2, 5, LOG_ID) });
		awareness.states.set(4, { sub: "peer-b" });
		const { raf, caf } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, {
			raf,
			caf,
			selfSub: "me",
		});
		const viewports = store.getSnapshot().viewports;
		expect([...viewports.keys()]).toEqual([2, 3]);
		expect(viewports.get(2)).toEqual({
			sub: "me",
			self: true,
			viewport: { anchor: { id: IF_ID, kind: "node" }, dLine: 1, ts: 1 },
		});
		expect(viewports.get(3)).toEqual({
			sub: "peer-a",
			self: false,
			viewport: { anchor: { id: LOG_ID, kind: "node" }, dLine: 2, ts: 5 },
		});
		store.dispose();
	});

	test("viewport heartbeats (ts-only changes) short-circuit; a move notifies once", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, { sub: "peer-a", ...viewportState(2, 100) });
		const { raf, caf, flushFrame } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		let emissions = 0;
		store.subscribe(() => emissions++);
		const before = store.getSnapshot();

		awareness.states.set(2, { sub: "peer-a", ...viewportState(2, 200) });
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(0);
		expect(store.getSnapshot()).toBe(before);

		awareness.states.set(2, { sub: "peer-a", ...viewportState(3, 300) });
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(1);
		expect(store.getSnapshot().viewports.get(2)?.viewport.dLine).toBe(3);

		awareness.states.set(2, { sub: "peer-a" });
		awareness.emitChange();
		flushFrame();
		expect(emissions).toBe(2);
		expect(store.getSnapshot().viewports.size).toBe(0);
		store.dispose();
	});

	test("hostile viewport payloads are rejected or clamped, never passed through", () => {
		const awareness = new FakeAwareness();
		awareness.states.set(2, {
			sub: "peer-a",
			[FLOWSCRIPT_VIEWPORT_FIELD]: {
				anchor: { id: "DROP TABLE users; --", kind: "node" },
				dLine: 0,
				ts: 1,
			},
		});
		awareness.states.set(3, {
			sub: "peer-b",
			[FLOWSCRIPT_VIEWPORT_FIELD]: {
				anchor: { id: IF_ID, kind: "node", label: "x".repeat(4_096) },
				dLine: 1e9,
				ts: -5,
				text: "const secret = 42",
			},
		});
		awareness.states.set(4, {
			sub: "peer-c",
			[FLOWSCRIPT_VIEWPORT_FIELD]: "not an object",
		});
		awareness.states.set(5, {
			sub: "peer-d",
			[FLOWSCRIPT_VIEWPORT_FIELD]: {
				anchor: { id: IF_ID, kind: "comment" },
				dLine: 0,
				ts: 1,
			},
		});
		awareness.states.set(6, {
			sub: "peer-e",
			[FLOWSCRIPT_VIEWPORT_FIELD]: {
				anchor: { id: IF_ID, kind: "node" },
				dLine: Number.NaN,
				ts: 1,
			},
		});
		const { raf, caf } = fakeRaf();
		const store = createFlowScriptPresenceStore(awareness, { raf, caf });
		const viewports = store.getSnapshot().viewports;
		expect([...viewports.keys()]).toEqual([3]);
		const clamped = viewports.get(3)?.viewport;
		expect(clamped).toEqual({
			anchor: { id: IF_ID, kind: "node" },
			dLine: MAX_WIRE_DLINE,
			ts: 0,
		});
		expect(wireSafetyViolations(clamped)).toEqual([]);
		store.dispose();
	});
});
