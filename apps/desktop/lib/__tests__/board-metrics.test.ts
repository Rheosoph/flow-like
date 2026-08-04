import {
	connectionCount,
	entryPointNodes,
	flaggedPatterns,
	layerCounts,
	minScores,
	scoreCoverage,
	totalBoardNodeCount,
	variableCounts,
	wasmPackages,
	worstDimension,
	worstScore,
} from "@flow-like/flow-like-ui/lib/board-metrics";
import type {
	IBoard,
	INode,
	INodeScores,
} from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { describe, expect, it } from "vitest";

function scores(overrides: Partial<INodeScores> = {}): INodeScores {
	return {
		security: 10,
		privacy: 10,
		governance: 10,
		performance: 10,
		reliability: 10,
		cost: 10,
		...overrides,
	};
}

function node(id: string, overrides: Partial<INode> = {}): INode {
	return {
		category: "test",
		description: "",
		friendly_name: id,
		id,
		name: id,
		pins: {},
		...overrides,
	} as INode;
}

function board(overrides: Partial<IBoard> = {}): IBoard {
	return {
		comments: {},
		created_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		description: "",
		id: "board",
		layers: {},
		log_level: "Debug",
		execution_mode: "Hybrid",
		name: "Board",
		nodes: {},
		refs: {},
		stage: "Dev",
		updated_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		variables: {},
		version: [1, 0, 0],
		viewport: [0, 0, 1],
		page_ids: [],
		...overrides,
	} as IBoard;
}

describe("minScores", () => {
	it("is undefined when no node declares a score, so callers never print 10/10", () => {
		expect(minScores(board({ nodes: { a: node("a") } }))).toBeUndefined();
	});

	it("takes the minimum per category, not an average", () => {
		const result = minScores(
			board({
				nodes: {
					a: node("a", { scores: scores({ security: 2 }) }),
					b: node("b", { scores: scores({ security: 9, cost: 4 }) }),
				},
			}),
		);
		expect(result?.security).toBe(2);
		expect(result?.cost).toBe(4);
		expect(result?.privacy).toBe(10);
	});

	it("skips reroute nodes the way the server does", () => {
		const result = minScores(
			board({
				nodes: {
					a: node("a", { scores: scores({ security: 8 }) }),
					r: node("reroute", { name: "reroute", scores: scores({ security: 1 }) }),
				},
			}),
		);
		expect(result?.security).toBe(8);
	});
});

describe("worstDimension", () => {
	it("names the lowest dimension and breaks ties in category order", () => {
		const board_ = board({
			nodes: { a: node("a", { scores: scores({ security: 3, privacy: 3 }) }) },
		});
		const result = minScores(board_);
		expect(worstDimension(result)).toBe("security");
		expect(worstScore(result)).toBe(3);
	});

	it("is undefined for an unscored board", () => {
		expect(worstDimension(undefined)).toBeUndefined();
		expect(worstScore(undefined)).toBeUndefined();
	});
});

describe("scoreCoverage", () => {
	it("counts scored nodes against non-reroute nodes", () => {
		const coverage = scoreCoverage(
			board({
				nodes: {
					a: node("a", { scores: scores() }),
					b: node("b"),
					r: node("reroute", { name: "reroute" }),
				},
			}),
		);
		expect(coverage).toEqual({ nodeCount: 2, scoredNodeCount: 1, ratio: 0.5 });
	});
});

describe("flaggedPatterns", () => {
	it("groups by node type and category, keeping the lowest score and a count", () => {
		const patterns = flaggedPatterns(
			board({
				nodes: {
					a: node("a", {
						name: "http_request",
						friendly_name: "HTTP Request",
						scores: scores({ security: 3 }),
					}),
					b: node("b", {
						name: "http_request",
						friendly_name: "HTTP Request",
						scores: scores({ security: 2 }),
					}),
					c: node("c", { name: "safe", scores: scores() }),
				},
			}),
		);
		expect(patterns).toHaveLength(1);
		expect(patterns[0]).toMatchObject({
			node: "http_request",
			friendlyName: "HTTP Request",
			category: "security",
			score: 2,
			count: 2,
		});
	});

	it("ignores anything at or above the flag threshold", () => {
		expect(
			flaggedPatterns(
				board({ nodes: { a: node("a", { scores: scores({ cost: 4 }) }) } }),
			),
		).toHaveLength(0);
	});
});

describe("entryPointNodes", () => {
	it("returns only nodes marked as a start", () => {
		const entries = entryPointNodes(
			board({
				nodes: {
					a: node("a", { start: true }),
					b: node("b"),
					c: node("c", { start: false }),
				},
			}),
		);
		expect(entries.map((entry) => entry.id)).toEqual(["a"]);
	});
});

describe("connectionCount", () => {
	it("halves the pin references because both ends list the edge", () => {
		expect(
			connectionCount(
				board({
					nodes: {
						a: node("a", { pins: { out: { connected_to: ["p1"] } } as never }),
						b: node("b", { pins: { in: { connected_to: ["p0"] } } as never }),
					},
				}),
			),
		).toBe(1);
	});
});

describe("totalBoardNodeCount", () => {
	it("unions layer nodes without double counting", () => {
		expect(
			totalBoardNodeCount(
				board({
					nodes: { a: node("a"), b: node("b") },
					layers: {
						l: { id: "l", nodes: { b: node("b"), c: node("c") } } as never,
					},
				}),
			),
		).toBe(3);
	});
});

describe("wasmPackages", () => {
	it("collects packages and permissions from layers as well as the root", () => {
		const usage = wasmPackages(
			board({
				nodes: {
					a: node("a", {
						wasm: { package_id: "pdf-tools", permissions: ["storage:read"] },
					} as never),
				},
				layers: {
					l: {
						id: "l",
						nodes: {
							b: node("b", {
								wasm: {
									package_id: "html-extract",
									permissions: ["network:http"],
								},
							} as never),
						},
					} as never,
				},
			}),
		);
		expect(usage.packageIds).toEqual(["html-extract", "pdf-tools"]);
		expect(usage.permissions).toEqual(["network:http", "storage:read"]);
	});
});

describe("variableCounts", () => {
	it("counts secrets and anything a run will prompt for", () => {
		expect(
			variableCounts(
				board({
					variables: {
						a: { id: "a", secret: true } as never,
						b: { id: "b", secret: false, runtime_configured: true } as never,
						c: { id: "c", secret: false } as never,
					},
				}),
			),
		).toEqual({ total: 3, secret: 1, promptedAtRuntime: 2 });
	});
});

describe("layerCounts", () => {
	it("counts layers by type", () => {
		const counts = layerCounts(
			board({
				layers: {
					a: { id: "a", type: "Function" } as never,
					b: { id: "b", type: "Function" } as never,
					c: { id: "c", type: "Macro" } as never,
				},
			}),
		);
		expect(counts.total).toBe(3);
		expect(counts.Function).toBe(2);
		expect(counts.Macro).toBe(1);
	});
});
