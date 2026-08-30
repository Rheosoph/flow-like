import { describe, expect, test } from "bun:test";
import {
	type IComment,
	ICommentType,
	type ISystemTime,
} from "../../../lib/schema/flow/board";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	FLOWSCRIPT_COMMENT_OFFSET_X,
	FLOWSCRIPT_COMMENT_OFFSET_Y,
	buildFlowScriptComment,
	canModifyFlowScriptComment,
	commentTimestampMs,
	deriveFlowScriptCommentAddLines,
	deriveFlowScriptCommentIndicators,
	deriveFlowScriptCommentThreads,
	formatFlowScriptCommentPreview,
	withFlowScriptCommentContent,
} from "./flowscript-comments";

const SCRIPT = [
	"function process() { //@l:layerFn",
	'\tconst greeting = "hi"; //@v:varGreeting',
	"\tlet result = add(1, 2); //@n:nodeAdd",
	"\tprint(result); //@n:nodePrint",
	"}",
].join("\n");

/** Same statements with two lines inserted above `nodeAdd` (anchors moved). */
const MOVED_SCRIPT = [
	"function process() { //@l:layerFn",
	'\tconst greeting = "hi"; //@v:varGreeting',
	"\t// a stray comment",
	"\tlet other = 1; //@n:nodeOther",
	"\tlet result = add(1, 2); //@n:nodeAdd",
	"\tprint(result); //@n:nodePrint",
	"}",
].join("\n");

const INDEX = parseFlowScriptAnchors(SCRIPT);
const MOVED_INDEX = parseFlowScriptAnchors(MOVED_SCRIPT);

function at(secs: number): ISystemTime {
	return { secs_since_epoch: secs, nanos_since_epoch: 0 };
}

function comment(id: string, overrides: Partial<IComment> = {}): IComment {
	return {
		id,
		comment_type: ICommentType.Text,
		content: `content-${id}`,
		coordinates: [0, 0, 0],
		timestamp: at(100),
		...overrides,
	};
}

describe("FlowScript comment thread derivation", () => {
	test("groups text comments by node_id into line-sorted threads ordered by timestamp", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				b: comment("b", { node_id: "nodeAdd", timestamp: at(200) }),
				a: comment("a", { node_id: "nodeAdd", timestamp: at(100) }),
				c: comment("c", { node_id: "nodePrint", timestamp: at(50) }),
			},
			INDEX,
		);
		expect(model.threads.map((thread) => thread.anchorId)).toEqual([
			"nodeAdd",
			"nodePrint",
		]);
		expect(model.threads[0].line).toBe(3);
		expect(model.threads[1].line).toBe(4);
		expect(model.threads[0].comments.map((entry) => entry.id)).toEqual([
			"a",
			"b",
		]);
		expect(model.threadsByAnchorId.get("nodePrint")?.comments[0].id).toBe("c");
		expect(model.unanchored).toEqual([]);
	});

	test("equal timestamps fall back to id order for a deterministic thread", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				z: comment("z", { node_id: "nodeAdd", timestamp: at(100) }),
				a: comment("a", { node_id: "nodeAdd", timestamp: at(100) }),
			},
			INDEX,
		);
		expect(model.threads[0].comments.map((entry) => entry.id)).toEqual([
			"a",
			"z",
		]);
	});

	test("image and video comments never enter the editor model", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				img: comment("img", {
					node_id: "nodeAdd",
					comment_type: ICommentType.Image,
				}),
				vid: comment("vid", { comment_type: ICommentType.Video }),
				txt: comment("txt", { node_id: "nodeAdd" }),
			},
			INDEX,
		);
		expect(model.threads).toHaveLength(1);
		expect(model.threads[0].comments.map((entry) => entry.id)).toEqual(["txt"]);
		expect(model.unanchored).toEqual([]);
	});

	test("dangling and absent node_ids collect as unanchored notes, sorted by time", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				ghost: comment("ghost", {
					node_id: "deleted-node",
					timestamp: at(300),
				}),
				note: comment("note", { timestamp: at(10) }),
				empty: comment("empty", { node_id: null, timestamp: at(20) }),
			},
			INDEX,
		);
		expect(model.threads).toEqual([]);
		expect(model.unanchored.map((entry) => entry.id)).toEqual([
			"note",
			"empty",
			"ghost",
		]);
	});

	test("threads follow their anchor when a re-render moves the statement", () => {
		const comments = {
			a: comment("a", { node_id: "nodeAdd" }),
		};
		const before = deriveFlowScriptCommentThreads(comments, INDEX);
		const after = deriveFlowScriptCommentThreads(comments, MOVED_INDEX);
		expect(before.threads[0].line).toBe(3);
		expect(after.threads[0].line).toBe(5);
		expect(after.threads[0].anchorId).toBe("nodeAdd");
	});
});

describe("FlowScript comment indicators", () => {
	const slotFor = (sub: string) => (sub === "user-1" ? 4 : undefined);

	test("one indicator per thread line with count and first-author slot", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				a: comment("a", {
					node_id: "nodeAdd",
					author: "user-1",
					timestamp: at(100),
				}),
				b: comment("b", {
					node_id: "nodeAdd",
					author: "user-2",
					timestamp: at(200),
				}),
				c: comment("c", { node_id: "nodePrint", author: "anonymous" }),
			},
			INDEX,
		);
		const { indicators, key } = deriveFlowScriptCommentIndicators(
			model.threads,
			slotFor,
		);
		expect(indicators).toEqual([
			{
				line: 3,
				anchorId: "nodeAdd",
				count: 2,
				firstAuthor: "user-1",
				slot: 4,
			},
			{
				line: 4,
				anchorId: "nodePrint",
				count: 1,
				firstAuthor: undefined,
				slot: undefined,
			},
		]);
		expect(key).toBe("3@nodeAdd:2#4|4@nodePrint:1#n");
	});

	test("the key moves when a comment joins a thread", () => {
		const one = deriveFlowScriptCommentThreads(
			{ a: comment("a", { node_id: "nodeAdd" }) },
			INDEX,
		);
		const two = deriveFlowScriptCommentThreads(
			{
				a: comment("a", { node_id: "nodeAdd" }),
				b: comment("b", { node_id: "nodeAdd", timestamp: at(200) }),
			},
			INDEX,
		);
		expect(deriveFlowScriptCommentIndicators(one.threads).key).not.toBe(
			deriveFlowScriptCommentIndicators(two.threads).key,
		);
	});

	test("add-affordance lines cover node anchors without a thread only", () => {
		const model = deriveFlowScriptCommentThreads(
			{ a: comment("a", { node_id: "nodeAdd" }) },
			INDEX,
		);
		// layerFn (layer) and varGreeting (variable) are never commentable;
		// nodeAdd already holds a thread — only nodePrint's line remains.
		expect(deriveFlowScriptCommentAddLines(INDEX, model)).toEqual([4]);
	});

	test("every line of a multi-line node loses the affordance once its node has a thread", () => {
		const branchIndex = parseFlowScriptAnchors(
			["if (x) { //@n:branch", "} else { //@n:branch", "}"].join("\n"),
		);
		const empty = deriveFlowScriptCommentThreads({}, branchIndex);
		expect(deriveFlowScriptCommentAddLines(branchIndex, empty)).toEqual([1, 2]);
		const threaded = deriveFlowScriptCommentThreads(
			{ a: comment("a", { node_id: "branch" }) },
			branchIndex,
		);
		expect(deriveFlowScriptCommentAddLines(branchIndex, threaded)).toEqual([]);
	});
});

describe("FlowScript comment create payload", () => {
	test("binds the anchor, offsets from the node and stamps author/time/layer", () => {
		const built = buildFlowScriptComment({
			id: "c1",
			anchorId: "nodeAdd",
			content: "hello",
			author: "user-1",
			node: { coordinates: [100, 200, 5], layer: "layerX" },
			nowMs: 1_700_000_123_456,
		});
		expect(built.id).toBe("c1");
		expect(built.node_id).toBe("nodeAdd");
		expect(built.comment_type).toBe(ICommentType.Text);
		expect(built.content).toBe("hello");
		expect(built.coordinates).toEqual([
			100 + FLOWSCRIPT_COMMENT_OFFSET_X,
			200 + FLOWSCRIPT_COMMENT_OFFSET_Y,
			5,
		]);
		expect(built.layer).toBe("layerX");
		expect(built.author).toBe("user-1");
		expect(built.timestamp).toEqual({
			secs_since_epoch: 1_700_000_123,
			nanos_since_epoch: 456_000_000,
		});
		expect(commentTimestampMs(built)).toBe(1_700_000_123_456);
	});

	test("without node facts it falls back to the canvas comment-less defaults", () => {
		const built = buildFlowScriptComment({
			id: "c2",
			anchorId: "nodeAdd",
			content: "hello",
			author: "",
			nowMs: 1_000,
		});
		expect(built.coordinates).toEqual([0, 0, 0]);
		expect(built.layer).toBeUndefined();
		expect(built.author).toBeUndefined();
		expect(built.width).toBeUndefined();
		expect(built.height).toBeUndefined();
	});

	test("a content edit preserves identity, binding, placement and timestamp", () => {
		const original = comment("c3", {
			node_id: "nodeAdd",
			author: "user-1",
			coordinates: [7, 8, 9],
			timestamp: at(42),
		});
		const edited = withFlowScriptCommentContent(original, "new text");
		expect(edited.content).toBe("new text");
		expect(edited.id).toBe("c3");
		expect(edited.node_id).toBe("nodeAdd");
		expect(edited.coordinates).toEqual([7, 8, 9]);
		expect(edited.timestamp).toEqual(at(42));
		expect(original.content).toBe("content-c3");
	});
});

describe("FlowScript comment permissions", () => {
	test("own, authorless and legacy-anonymous comments are modifiable; others are not", () => {
		const sub = "user-1";
		expect(
			canModifyFlowScriptComment(comment("a", { author: "user-1" }), sub),
		).toBe(true);
		expect(
			canModifyFlowScriptComment(comment("b", { author: "user-2" }), sub),
		).toBe(false);
		expect(canModifyFlowScriptComment(comment("c"), sub)).toBe(true);
		expect(
			canModifyFlowScriptComment(comment("d", { author: null }), sub),
		).toBe(true);
		expect(
			canModifyFlowScriptComment(comment("e", { author: "anonymous" }), sub),
		).toBe(true);
		expect(
			canModifyFlowScriptComment(comment("f", { author: "user-2" }), undefined),
		).toBe(false);
	});
});

describe("FlowScript comment preview", () => {
	const nameFor = (author?: string) => author ?? "User";
	const timeFor = (ms: number) => `t${ms}`;

	test("lists author, time and flattened content per comment", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				a: comment("a", {
					node_id: "nodeAdd",
					author: "user-1",
					content: "line one\nline two",
					timestamp: at(1),
				}),
				b: comment("b", {
					node_id: "nodeAdd",
					author: "anonymous",
					content: "second",
					timestamp: at(2),
				}),
			},
			INDEX,
		);
		expect(
			formatFlowScriptCommentPreview(model.threads[0], nameFor, timeFor),
		).toBe("user-1 · t1000: line one line two\nUser · t2000: second");
	});

	test("truncates long content and folds overflowing comments into a +n line", () => {
		const model = deriveFlowScriptCommentThreads(
			{
				a: comment("a", {
					node_id: "nodeAdd",
					content: "x".repeat(200),
					timestamp: at(1),
				}),
				b: comment("b", { node_id: "nodeAdd", timestamp: at(2) }),
				c: comment("c", { node_id: "nodeAdd", timestamp: at(3) }),
				d: comment("d", { node_id: "nodeAdd", timestamp: at(4) }),
			},
			INDEX,
		);
		const preview = formatFlowScriptCommentPreview(
			model.threads[0],
			nameFor,
			timeFor,
		);
		const lines = preview.split("\n");
		expect(lines).toHaveLength(4);
		expect(lines[0].endsWith("…")).toBe(true);
		expect(lines[0].length).toBeLessThan(200);
		expect(lines[3]).toBe("+1");
	});
});
