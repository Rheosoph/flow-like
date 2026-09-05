import { expect, test } from "bun:test";
import { waitForMigration } from "./wait-for-migration";

test("the API waits for the migration Job to complete", async () => {
	let reads = 0;
	let pauses = 0;
	await waitForMigration(async () => {
		reads++;
		if (reads === 1) return null;
		if (reads === 2) return { status: {} };
		return { status: { conditions: [{ type: "Complete", status: "True" }] } };
	}, async () => { pauses++; });
	expect(pauses).toBe(2);
});

test("failed and unfinished migrations never allow the API to start", async () => {
	await expect(waitForMigration(async () => ({ status: { conditions: [{ type: "Failed", status: "True" }] } }), async () => {})).rejects.toThrow("migration failed");
	await expect(waitForMigration(async () => null, async () => {}, 2)).rejects.toThrow("Timed out");
});
