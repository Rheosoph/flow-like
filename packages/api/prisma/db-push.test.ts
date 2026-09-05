import { expect, test } from "bun:test";
import { pushPostgresSchema, type CommandRunner } from "./db-push";

test("PostgreSQL push builds its mirror and preserves connection options", async () => {
	const steps: string[] = [];
	const env = { DATABASE_URL: "postgresql://audit:example@localhost:5432/audit?options=-c%20statement_timeout%3D5000" };
	const run: CommandRunner = async (command, args, environment) => {
		expect(environment.DATABASE_URL).toBe(env.DATABASE_URL);
		expect(environment.DATABASE_URL).not.toContain("create_table_with_schema_locked");
		steps.push([command, ...args].join(" "));
		return 0;
	};
	expect(await pushPostgresSchema(
		{ connect: async () => { steps.push("connect"); }, end: async () => { steps.push("end"); } },
		async () => "postgresql",
		async () => { steps.push("pre-push"); }, run, env, ["--accept-data-loss"],
	)).toBe(0);
	expect(steps).toEqual([
		"connect", "bash scripts/make-prisma-mirror.sh --target postgresql", "pre-push",
		"bunx prisma db push --schema prisma-postgres-mirror/schema --accept-data-loss", "end",
	]);
});

test("legacy CockroachDB is rejected before any schema mutation", async () => {
	let closed = false;
	const unexpected = async () => { throw new Error("schema mutation was attempted"); };
	await expect(pushPostgresSchema(
		{ connect: async () => {}, end: async () => { closed = true; } },
		async () => "cockroachdb", unexpected, unexpected, {}, [],
	)).rejects.toThrow("frozen legacy source");
	expect(closed).toBe(true);
});

test("a failed mirror leaves the database untouched", async () => {
	let prePush = false;
	expect(await pushPostgresSchema(
		{ connect: async () => {}, end: async () => {} },
		async () => "postgresql", async () => { prePush = true; }, async () => 7, {}, [],
	)).toBe(7);
	expect(prePush).toBe(false);
});
