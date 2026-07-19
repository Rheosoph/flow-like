import "dotenv/config";
import { spawnSync } from "node:child_process";
import { Client } from "pg";

/**
 * CockroachDB creates tables with `schema_locked = true` when the cluster
 * setting `create_table_with_schema_locked` is on. Prisma cannot alter a locked
 * table, so `prisma db push` fails on any table added since that default landed.
 * Unlock the affected tables, push, then restore the original lock state.
 */

const LOCKED_TABLES_QUERY = `
	SET allow_unsafe_internals = true;
	SELECT descriptor_name
	FROM crdb_internal.create_statements
	WHERE database_name = current_database()
		AND schema_name = 'public'
		AND descriptor_type = 'table'
		AND create_statement ILIKE '%schema_locked = true%'
	ORDER BY descriptor_name;
`;

function quoteIdentifier(name: string): string {
	return `"${name.replace(/"/g, '""')}"`;
}

async function connect(): Promise<Client | null> {
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
		console.warn(`Could not connect to inspect schema locks: ${error}`);
		return null;
	}
}

async function findLockedTables(client: Client): Promise<string[]> {
	try {
		const results = await client.query(LOCKED_TABLES_QUERY);
		const rows = Array.isArray(results) ? results.at(-1)?.rows : results.rows;
		return (rows ?? []).map(
			(row: { descriptor_name: string }) => row.descriptor_name,
		);
	} catch {
		// Not CockroachDB (e.g. plain Postgres) — nothing to unlock.
		return [];
	}
}

async function setSchemaLocked(
	client: Client,
	tables: string[],
	locked: boolean,
): Promise<void> {
	for (const table of tables) {
		try {
			await client.query(
				`ALTER TABLE ${quoteIdentifier(table)} SET (schema_locked = ${locked})`,
			);
		} catch (error) {
			console.warn(
				`Failed to set schema_locked=${locked} on ${table}: ${error}`,
			);
		}
	}
}

const client = await connect();
const locked = client ? await findLockedTables(client) : [];

if (client && locked.length > 0) {
	console.log(`Unlocking schema on: ${locked.join(", ")}`);
	await setSchemaLocked(client, locked, false);
}

const push = spawnSync(
	"bunx",
	[
		"prisma",
		"db",
		"push",
		"--schema",
		"prisma/schema",
		...process.argv.slice(2),
	],
	{ stdio: "inherit" },
);

if (client) {
	if (locked.length > 0) {
		console.log(`Restoring schema lock on: ${locked.join(", ")}`);
		await setSchemaLocked(client, locked, true);
	}
	await client.end();
}

process.exit(push.status ?? 1);
