import "dotenv/config";
import { Client } from "pg";

/**
 * Backfill for the `EventSetup` table: insert one `stable` row for every
 * Event whose `lastSetupVersion` is set, copying the legacy setup pointer
 * columns. `EventSetup` is the per-variant serving pointer for the inbound
 * REST/MCP surface; `event.lastSetupVersion` keeps being written and stays
 * the fallback, so deployments serve correctly before AND after this runs.
 *
 * Idempotent — the unique constraint on (eventId, variant) skips events
 * that already have a stable row. Run manually after `db:push` created the
 * table.
 */

const SKIPPED_QUERY = `
	SELECT count(*)::int AS count
	FROM "Event"
	WHERE "lastSetupVersion" IS NOT NULL AND "boardId" IS NULL;
`;

const BACKFILL_QUERY = `
	INSERT INTO "EventSetup"
		(id, "appId", "eventId", variant, "eventVersion", "boardId",
		 "boardVersion", "setupStatus", "lastSetupAt", "lastSetupError",
		 "createdAt", "updatedAt")
	SELECT
		gen_random_uuid()::text,
		"appId",
		id,
		'stable',
		"lastSetupVersion",
		"boardId",
		"boardVersion",
		"setupStatus",
		"lastSetupAt",
		"lastSetupError",
		now(),
		now()
	FROM "Event"
	WHERE "lastSetupVersion" IS NOT NULL AND "boardId" IS NOT NULL
	ON CONFLICT ("eventId", variant) DO NOTHING;
`;

async function connect(): Promise<Client> {
	const url = process.env.DATABASE_URL;
	if (!url) {
		console.error("DATABASE_URL is not set");
		process.exit(1);
	}

	const client = new Client({ connectionString: url });
	try {
		await client.connect();
		return client;
	} catch (error) {
		console.error(`Could not connect to the database: ${error}`);
		process.exit(1);
	}
}

const client = await connect();
try {
	const skipped = await client.query(SKIPPED_QUERY);
	const skippedCount = skipped.rows[0]?.count ?? 0;
	if (skippedCount > 0) {
		console.warn(
			`Skipping ${skippedCount} event(s) with a lastSetupVersion but no boardId — nothing to serve`,
		);
	}

	const result = await client.query(BACKFILL_QUERY);
	console.log(`Inserted ${result.rowCount ?? 0} stable EventSetup row(s)`);
} finally {
	await client.end();
}
