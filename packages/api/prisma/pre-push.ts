import "dotenv/config";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { Client } from "pg";

/**
 * Standalone runner: `bun prisma/pre-push.ts` (DATABASE_URL) — for deployments that call
 * `prisma db push` directly instead of through db-push.ts.
 *
 * SQL that has to run before `prisma db push` on an existing database: type changes
 * Prisma emits without the `USING` clause they need (enum -> TEXT, array -> JSONB,
 * timestamp -> TIMESTAMPTZ).
 * Files under prisma/pre-push hold one statement per line; `-- @` comment lines
 * directly above a statement guard it (see the header of 834-enums-arrays.sql).
 * Column-type statements are skipped once the column already has the target type
 * and every statement runs in its own autocommit (CockroachDB refuses column type
 * changes inside explicit transactions), so a re-run is a no-op.
 */
const PRE_PUSH_DIR = "prisma/pre-push";
const COLUMN_TYPE_STATEMENT =
	/^ALTER TABLE "([^"]+)" ALTER COLUMN "([^"]+)" TYPE (TEXT|JSONB|TIMESTAMPTZ)\b/i;
/** information_schema.columns.data_type for the SQL types those statements target. */
const TARGET_DATA_TYPE: Record<string, string> = {
	text: "text",
	jsonb: "jsonb",
	timestamptz: "timestamp with time zone",
};
const GUARD_LINE = /^-- @(dialect|if-type|unless-type) (.+)$/;
const COLUMN_REF = /^"([^"]+)"\."([^"]+)"\s+(\S+)$/;

type Dialect = "postgresql" | "cockroachdb";

interface PrePushStatement {
	readonly file: string;
	readonly sql: string;
	readonly dialect?: Dialect;
	readonly ifType?: { column: string; type: string };
	readonly unlessType?: { column: string; type: string };
}

function parseColumnGuard(
	file: string,
	line: string,
	spec: string,
): { column: string; type: string } {
	const ref = COLUMN_REF.exec(spec.trim());
	if (!ref) {
		throw new Error(
			`${file}: expected '"Table"."column" <type>' after ${line.split(/\s+/)[1]}, got "${spec}"`,
		);
	}
	return { column: `${ref[1]}.${ref[2]}`, type: ref[3].toLowerCase() };
}

export function parsePrePushFile(
	file: string,
	text: string,
): PrePushStatement[] {
	const statements: PrePushStatement[] = [];
	let guards: Partial<PrePushStatement> = {};
	for (const raw of text.split("\n")) {
		const line = raw.trim();
		if (line.length === 0) continue;
		const guard = GUARD_LINE.exec(line);
		if (guard) {
			const [, kind, spec] = guard;
			if (kind === "dialect") {
				if (spec.trim() !== "cockroachdb" && spec.trim() !== "postgresql") {
					throw new Error(`${file}: unknown dialect "${spec}"`);
				}
				guards.dialect = spec.trim() as Dialect;
			} else if (kind === "if-type") {
				guards.ifType = parseColumnGuard(file, line, spec);
			} else {
				guards.unlessType = parseColumnGuard(file, line, spec);
			}
			continue;
		}
		if (line.startsWith("--")) continue;
		statements.push({ file, sql: line.replace(/;$/, ""), ...guards });
		guards = {};
	}
	return statements;
}

function prePushStatements(): PrePushStatement[] {
	let files: string[];
	try {
		files = readdirSync(PRE_PUSH_DIR)
			.filter((name) => name.endsWith(".sql"))
			.sort();
	} catch {
		return [];
	}
	return files.flatMap((file) =>
		parsePrePushFile(file, readFileSync(join(PRE_PUSH_DIR, file), "utf8")),
	);
}

async function columnTypes(client: Client): Promise<Map<string, string>> {
	const result = await client.query(
		`SELECT table_name, column_name, data_type
		FROM information_schema.columns
		WHERE table_schema = 'public'`,
	);
	return new Map(
		result.rows.map(
			(row: { table_name: string; column_name: string; data_type: string }) => [
				`${row.table_name}.${row.column_name}`,
				row.data_type.toLowerCase(),
			],
		),
	);
}

export async function detectDialect(client: Client): Promise<Dialect> {
	try {
		const result = await client.query("SELECT version() AS version");
		return /cockroach/i.test(result.rows[0]?.version ?? "")
			? "cockroachdb"
			: "postgresql";
	} catch {
		return "postgresql";
	}
}

function shouldRun(
	statement: PrePushStatement,
	dialect: Dialect,
	types: Map<string, string>,
): boolean {
	if (statement.dialect && statement.dialect !== dialect) return false;
	if (
		statement.ifType &&
		types.get(statement.ifType.column) !== statement.ifType.type
	) {
		return false;
	}
	if (
		statement.unlessType &&
		types.get(statement.unlessType.column) === statement.unlessType.type
	) {
		return false;
	}
	const column = COLUMN_TYPE_STATEMENT.exec(statement.sql);
	if (column) {
		const current = types.get(`${column[1]}.${column[2]}`);
		if (
			current === undefined ||
			current === TARGET_DATA_TYPE[column[3].toLowerCase()]
		)
			return false;
	}
	return true;
}

export async function runPrePush(client: Client): Promise<void> {
	const statements = prePushStatements();
	if (statements.length === 0) return;

	const dialect = await detectDialect(client);
	if (dialect === "cockroachdb") {
		// Older CockroachDB releases gate rewriting column type changes behind this
		// session setting; newer ones no longer know it.
		await client
			.query("SET enable_experimental_alter_column_type_general = true")
			.catch(() => undefined);
	}

	let types = await columnTypes(client);
	let applied = 0;
	for (const statement of statements) {
		if (!shouldRun(statement, dialect, types)) continue;
		try {
			await client.query(statement.sql);
		} catch (error) {
			console.error(`pre-push (${statement.file}) failed: ${statement.sql}`);
			throw error;
		}
		applied++;
		types = await columnTypes(client);
	}
	console.log(
		`pre-push (${dialect}): applied ${applied} of ${statements.length} statement(s)`,
	);
}

if (import.meta.main) {
	const url = process.env.DATABASE_URL;
	if (!url) {
		console.error("DATABASE_URL is not set");
		process.exit(1);
	}
	const client = new Client({ connectionString: url });
	await client.connect();
	try {
		await runPrePush(client);
	} finally {
		await client.end();
	}
}
