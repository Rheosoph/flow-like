import "dotenv/config";
import { spawn } from "node:child_process";
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

/// Prisma drops a stale `@@unique` with `DROP INDEX`, which CockroachDB refuses
/// while the index backs a unique constraint. Drop the constraint instead and
/// let the push run again.
const CONSTRAINT_IN_USE = /index "([^"]+)" is in use as unique constraint/;
const MAX_PUSH_ATTEMPTS = 5;

function quoteIdentifier(name: string): string {
	return `"${name.replace(/"/g, '""')}"`;
}

function databaseUrlForPush(databaseUrl: string): string {
	const url = new URL(databaseUrl);
	const options = url.searchParams.get("options");
	const disableNewTableLocks = "-c create_table_with_schema_locked=off";
	url.searchParams.set(
		"options",
		options ? `${options} ${disableNewTableLocks}` : disableNewTableLocks,
	);
	return url.toString();
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

async function dropUniqueConstraint(
	client: Client,
	constraint: string,
): Promise<boolean> {
	try {
		const owner = await client.query(
			`SELECT namespace.nspname AS schema_name, class.relname AS table_name
			FROM pg_constraint constraints
			JOIN pg_class class ON class.oid = constraints.conrelid
			JOIN pg_namespace namespace ON namespace.oid = class.relnamespace
			WHERE constraints.conname = $1 AND constraints.contype = 'u'`,
			[constraint],
		);
		const owned = owner.rows[0];
		if (!owned) {
			console.warn(`No unique constraint named ${constraint} found`);
			return false;
		}

		const table = `${quoteIdentifier(owned.schema_name)}.${quoteIdentifier(owned.table_name)}`;
		console.log(`Dropping stale unique constraint ${constraint} on ${table}`);
		await client.query(
			`ALTER TABLE ${table} DROP CONSTRAINT ${quoteIdentifier(constraint)}`,
		);
		return true;
	} catch (error) {
		console.warn(`Failed to drop constraint ${constraint}: ${error}`);
		return false;
	}
}

function runPush(): Promise<{ status: number; output: string }> {
	return new Promise((resolve) => {
		const child = spawn(
			"bunx",
			[
				"prisma",
				"db",
				"push",
				"--schema",
				"prisma/schema",
				...process.argv.slice(2),
			],
			{
				stdio: ["inherit", "pipe", "pipe"],
				env: {
					...process.env,
					// CockroachDB 26.1+ locks newly created tables by default. Prisma may
					// create a table and add its indexes as separate schema changes, so a
					// new lock would make the same push fail midway through.
					DATABASE_URL: databaseUrlForPush(process.env.DATABASE_URL!),
				},
			},
		);

		let output = "";
		child.stdout?.on("data", (chunk) => {
			output += chunk;
			process.stdout.write(chunk);
		});
		child.stderr?.on("data", (chunk) => {
			output += chunk;
			process.stderr.write(chunk);
		});
		child.on("close", (status) => resolve({ status: status ?? 1, output }));
	});
}

const client = await connect();
const locked = client ? await findLockedTables(client) : [];

if (client && locked.length > 0) {
	console.log(`Unlocking schema on: ${locked.join(", ")}`);
	await setSchemaLocked(client, locked, false);
}

let push = await runPush();
for (
	let attempt = 1;
	client && push.status !== 0 && attempt < MAX_PUSH_ATTEMPTS;
	attempt++
) {
	const constraint = CONSTRAINT_IN_USE.exec(push.output)?.[1];
	if (!constraint || !(await dropUniqueConstraint(client, constraint))) break;
	push = await runPush();
}

if (client) {
	if (locked.length > 0) {
		console.log(`Restoring schema lock on: ${locked.join(", ")}`);
		await setSchemaLocked(client, locked, true);
	}
	await client.end();
}

process.exit(push.status);
