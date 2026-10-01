import { readFileSync, readdirSync } from "node:fs";
import { expect, test } from "vitest";

const fixtureDirectory = new URL("../fixtures/", import.meta.url);
const fixtureNames = readdirSync(fixtureDirectory).filter((name) =>
	name.endsWith(".tauri.json"),
);

function checkPinRows(value: unknown, path: string): void {
	if (!value || typeof value !== "object") return;
	if (Array.isArray(value)) {
		value.forEach((entry, index) => checkPinRows(entry, `${path}[${index}]`));
		return;
	}

	const record = value as Record<string, unknown>;
	if (record.pins && typeof record.pins === "object") {
		const pins = Object.values(record.pins) as {
			pin_type: string;
			index: number;
		}[];
		for (const direction of ["Input", "Output"]) {
			const indices = pins
				.filter((pin) => pin.pin_type === direction)
				.map((pin) => pin.index)
				.sort((a, b) => a - b);
			// The backend numbers each direction from 1. Index 0 places a
			// handle in the node header when the editor renders the fixture.
			expect(indices, `${path}.pins (${direction})`).toEqual(
				Array.from({ length: indices.length }, (_, index) => index + 1),
			);
		}
	}
	for (const [key, entry] of Object.entries(record)) {
		checkPinRows(entry, `${path}.${key}`);
	}
}

test.each(fixtureNames)("%s uses the backend's pin rows", (name) => {
	const fixture = JSON.parse(
		readFileSync(new URL(name, fixtureDirectory), "utf8"),
	);
	checkPinRows(fixture, name);
});
