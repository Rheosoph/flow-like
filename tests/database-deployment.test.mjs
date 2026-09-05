import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseAllDocuments } from "yaml";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function render(settings) {
	const args = ["template", "db-audit", join(repo, "apps/backend/kubernetes/helm")];
	for (const [key, value] of Object.entries({
		"jwt.existingSecret": "audit-jwt", "storage.provider": "s3",
		"storage.s3.existingSecret": "audit-storage", ...settings,
	})) args.push("--set", `${key}=${value}`);
	const result = spawnSync("helm", args, { encoding: "utf8" });
	assert.equal(result.status, 0, result.stderr);
	return parseAllDocuments(result.stdout).map((doc) => doc.toJSON()).filter(Boolean);
}

for (const mode of ["internal", "external-url", "external-secret", "external-cockroach"]) {
	test(`${mode} creates a migration Job that gates API startup`, () => {
		const external = mode !== "internal";
		const docs = render({
			"database.type": external ? "external" : "internal",
			...(mode === "external-secret"
				? { "database.external.existingSecret": "audit-db" }
				: external ? { "database.external.connectionString": "postgresql://audit:example@localhost:5432/audit" } : {}),
			...(mode === "external-cockroach" ? { "database.external.provider": "cockroachdb" } : {}),
			"database.pool.maxConnections": "3", "database.pool.minConnections": "0",
		});
		const job = docs.find((doc) => doc.kind === "Job" && doc.metadata.labels["app.kubernetes.io/component"] === "db-migration");
		const api = docs.find((doc) => doc.kind === "Deployment" && doc.metadata.name.endsWith("-api"));
		assert.equal(job.metadata.annotations?.["helm.sh/hook"], undefined);
		assert.equal(job.spec.ttlSecondsAfterFinished, undefined);
		assert.equal(api.spec.template.spec.initContainers[0].env.find((item) => item.name === "MIGRATION_JOB_NAME").value, job.metadata.name);
		const env = job.spec.template.spec.containers[0].env;
		assert.equal(env.find((item) => item.name === "DATABASE_PROVIDER").value, external && mode !== "external-cockroach" ? "postgresql" : "cockroachdb");
		const secret = env.find((item) => item.name === "DATABASE_URL").valueFrom.secretKeyRef.name;
		if (mode === "external-secret") assert.equal(secret, "audit-db");
		else assert.ok(docs.some((doc) => doc.kind === "Secret" && doc.metadata.name === secret));
		assert.equal(api.spec.template.spec.containers[0].env.find((item) => item.name === "DATABASE_POOL_MIN_CONNECTIONS").value, "0");
	});
}

test("disabling migrations removes both the Job and startup waiter", () => {
	const docs = render({ "database.migration.enabled": "false" });
	assert.ok(!docs.some((doc) => doc.kind === "Job" && doc.metadata.labels["app.kubernetes.io/component"] === "db-migration"));
	const api = docs.find((doc) => doc.kind === "Deployment" && doc.metadata.name.endsWith("-api"));
	assert.equal(api.spec.template.spec.initContainers, undefined);
});

test("an existing API service account receives permission to read migration Jobs", () => {
	const docs = render({ "serviceAccount.create": "false", "serviceAccount.name": "existing-api" });
	assert.ok(!docs.some((doc) => doc.kind === "ServiceAccount" && doc.metadata.name === "existing-api"));
	const api = docs.find((doc) => doc.kind === "Deployment" && doc.metadata.name.endsWith("-api"));
	assert.equal(api.spec.template.spec.serviceAccountName, "existing-api");
	const binding = docs.find((doc) => doc.kind === "RoleBinding" && doc.metadata.name.endsWith("-job-manager"));
	assert.equal(binding.subjects[0].name, "existing-api");
	const role = docs.find((doc) => doc.kind === "Role" && doc.metadata.name === binding.roleRef.name);
	assert.ok(role.rules.some((rule) => rule.apiGroups.includes("batch") && rule.resources.includes("jobs") && rule.verbs.includes("get")));
});

for (const provider of ["postgresql", "cockroachdb"]) {
	test(`the migration entrypoint selects the ${provider} schema`, () => {
		const stage = mkdtempSync(join(tmpdir(), "flowlike-migration-"));
		try {
			for (const command of ["pg_isready", "psql", "bun", "bunx", "make-prisma-mirror.sh"]) {
				const script = ["pg_isready", "psql"].includes(command) ? "exit 0" : `printf '%s\\n' "${command} $*" >> "$TRACE_FILE"`;
				const path = join(stage, command);
				writeFileSync(path, `#!/bin/bash\n${script}\n`);
				chmodSync(path, 0o755);
			}
			const trace = join(stage, "trace");
			const result = spawnSync("bash", [join(repo, "apps/backend/kubernetes/migration/run-migration.sh")], {
				cwd: stage,
				env: { PATH: `${stage}:${process.env.PATH}`, TRACE_FILE: trace,
					DATABASE_URL: "postgresql://audit:example_secret@localhost:5432/audit", DATABASE_PROVIDER: provider },
				encoding: "utf8",
			});
			assert.equal(result.status, 0, result.stderr);
			assert.ok(!(result.stdout + result.stderr).includes("example_secret"));
			assert.deepEqual(readFileSync(trace, "utf8").trim().split("\n"), provider === "postgresql"
				? ["make-prisma-mirror.sh --target postgresql", "bun prisma/pre-push.ts", "bunx prisma db push --schema=prisma-postgres-mirror/schema --accept-data-loss"]
				: ["bun prisma/pre-push.ts", "bunx prisma db push --schema=prisma/schema --accept-data-loss"]);
		} finally { rmSync(stage, { recursive: true, force: true }); }
	});
}

test("the database URL export wrapper does not print credentials", () => {
	const result = spawnSync("bash", [join(repo, "packages/api/scripts/export-database-url.sh"), "true"], {
		env: { PATH: process.env.PATH, POSTGRES_PASSWORD: "example_secret" }, encoding: "utf8",
	});
	assert.equal(result.status, 0, result.stderr);
	assert.ok(!(result.stdout + result.stderr).includes("example_secret"));
});
