import "dotenv/config";
import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";
import { Client } from "pg";
import { detectDialect, runPrePush } from "./pre-push";

export interface PushClient {
	connect(): Promise<void>;
	end(): Promise<void>;
}

export type CommandRunner = (
	command: string,
	args: string[],
	environment: NodeJS.ProcessEnv,
) => Promise<number>;

const runCommand: CommandRunner = (command, args, environment) =>
	new Promise((resolve, reject) => {
		const child = spawn(command, args, { stdio: "inherit", env: environment });
		child.once("error", reject);
		child.once("close", (status) => resolve(status ?? 1));
	});

/** Inspect the target before any schema changes, then push its PostgreSQL mirror. */
export async function pushPostgresSchema(
	client: PushClient,
	dialect: () => Promise<string>,
	prePush: () => Promise<void>,
	run: CommandRunner,
	environment: NodeJS.ProcessEnv,
	args: string[],
): Promise<number> {
	await client.connect();
	try {
		if ((await dialect()) !== "postgresql") {
			throw new Error(
				"Refusing to push: CockroachDB is a frozen legacy source. Point DATABASE_URL at PostgreSQL, or use the DSQL migration job.",
			);
		}
		const mirrored = await run(
			"bash",
			["scripts/make-prisma-mirror.sh", "--target", "postgresql"],
			environment,
		);
		if (mirrored !== 0) return mirrored;
		await prePush();
		return await run(
			"bunx",
			["prisma", "db", "push", "--schema", "prisma-postgres-mirror/schema", ...args],
			environment,
		);
	} finally {
		await client.end();
	}
}

async function main(): Promise<number> {
	if (!process.env.DATABASE_URL) throw new Error("DATABASE_URL is not set");
	const client = new Client({ connectionString: process.env.DATABASE_URL });
	return pushPostgresSchema(
		client,
		() => detectDialect(client),
		() => runPrePush(client),
		runCommand,
		process.env,
		process.argv.slice(2),
	);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
	main().then(
		(status) => { process.exitCode = status; },
		(error) => {
			console.error(error instanceof Error ? error.message : String(error));
			process.exitCode = 1;
		},
	);
}
