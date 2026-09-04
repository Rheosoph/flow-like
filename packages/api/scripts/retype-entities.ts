/**
 * Post-codegen pass over the generated SeaORM entities.
 *
 * The database stores enum-valued columns as TEXT and string lists as JSONB, so
 * `sea-orm-cli generate entity` types them as `String` / `Json`. This script
 * re-types every field listed in entity-typemap.tsv to the hand-maintained enum or
 * `StringList` newtype (keeping `Option`-ness and the generated `#[sea_orm(...)]`
 * attributes) and fails when a mapped field is missing, so schema drift between the
 * Prisma schema, the typemap and the entity crate is caught at generation time.
 *
 *   bun scripts/retype-entities.ts [--entities src/entity] [--map scripts/entity-typemap.tsv]
 */
import {
	existsSync,
	readFileSync,
	readdirSync,
	unlinkSync,
	writeFileSync,
} from "node:fs";
import { join } from "node:path";

export interface TypemapEntry {
	readonly entity: string;
	readonly field: string;
	readonly type: string;
}

export interface RetypeResult {
	readonly source: string;
	readonly rewritten: string[];
	readonly unchanged: string[];
	readonly missing: string[];
}

const GENERATED_TYPES = ["String", "Json"] as const;

function escapeRegex(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function parseTypemap(text: string): TypemapEntry[] {
	const entries: TypemapEntry[] = [];
	for (const [index, raw] of text.split("\n").entries()) {
		const line = raw.trim();
		if (line.length === 0 || line.startsWith("#")) continue;
		const parts = line.split("\t").map((part) => part.trim());
		if (parts.length !== 3 || parts.some((part) => part.length === 0)) {
			throw new Error(
				`entity-typemap line ${index + 1}: expected "entity<TAB>field<TAB>type", got "${raw}"`,
			);
		}
		entries.push({ entity: parts[0], field: parts[1], type: parts[2] });
	}
	return entries;
}

export function groupByEntity(
	entries: TypemapEntry[],
): Map<string, TypemapEntry[]> {
	const grouped = new Map<string, TypemapEntry[]>();
	for (const entry of entries) {
		const list = grouped.get(entry.entity) ?? [];
		list.push(entry);
		grouped.set(entry.entity, list);
	}
	return grouped;
}

/**
 * Rewrites `pub <field>: String|Json|Option<String|Json>,` to the mapped type. A field
 * already carrying the mapped type counts as unchanged, so the pass is idempotent.
 * Matches both rustfmt output and the single-line token stream sea-orm-cli writes when
 * rustfmt is unavailable (the codegen container has none).
 */
export function retypeSource(
	source: string,
	entries: TypemapEntry[],
): RetypeResult {
	let result = source;
	const rewritten: string[] = [];
	const unchanged: string[] = [];
	const missing: string[] = [];
	for (const entry of entries) {
		const field = escapeRegex(entry.field);
		const generated = new RegExp(
			`(\\bpub\\s+(?:r#)?${field}\\s*:\\s*)(Option\\s*<\\s*)?(${GENERATED_TYPES.join("|")})(\\s*>)?(\\s*,)`,
		);
		const target = escapeRegex(entry.type);
		const already = new RegExp(
			`\\bpub\\s+(?:r#)?${field}\\s*:\\s*(?:Option\\s*<\\s*)?${target}(?:\\s*>)?\\s*,`,
		);
		if (generated.test(result)) {
			result = result.replace(
				generated,
				(_match, prefix: string, open?: string) =>
					open ? `${prefix}Option<${entry.type}>,` : `${prefix}${entry.type},`,
			);
			rewritten.push(entry.field);
		} else if (already.test(result)) {
			unchanged.push(entry.field);
		} else {
			missing.push(entry.field);
		}
	}
	return { source: result, rewritten, unchanged, missing };
}

/** Generated code that still reflects an enum or array column in the database. */
export function driftIn(source: string): string[] {
	const problems: string[] = [];
	if (/\buse\s+super\s*::\s*sea_orm_active_enums\b/.test(source)) {
		problems.push(
			"imports super::sea_orm_active_enums (database still has enum types)",
		);
	}
	if (
		/\bpub\s+(?:r#)?\w+\s*:\s*(?:Option\s*<\s*)?Vec\s*<\s*String\s*>/.test(
			source,
		)
	) {
		problems.push(
			"has a Vec<String> field (database still has an array column)",
		);
	}
	return problems;
}

/**
 * sea-orm-cli never deletes files for tables that no longer exist (or for enums once
 * the database has none), so anything mod.rs does not declare is a leftover.
 */
export function removeStaleEntities(entitiesDir: string): string[] {
	const declared = new Set(
		[
			...readFileSync(join(entitiesDir, "mod.rs"), "utf8").matchAll(
				/\bpub\s+mod\s+(\w+)\s*;/g,
			),
		].map((match) => match[1]),
	);
	const removed: string[] = [];
	for (const name of readdirSync(entitiesDir)) {
		if (!name.endsWith(".rs") || name === "mod.rs") continue;
		const module = name.slice(0, -3);
		if (declared.has(module)) continue;
		unlinkSync(join(entitiesDir, name));
		removed.push(name);
		console.log(`removed stale ${join(entitiesDir, name)}`);
	}
	return removed;
}

function parseArgs(argv: string[]): { entities: string; map: string } {
	const options = { entities: "src/entity", map: "scripts/entity-typemap.tsv" };
	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--entities") options.entities = argv[++i];
		else if (arg === "--map") options.map = argv[++i];
		else throw new Error(`Unknown argument: ${arg}`);
	}
	return options;
}

export function run(entitiesDir: string, mapPath: string): number {
	const entries = parseTypemap(readFileSync(mapPath, "utf8"));
	const failures: string[] = [];
	let rewrittenFields = 0;

	const modPath = join(entitiesDir, "mod.rs");
	if (
		/\bpub\s+mod\s+sea_orm_active_enums\s*;/.test(readFileSync(modPath, "utf8"))
	) {
		failures.push(
			`${modPath}: codegen emitted sea_orm_active_enums — the database still defines enum types; enums live in packages/api/entity/src/sea_orm_active_enums.rs`,
		);
	}
	removeStaleEntities(entitiesDir);

	for (const [entity, fields] of groupByEntity(entries)) {
		const path = join(entitiesDir, `${entity}.rs`);
		if (!existsSync(path)) {
			failures.push(`${path}: entity listed in ${mapPath} was not generated`);
			continue;
		}
		const source = readFileSync(path, "utf8");
		const result = retypeSource(source, fields);
		for (const field of result.missing) {
			failures.push(
				`${path}: field "${field}" not found as a generated String/Json field or as ${fields.find((f) => f.field === field)?.type}`,
			);
		}
		if (result.source !== source) writeFileSync(path, result.source);
		rewrittenFields += result.rewritten.length;
	}

	for (const name of readdirSync(entitiesDir)) {
		if (!name.endsWith(".rs")) continue;
		const path = join(entitiesDir, name);
		for (const problem of driftIn(readFileSync(path, "utf8"))) {
			failures.push(`${path}: ${problem}`);
		}
	}

	if (failures.length > 0) {
		console.error("retype-entities: schema drift detected");
		for (const failure of failures) console.error(`  - ${failure}`);
		return 1;
	}
	console.log(
		`retype-entities: ${rewrittenFields} field(s) rewritten, ${entries.length - rewrittenFields} already typed`,
	);
	return 0;
}

if (import.meta.main) {
	const { entities, map } = parseArgs(process.argv.slice(2));
	process.exit(run(entities, map));
}
