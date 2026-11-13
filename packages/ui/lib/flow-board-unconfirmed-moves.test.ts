import { describe, expect, test } from "bun:test";
import {
	type UnconfirmedMove,
	keepUnconfirmedPositions,
} from "./flow-board-helpers";

interface TestNode {
	id: string;
	position: { x: number; y: number };
	dragging?: boolean;
}

const node = (id: string, x: number, y: number, dragging?: boolean) =>
	({ id, position: { x, y }, dragging }) satisfies TestNode;

describe("keepUnconfirmedPositions", () => {
	test("returns the parsed nodes untouched when nothing is unconfirmed", () => {
		const parsed = [node("a", 0, 0), node("b", 10, 10)];
		expect(keepUnconfirmedPositions(parsed, [], new Map())).toBe(parsed);
	});

	test("a node whose move is still saving keeps its local position", () => {
		const parsed = [node("a", 0, 0), node("b", 10, 10)];
		const unconfirmed = new Map<string, UnconfirmedMove>([
			["b", { position: { x: 40, y: 70 }, save: 2 }],
		]);
		const result = keepUnconfirmedPositions(parsed, [], unconfirmed);
		expect(result[0]).toBe(parsed[0]);
		expect(result[1]).toEqual(node("b", 40, 70, undefined));
	});

	test("a dragged node keeps its live position and drag state", () => {
		const parsed = [node("a", 0, 0)];
		const current = [node("a", 25, 5, true)];
		const unconfirmed = new Map<string, UnconfirmedMove>([
			["a", { position: { x: 99, y: 99 }, save: 1 }],
		]);
		expect(keepUnconfirmedPositions(parsed, current, unconfirmed)).toEqual([
			node("a", 25, 5, true),
		]);
	});

	test("a confirmed position keeps the parsed node identity", () => {
		const parsed = [node("a", 40, 70)];
		const unconfirmed = new Map<string, UnconfirmedMove>([
			["a", { position: { x: 40, y: 70 }, save: 3 }],
		]);
		expect(keepUnconfirmedPositions(parsed, [], unconfirmed)[0]).toBe(
			parsed[0],
		);
	});

	test("nodes missing from the parsed layer are ignored", () => {
		const parsed = [node("a", 0, 0)];
		const unconfirmed = new Map<string, UnconfirmedMove>([
			["elsewhere", { position: { x: 5, y: 5 }, save: 1 }],
		]);
		expect(keepUnconfirmedPositions(parsed, [], unconfirmed)).toEqual(parsed);
	});
});
