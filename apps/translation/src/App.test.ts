import { describe, expect, test } from "bun:test";
import { dirtyAfterSave } from "./lib/save-state";

describe("save bookkeeping", () => {
	test("keeps an entry dirty when it is edited while a save is in flight", async () => {
		let finishSave: (() => void) | undefined;
		const request = new Promise<void>((resolve) => {
			finishSave = resolve;
		});
		const pending = [
			{ entry: "de/common", version: 1, tree: { greeting: "Hallo" } },
		];
		const versions: Record<string, number> = { "de/common": 1 };

		const completion = request.then(() =>
			dirtyAfterSave(new Set(["de/common"]), pending, versions),
		);
		versions["de/common"] = 2;
		finishSave?.();

		expect(await completion).toEqual(new Set(["de/common"]));
	});

	test("clears an unchanged entry after its save completes", () => {
		expect(
			dirtyAfterSave(
				new Set(["de/common"]),
				[{ entry: "de/common", version: 1, tree: {} }],
				{ "de/common": 1 },
			),
		).toEqual(new Set());
	});
});
