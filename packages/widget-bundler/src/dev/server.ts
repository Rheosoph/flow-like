import { type ChildProcess, spawn } from "node:child_process";
import { type Server, type ServerResponse, createServer } from "node:http";
import { createServer as createNetServer } from "node:net";
import { join, resolve } from "node:path";
import {
	type FrameworkGroup,
	discoverGroupWidgets,
	discoverGroups,
} from "../pack";
import {
	ContractCache,
	type ContractGroupRef,
	handleContractRequest,
} from "./contract-endpoint";
import { type HarnessWidget, harnessHtml } from "./harness-html";

export const DEFAULT_HARNESS_PORT = 4700;

export interface DevServerOptions {
	/** Harness port (child dev servers get the next free ports after it) */
	port?: number;
	quiet?: boolean;
}

export interface DevGroup {
	group: FrameworkGroup;
	widgetIds: string[];
	/** Port assigned by the harness and forwarded to vite (`--port --strictPort`) */
	assignedPort: number;
	/** Port parsed from the dev server's `Local:` line — authoritative once seen */
	reportedPort: number | null;
	child: ChildProcess | null;
}

export interface DevServerHandle {
	port: number;
	url: string;
	groups: DevGroup[];
	server: Server;
	close(): Promise<void>;
}

export function widgetEntryUrl(port: number, widgetId: string): string {
	return `http://localhost:${port}/src/widgets/${widgetId}/index.html`;
}

function isPortFree(port: number): Promise<boolean> {
	return new Promise((done) => {
		const probe = createNetServer();
		probe.once("error", () => done(false));
		probe.listen({ port, host: "127.0.0.1" }, () => {
			probe.close(() => done(true));
		});
	});
}

async function nextFreePort(
	start: number,
	taken: Set<number>,
): Promise<number> {
	let port = start;
	while (taken.has(port) || !(await isPortFree(port))) {
		port += 1;
		if (port > 65535) throw new Error("No free port found for a dev server");
	}
	return port;
}

const ANSI_RE = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*[A-Za-z]`, "g");
const LOCAL_URL_RE = /\bLocal:\s+https?:\/\/[^\s:/]+:(\d+)/;

function lineSplitter(
	onLine: (line: string) => void,
): (chunk: unknown) => void {
	let buffer = "";
	return (chunk) => {
		buffer += String(chunk);
		const lines = buffer.split(/\r?\n/);
		buffer = lines.pop() ?? "";
		for (const line of lines) onLine(line);
	};
}

/**
 * Start one framework group's own dev script.
 *
 * Port mechanism: the harness assigns each group a free port up front and
 * forwards it (`bun run dev -- --port <n> --strictPort`), so widget iframe
 * URLs are deterministic from the start. As a safety net the child's stdout
 * is also scanned for vite's `Local: http://localhost:<port>` line; if the
 * dev script ignored the forwarded flags (or vite still moved), the reported
 * port overrides the assigned one for all URLs the harness serves.
 */
function spawnGroup(
	entry: DevGroup,
	log: (line: string) => void,
): ChildProcess {
	const child = spawn(
		"bun",
		["run", "dev", "--", "--port", String(entry.assignedPort), "--strictPort"],
		{
			cwd: entry.group.dir,
			stdio: ["ignore", "pipe", "pipe"],
			detached: process.platform !== "win32",
			env: { ...process.env, FORCE_COLOR: "0", NO_COLOR: "1" },
		},
	);
	const onLine = (line: string) => {
		const clean = line.replace(ANSI_RE, "");
		const match = LOCAL_URL_RE.exec(clean);
		const port = match?.[1] !== undefined ? Number(match[1]) : Number.NaN;
		if (Number.isInteger(port) && port > 0) entry.reportedPort = port;
		if (clean.trim().length > 0) log(clean);
	};
	child.stdout?.on("data", lineSplitter(onLine));
	child.stderr?.on("data", lineSplitter(onLine));
	child.on("error", (e) => log(`failed to start dev server: ${e.message}`));
	child.on("exit", (code, signal) => {
		log(`dev server exited (${signal ?? `code ${code}`})`);
	});
	return child;
}

function killChild(child: ChildProcess | null): void {
	if (!child || child.exitCode !== null || child.pid === undefined) return;
	if (process.platform === "win32") {
		child.kill();
		return;
	}
	try {
		process.kill(-child.pid, "SIGTERM");
	} catch {
		child.kill("SIGTERM");
	}
}

function widgetList(devGroups: DevGroup[]): HarnessWidget[] {
	const widgets: HarnessWidget[] = [];
	for (const entry of devGroups) {
		const port = entry.reportedPort ?? entry.assignedPort;
		for (const id of entry.widgetIds) {
			widgets.push({
				group: entry.group.name,
				id,
				entryUrl: widgetEntryUrl(port, id),
			});
		}
	}
	return widgets;
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
	res.writeHead(status, { "Content-Type": "application/json; charset=utf-8" });
	res.end(JSON.stringify(body));
}

/**
 * `flow-like-widgets dev`: spawn every framework group's dev script and serve
 * the mock-host harness page (design §8.4 Layer 1) plus its JSON endpoints:
 *
 * - `GET /`                            harness page
 * - `GET /api/widgets`                 live widget list + iframe entry URLs
 * - `GET /api/contract/<group>/<id>`   re-extracted contract (mtime-cached)
 */
export async function startDevServer(
	projectDir: string,
	opts: DevServerOptions = {},
): Promise<DevServerHandle> {
	const project = resolve(projectDir);
	const groups = discoverGroups(project);
	if (groups.length === 0) {
		throw new Error(
			`No framework groups found under ${join(project, "widgets")} (expected widgets/<group>/package.json)`,
		);
	}

	const harnessPort = opts.port ?? DEFAULT_HARNESS_PORT;
	const taken = new Set<number>([harnessPort]);
	const devGroups: DevGroup[] = [];
	let searchFrom = harnessPort + 1;
	for (const group of groups) {
		const widgetIds = discoverGroupWidgets(group.dir);
		if (widgetIds.length === 0) continue;
		const assignedPort = await nextFreePort(searchFrom, taken);
		taken.add(assignedPort);
		searchFrom = assignedPort + 1;
		devGroups.push({
			group,
			widgetIds,
			assignedPort,
			reportedPort: null,
			child: null,
		});
	}
	if (devGroups.length === 0) {
		throw new Error(
			`No widgets found (expected widgets/<group>/src/widgets/<id>/widget.config.ts under ${project})`,
		);
	}

	const log = opts.quiet ? () => {} : (line: string) => console.log(line);
	for (const entry of devGroups) {
		log(
			`[harness] starting '${entry.group.name}' dev server on port ${entry.assignedPort} (${entry.widgetIds.join(", ")})`,
		);
		entry.child = spawnGroup(entry, (line) =>
			log(`[${entry.group.name}] ${line}`),
		);
	}

	const cache = new ContractCache();
	const groupRefs: ContractGroupRef[] = devGroups.map((entry) => ({
		name: entry.group.name,
		dir: entry.group.dir,
	}));
	const refreshWidgetIds = () => {
		for (const entry of devGroups) {
			const ids = discoverGroupWidgets(entry.group.dir);
			if (ids.length > 0) entry.widgetIds = ids;
		}
	};

	const server = createServer((req, res) => {
		const url = new URL(req.url ?? "/", `http://localhost:${harnessPort}`);
		if (req.method !== "GET") {
			sendJson(res, 405, { error: "method not allowed" });
			return;
		}
		if (url.pathname === "/") {
			refreshWidgetIds();
			res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
			res.end(harnessHtml(widgetList(devGroups)));
			return;
		}
		if (url.pathname === "/api/widgets") {
			refreshWidgetIds();
			sendJson(res, 200, { widgets: widgetList(devGroups) });
			return;
		}
		const match = /^\/api\/contract\/([^/]+)\/([^/]+)$/.exec(url.pathname);
		if (match?.[1] !== undefined && match[2] !== undefined) {
			const { status, body } = handleContractRequest(
				cache,
				groupRefs,
				decodeURIComponent(match[1]),
				decodeURIComponent(match[2]),
			);
			sendJson(res, status, body);
			return;
		}
		sendJson(res, 404, { error: "not found" });
	});

	try {
		await new Promise<void>((done, fail) => {
			const onError = (e: Error) =>
				fail(
					new Error(
						`Failed to bind the harness port ${harnessPort}: ${e.message} (pass --port to choose another)`,
					),
				);
			server.once("error", onError);
			server.listen(harnessPort, "127.0.0.1", () => {
				server.off("error", onError);
				done();
			});
		});
	} catch (e) {
		for (const entry of devGroups) killChild(entry.child);
		throw e;
	}

	let closed = false;
	const close = async () => {
		if (closed) return;
		closed = true;
		process.off("SIGINT", onSignal);
		process.off("SIGTERM", onSignal);
		for (const entry of devGroups) killChild(entry.child);
		await new Promise<void>((done) => server.close(() => done()));
	};
	const onSignal = () => {
		void close().then(() => process.exit(0));
	};
	process.once("SIGINT", onSignal);
	process.once("SIGTERM", onSignal);

	return {
		port: harnessPort,
		url: `http://localhost:${harnessPort}/`,
		groups: devGroups,
		server,
		close,
	};
}
