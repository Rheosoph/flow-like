import { readFileSync } from "node:fs";

interface Job {
	status?: { conditions?: { type: string; status: string }[] };
}

export async function waitForMigration(
	readJob: () => Promise<Job | null>,
	pause: () => Promise<void>,
	attempts = 120,
): Promise<void> {
	for (let attempt = 0; attempt < attempts; attempt++) {
		const job = await readJob();
		const conditions = job?.status?.conditions ?? [];
		if (conditions.some((item) => item.type === "Failed" && item.status === "True")) {
			throw new Error("Database migration failed; inspect the migration Job logs");
		}
		if (conditions.some((item) => item.type === "Complete" && item.status === "True")) return;
		await pause();
	}
	throw new Error("Timed out waiting for the database migration Job");
}

async function main(): Promise<void> {
	const namespace = process.env.KUBERNETES_NAMESPACE;
	const name = process.env.MIGRATION_JOB_NAME;
	if (!namespace || !name) throw new Error("Migration Job name and namespace are required");
	const serviceAccount = "/var/run/secrets/kubernetes.io/serviceaccount";
	const ca = readFileSync(`${serviceAccount}/ca.crt`, "utf8");
	const url = `https://kubernetes.default.svc/apis/batch/v1/namespaces/${encodeURIComponent(namespace)}/jobs/${encodeURIComponent(name)}`;
	await waitForMigration(async () => {
		const response = await fetch(url, {
			headers: { Authorization: `Bearer ${readFileSync(`${serviceAccount}/token`, "utf8").trim()}` },
			tls: { ca },
			signal: AbortSignal.timeout(5_000),
		});
		if (response.status === 404) return null;
		if (!response.ok) throw new Error(`Reading migration Job failed: HTTP ${response.status}`);
		return response.json() as Promise<Job>;
	}, () => Bun.sleep(5_000));
}

if (import.meta.main) {
	main().catch((error) => {
		console.error(error instanceof Error ? error.message : String(error));
		process.exitCode = 1;
	});
}
