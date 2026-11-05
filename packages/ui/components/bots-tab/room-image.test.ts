import { describe, expect, test } from "bun:test";
import { centeredSquare } from "./room-image";

describe("centeredSquare", () => {
	test("crops a wide image to its middle and scales it down", () => {
		expect(centeredSquare(1200, 600, 256)).toEqual({
			sx: 300,
			sy: 0,
			side: 600,
			edge: 256,
		});
	});

	test("crops a tall image to its middle", () => {
		expect(centeredSquare(300, 900, 256)).toEqual({
			sx: 0,
			sy: 300,
			side: 300,
			edge: 256,
		});
	});

	test("never scales a small image up", () => {
		expect(centeredSquare(64, 80, 256)).toEqual({
			sx: 0,
			sy: 8,
			side: 64,
			edge: 64,
		});
	});
});
