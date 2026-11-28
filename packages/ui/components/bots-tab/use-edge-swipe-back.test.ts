import { expect, test } from "bun:test";
import { lockSwipe, swipeCommits } from "./use-edge-swipe-back";

test("commits past 35% of the width", () => {
	expect(swipeCommits(140, 390, 0)).toBe(true);
	expect(swipeCommits(130, 390, 0)).toBe(false);
});

test("commits on a fast flick but not on a tiny one", () => {
	expect(swipeCommits(60, 390, 0.8)).toBe(true);
	expect(swipeCommits(10, 390, 2)).toBe(false);
	expect(swipeCommits(60, 390, 0.2)).toBe(false);
});

test("never commits leftwards or without width", () => {
	expect(swipeCommits(-50, 390, 2)).toBe(false);
	expect(swipeCommits(50, 0, 2)).toBe(false);
});

test("locks horizontally only for a mostly rightward move", () => {
	expect(lockSwipe(3, 2)).toBeUndefined();
	expect(lockSwipe(12, 3)).toBe("horizontal");
	expect(lockSwipe(10, 14)).toBe("abort");
	expect(lockSwipe(-12, 1)).toBe("abort");
});
