import { describe, expect, test } from "bun:test";
import { type CommunityEdge, detectCommunities } from "./graph-communities";

/** Three cliques joined by one edge each — the textbook separable case. */
function threeCliques(): { nodes: string[]; edges: CommunityEdge[] } {
	const nodes: string[] = [];
	const edges: CommunityEdge[] = [];
	for (let clique = 0; clique < 3; clique += 1) {
		const members = Array.from({ length: 6 }, (_, i) => `c${clique}-n${i}`);
		nodes.push(...members);
		for (let i = 0; i < members.length; i += 1) {
			for (let j = i + 1; j < members.length; j += 1) {
				edges.push({ source: members[i], target: members[j] });
			}
		}
	}
	edges.push({ source: "c0-n0", target: "c1-n0" });
	edges.push({ source: "c1-n1", target: "c2-n1" });
	return { nodes, edges };
}

describe("detectCommunities", () => {
	test("separates cliques joined by a single edge", () => {
		const { nodes, edges } = threeCliques();
		const result = detectCommunities(nodes, edges);

		expect(result.members.length).toBe(3);
		for (let clique = 0; clique < 3; clique += 1) {
			const ids = new Set(
				Array.from({ length: 6 }, (_, i) =>
					result.communityByNode.get(`c${clique}-n${i}`),
				),
			);
			expect(ids.size).toBe(1);
		}
		expect(result.modularity).toBeGreaterThan(0.5);
	});

	test("is deterministic across runs and input order", () => {
		const { nodes, edges } = threeCliques();
		const forward = detectCommunities(nodes, edges);
		const again = detectCommunities(nodes, edges);
		const reversed = detectCommunities(nodes, [...edges].reverse());

		expect([...again.communityByNode]).toEqual([...forward.communityByNode]);
		// Same partition, whatever order the rows arrived in.
		const grouping = (r: typeof forward) =>
			r.members.map((m) => [...m].sort().join(",")).sort();
		expect(grouping(reversed)).toEqual(grouping(forward));
	});

	test("numbers communities largest-first", () => {
		const nodes = ["a", "b", "c", "d", "e"];
		const edges: CommunityEdge[] = [
			{ source: "a", target: "b" },
			{ source: "b", target: "c" },
			{ source: "a", target: "c" },
			{ source: "d", target: "e" },
		];
		const result = detectCommunities(nodes, edges);
		expect(result.members[0].length).toBeGreaterThanOrEqual(
			result.members[result.members.length - 1].length,
		);
		expect(result.communityByNode.get("a")).toBe(0);
	});

	test("puts every node somewhere, isolated ones included", () => {
		const nodes = ["a", "b", "lonely"];
		const result = detectCommunities(nodes, [{ source: "a", target: "b" }]);
		expect(result.communityByNode.size).toBe(3);
		expect(result.communityByNode.get("lonely")).toBeDefined();
	});

	test("survives a graph with no edges at all", () => {
		const result = detectCommunities(["a", "b"], []);
		expect(result.communityByNode.size).toBe(2);
		expect(result.modularity).toBe(0);
	});

	test("ignores edges naming nodes outside the set", () => {
		const result = detectCommunities(
			["a", "b"],
			[
				{ source: "a", target: "b" },
				{ source: "a", target: "ghost" },
			],
		);
		expect(result.communityByNode.size).toBe(2);
	});

	test("a higher resolution splits into more communities", () => {
		const { nodes, edges } = threeCliques();
		const coarse = detectCommunities(nodes, edges, { resolution: 0.4 });
		const fine = detectCommunities(nodes, edges, { resolution: 2.5 });
		expect(fine.members.length).toBeGreaterThanOrEqual(coarse.members.length);
	});

	test("handles self-loops without corrupting the partition", () => {
		const result = detectCommunities(
			["a", "b", "c"],
			[
				{ source: "a", target: "a" },
				{ source: "a", target: "b" },
				{ source: "b", target: "c" },
			],
		);
		expect(result.communityByNode.size).toBe(3);
		expect(Number.isFinite(result.modularity)).toBe(true);
	});
});
