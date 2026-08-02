#!/usr/bin/env bun
import { statSync } from "node:fs";
import { parseArgs } from "node:util";
import { addWidget } from "./add";
import { DEFAULT_HARNESS_PORT, startDevServer } from "./dev/server";
import { pack } from "./pack";
import { validateBundle, validateProject } from "./validate-cmd";

const USAGE = `flow-like-widgets — Flow-Like widget bundler

Usage:
  flow-like-widgets pack --project <dir> --out <file> [--serving-prefix <url>] [--connect <host> ...] [--created-at <iso>]
  flow-like-widgets dev [--project <dir>] [--port <n>]
  flow-like-widgets validate <project-dir | file.flwb>
  flow-like-widgets add <widget-id> [--group <dir>]

Commands:
  pack       Build widgets.flwb from a package project (framework groups must be built first)
  dev        Start every framework group's dev server + the mock-host harness (default port ${DEFAULT_HARNESS_PORT})
  validate   Validate a project's widget contracts, or a built .flwb bundle
  add        Scaffold src/widgets/<widget-id>/ inside a framework group
`;

function fail(message: string): never {
	console.error(`error: ${message}`);
	process.exit(1);
}

async function runPack(args: string[]): Promise<void> {
	const { values } = parseArgs({
		args,
		options: {
			project: { type: "string", default: "." },
			out: { type: "string" },
			"serving-prefix": { type: "string" },
			connect: { type: "string", multiple: true },
			"created-at": { type: "string" },
		},
	});
	const result = await pack(values.project, {
		out: values.out,
		servingPrefix: values["serving-prefix"] ?? null,
		connectHosts: values.connect ?? [],
		createdAt: values["created-at"],
	});
	for (const warning of result.warnings) {
		console.warn(`warning: ${warning}`);
	}
}

async function runDev(args: string[]): Promise<void> {
	const { values } = parseArgs({
		args,
		options: {
			project: { type: "string", default: "." },
			port: { type: "string" },
		},
	});
	let port: number | undefined;
	if (values.port !== undefined) {
		port = Number(values.port);
		if (!Number.isInteger(port) || port < 1 || port > 65535) {
			fail(`invalid --port '${values.port}' (expected 1-65535)`);
		}
	}
	const handle = await startDevServer(values.project, { port });
	console.log(`\n[harness] mock host ready: ${handle.url}\n`);
}

function runValidate(args: string[]): void {
	const { positionals } = parseArgs({
		args,
		allowPositionals: true,
		options: {},
	});
	const target = positionals[0] ?? ".";
	const isBundle = (() => {
		try {
			return statSync(target).isFile();
		} catch {
			return target.endsWith(".flwb");
		}
	})();

	if (isBundle) {
		const result = validateBundle(target);
		for (const error of result.errors) console.error(`error: ${error}`);
		if (!result.ok) process.exit(1);
		console.log(
			`ok: ${target} is a valid widget bundle (${result.manifest?.widgets.length ?? 0} widget(s))`,
		);
		return;
	}

	const result = validateProject(target);
	for (const warning of result.warnings) console.warn(`warning: ${warning}`);
	for (const error of result.errors) console.error(`error: ${error}`);
	if (!result.ok) process.exit(1);
	console.log(
		`ok: ${result.widgets.length} widget contract(s) valid (${result.widgets
			.map((w) => w.id)
			.join(", ")})`,
	);
}

function runAdd(args: string[]): void {
	const { values, positionals } = parseArgs({
		args,
		allowPositionals: true,
		options: { group: { type: "string", default: "." } },
	});
	const widgetId = positionals[0];
	if (!widgetId)
		fail(
			"missing <widget-id> (usage: flow-like-widgets add <widget-id> [--group <dir>])",
		);
	const result = addWidget(values.group, widgetId);
	console.log(`created ${result.widgetDir}`);
	for (const file of result.files) console.log(`  ${file}`);
}

async function main(): Promise<void> {
	const [command, ...rest] = process.argv.slice(2);
	try {
		switch (command) {
			case "pack":
				await runPack(rest);
				break;
			case "dev":
				await runDev(rest);
				break;
			case "validate":
				runValidate(rest);
				break;
			case "add":
				runAdd(rest);
				break;
			case "help":
			case "--help":
			case "-h":
				console.log(USAGE);
				break;
			default:
				console.error(USAGE);
				process.exit(1);
		}
	} catch (e) {
		fail(e instanceof Error ? e.message : String(e));
	}
}

await main();
