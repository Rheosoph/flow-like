import { describe, expect, test } from "bun:test";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	collectCommandEntityIds,
	createFlowScriptPresencePublisher,
	createFlowScriptPresenceStore,
	cursorToWire,
	deriveClaimedAnchorIds,
	deriveRemoteEditorsByNode,
	findClaimCollision,
	readPeerFlowScriptClaims,
	resolveWireCursor,
} from "./flowscript-presence";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
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

	test("filters self (clientID and own sub), collects claims and canvas selections", () => {
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
		expect(snapshot.cursors).toEqual([]);
		expect(snapshot.claims).toEqual([
			{ clientId: 4, sub: "peer-b", anchorIds: [CONST_ID, IF_ID] },
		]);
		expect(snapshot.canvasSelections[0]?.nodeIds).toEqual([
			"nodeanchor0000000008",
			"nodeanchor0000000009",
		]);
		store.dispose();
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
		});
		expect(byNode.get(LOG_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", active: true },
		]);
		expect(byNode.get(IF_ID)).toEqual([
			{ clientId: 2, sub: "peer-a", active: false },
		]);
		expect(byNode.has(CONST_ID)).toBe(false);
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

	test("cursor publishes only on change and clears on blur", () => {
		const { awareness, scheduler, publisher } = makePublisher();
		publisher.publishCursor(caret(6, 7));
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
		advance(10);
		publisher.publishCursor(caret(6, 7)); // within min interval — deferred
		expect(awareness.published.length).toBe(1);
		scheduler.flush();
		expect(awareness.published.length).toBe(2);
		const payload = awareness.published[1][1] as { anchor: { id: string } };
		expect(payload.anchor.id).toBe(LOG_ID);
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
