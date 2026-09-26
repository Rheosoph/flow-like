import { beforeAll, describe, expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { unzipSync } from "fflate";
import {
	CONNECT_HOSTS_REMOVED,
	archiveNameCollisions,
	entryHash,
	pack,
	readPackageInfo,
	sha256Hex,
} from "../src/pack";
import {
	HELLO_WIDGET_CONFIG,
	type ProjectFixture,
	makeProjectFixture,
	tmpDir,
} from "./helpers";

const DECODER = new TextDecoder();

const CLI_PATH = join(import.meta.dir, "..", "src", "cli.ts");

const CSP_HOSTS = [
	"api.maptiler.com",
	"live.example.com",
	"tile.openstreetmap.org",
	"fonts.gstatic.com",
	"fonts.googleapis.com",
];

function metaCsp(html: string): string {
	const match =
		/<meta http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(html);
	if (!match?.[1]) throw new Error("packed document has no CSP meta");
	return match[1];
}

describe("readPackageInfo", () => {
	test("reads top-level id/version", () => {
		const dir = tmpDir("flwb-toml");
		writeFileSync(
			join(dir, "flow-like.toml"),
			'id = "com.example.top"\nversion = "0.1.0"\n',
		);
		expect(readPackageInfo(dir)).toEqual({
			id: "com.example.top",
			version: "0.1.0",
		});
	});

	test("reads the [package] table form", () => {
		const dir = tmpDir("flwb-toml");
		writeFileSync(
			join(dir, "flow-like.toml"),
			'[package]\nname = "Demo"\nid = "com.example.table"\nversion = "2.0.0"\n',
		);
		expect(readPackageInfo(dir)).toEqual({
			id: "com.example.table",
			version: "2.0.0",
		});
	});

	test("errors on missing keys and missing file", () => {
		const dir = tmpDir("flwb-toml");
		writeFileSync(join(dir, "flow-like.toml"), '[package]\nname = "Demo"\n');
		expect(() => readPackageInfo(dir)).toThrow(/'id'/);
		expect(() => readPackageInfo(tmpDir("flwb-toml"))).toThrow(
			/flow-like\.toml/,
		);
	});
});

describe("pack", () => {
	let fixture: ProjectFixture;

	beforeAll(() => {
		Reflect.deleteProperty(process.env, "SOURCE_DATE_EPOCH");
		fixture = makeProjectFixture();
	});

	test("produces a valid, hash-consistent .flwb", async () => {
		const result = await pack(fixture.projectDir, {
			out: join(fixture.projectDir, "widgets.flwb"),
			servingPrefix: "flow-widget://com.example.demo@hash/",
			quiet: true,
		});

		expect(result.hash).toBe(sha256Hex(result.bytes));
		expect(result.warnings).toEqual([]);

		const entries = unzipSync(result.bytes);
		const paths = Object.keys(entries);
		expect(paths).toEqual([...paths].sort());
		expect(paths).toEqual([
			"bundle.json",
			"shared/react-abc123.js",
			"widgets/hello-widget/contract.json",
			"widgets/hello-widget/index.html",
		]);

		const manifest = JSON.parse(
			DECODER.decode(entries["bundle.json"] as Uint8Array),
		);
		expect(manifest.formatVersion).toBe(1);
		expect(manifest.packageId).toBe("com.example.demo");
		expect(manifest.packageVersion).toBe("1.2.0");
		expect(manifest.protocol).toBe("flw/1");
		expect(manifest.createdAt).toBeUndefined();
		expect(manifest.shared).toEqual([
			{
				path: "shared/react-abc123.js",
				hash: entryHash(entries["shared/react-abc123.js"] as Uint8Array),
			},
		]);
		expect(manifest.widgets).toHaveLength(1);
		const widget = manifest.widgets[0];
		expect(widget.id).toBe("hello-widget");
		expect(widget.name).toBe("Hello Widget");
		expect(widget.description).toBe("Says hello");
		expect(widget.entry).toBe("widgets/hello-widget/index.html");
		expect(widget.contract).toBe("widgets/hello-widget/contract.json");
		expect(widget.entryHash).toBe(
			entryHash(entries["widgets/hello-widget/index.html"] as Uint8Array),
		);
		expect(widget.assets).toEqual(["shared/react-abc123.js"]);
		expect(widget.framework).toBe("react");
		expect(widget.sizeHint.raw).toBeGreaterThan(0);
		expect(widget.sizeHint.gzip).toBeGreaterThan(0);

		const html = DECODER.decode(
			entries["widgets/hello-widget/index.html"] as Uint8Array,
		);
		expect(html).toContain('http-equiv="Content-Security-Policy"');
		expect(html).toContain("flow-widget://com.example.demo@hash/");
		expect(html).toContain("connect-src data: blob:; worker-src 'none'");
		expect(html).toContain('src="../../shared/react-abc123.js"');
		expect(html).toContain("hello entry");
		expect(html).toContain("<style>#root { color: red; }");
		expect(html).toContain("globalThis.__FLW_CONTRACT__");
		expect(html.indexOf("__FLW_CONTRACT__")).toBeLessThan(
			html.indexOf("hello entry"),
		);
		expect(html).not.toContain('src="./index.js"');

		const contract = JSON.parse(
			DECODER.decode(
				entries["widgets/hello-widget/contract.json"] as Uint8Array,
			),
		);
		expect(contract.contractVersion).toBe(1);
		expect(contract.id).toBe("hello-widget");
		expect(contract.inputs.greeting).toEqual({
			type: "string",
			description: "Greeting text",
			default: "Hello",
		});
		expect(contract.events.dismissed).toEqual({ payloadSchema: null });
		expect(contract.queries.getGreeting).toEqual({
			argsSchema: null,
			resultSchema: { type: "string" },
		});
		expect(contract.sizing).toEqual({
			defaultHeight: 200,
			resizable: false,
			maxHeight: 600,
		});

		expect(result.report).toContain("hello-widget");
		expect(result.report).toContain(result.hash);
	}, 60000);

	test("is deterministic (two packs are byte-identical)", async () => {
		const a = await pack(fixture.projectDir, {
			out: join(tmpDir("flwb-out"), "a.flwb"),
			quiet: true,
		});
		const b = await pack(fixture.projectDir, {
			out: join(tmpDir("flwb-out"), "b.flwb"),
			quiet: true,
		});
		expect(a.hash).toBe(b.hash);
		expect(Buffer.compare(Buffer.from(a.bytes), Buffer.from(b.bytes))).toBe(0);
	}, 60000);

	test("stamps createdAt only when requested", async () => {
		const result = await pack(fixture.projectDir, {
			out: join(tmpDir("flwb-out"), "c.flwb"),
			createdAt: "2026-07-21T12:00:00Z",
			quiet: true,
		});
		expect(result.manifest.createdAt).toBe("2026-07-21T12:00:00Z");
		const entries = unzipSync(result.bytes);
		expect(DECODER.decode(entries["bundle.json"] as Uint8Array)).toContain(
			'"createdAt": "2026-07-21T12:00:00Z"',
		);
	}, 60000);

	test("errors clearly when the built document is missing", async () => {
		const broken = makeProjectFixture();
		const { rmSync } = await import("node:fs");
		rmSync(join(broken.groupDir, "dist"), { recursive: true });
		await expect(pack(broken.projectDir, { quiet: true })).rejects.toThrow(
			/bun run build/,
		);
	}, 60000);

	test("packs csp into a v2 contract but never into the meta CSP", async () => {
		const project = makeProjectFixture();
		writeFileSync(
			project.widgetConfigPath,
			HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				`id: "hello-widget",
	capabilities: { workers: true, media: true },
	csp: [
		{
			reason: "Loads map tiles and live positions",
			connectSrc: ["wss://live.example.com", "https://API.maptiler.com"],
			imgSrc: ["https://a.tile.openstreetmap.org"],
			mediaSrc: ["https://media.maptiler.com"],
		},
		{
			reason: "Loads web fonts for labels",
			fontSrc: ["https://fonts.gstatic.com"],
			styleSrc: ["https://fonts.googleapis.com"],
		},
	],`,
			),
		);
		const result = await pack(project.projectDir, {
			out: join(tmpDir("flwb-out"), "csp.flwb"),
			quiet: true,
		});
		const entries = unzipSync(result.bytes);
		const contract = JSON.parse(
			DECODER.decode(
				entries["widgets/hello-widget/contract.json"] as Uint8Array,
			),
		);
		expect(contract.contractVersion).toBe(2);
		expect(contract.csp).toEqual([
			{
				reason: "Loads map tiles and live positions",
				connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
				imgSrc: ["https://a.tile.openstreetmap.org"],
				mediaSrc: ["https://media.maptiler.com"],
			},
			{
				reason: "Loads web fonts for labels",
				fontSrc: ["https://fonts.gstatic.com"],
				styleSrc: ["https://fonts.googleapis.com"],
			},
		]);

		const html = DECODER.decode(
			entries["widgets/hello-widget/index.html"] as Uint8Array,
		);
		const csp = metaCsp(html);
		for (const host of [...CSP_HOSTS, "media.maptiler.com", "https:", "wss:"]) {
			expect(csp).not.toContain(host);
		}
		expect(csp).toContain(
			"connect-src data: blob: 'self' flow-widget: http://flow-widget.localhost;",
		);
		expect(csp).toEndWith(
			"media-src data: blob: 'self' flow-widget: http://flow-widget.localhost",
		);
	}, 60000);

	test("rejects the removed connectHosts option", async () => {
		const project = makeProjectFixture();
		await expect(
			pack(project.projectDir, {
				quiet: true,
				connectHosts: ["https://api.example.com"],
			} as never),
		).rejects.toThrow(CONNECT_HOSTS_REMOVED);
	});

	test.skipIf(process.platform === "win32")(
		"refuses shared chunk file names the hub would refuse",
		async () => {
			const project = makeProjectFixture();
			const out = join(tmpDir("flwb-unsafe"), "widgets.flwb");
			writeFileSync(
				join(project.groupDir, "dist", "shared", "chunk:ads.js"),
				"x",
			);
			await expect(
				pack(project.projectDir, { out, quiet: true }),
			).rejects.toThrow("Unsafe widget bundle entry path: shared/chunk:ads.js");
			expect(existsSync(out)).toBeFalse();
		},
		60000,
	);

	test("rejects package ids the hub would refuse", async () => {
		const dir = tmpDir("flwb-toml");
		writeFileSync(
			join(dir, "flow-like.toml"),
			'id = "com.example; script-src *"\nversion = "1.0.0"\n',
		);
		expect(() => readPackageInfo(dir)).toThrow(
			/Invalid package id "com\.example; script-src \*"/,
		);
	});
});

describe("archiveNameCollisions", () => {
	test("flags names that alias on case-insensitive filesystems", () => {
		expect(
			archiveNameCollisions([
				"bundle.json",
				"widgets/x/contract.json",
				"widgets/x/CONTRACT.json",
				"shared/a.js.",
				"shared/A.js",
				"shared/dir/",
				"shared/Dir",
				"shared/dir/chunk.js",
				"shared/ ./x.js",
			]),
		).toEqual([
			"Widget bundle entry 'shared/ ./x.js' has a path segment made only of dots or spaces",
			"Widget bundle entries 'widgets/x/contract.json' and 'widgets/x/CONTRACT.json' collide on case-insensitive filesystems",
			"Widget bundle entries 'shared/a.js.' and 'shared/A.js' collide on case-insensitive filesystems",
			"Widget bundle entry 'shared/Dir' collides with directory of 'shared/dir/chunk.js' on case-insensitive filesystems",
		]);
	});

	test("folds Unicode case like Rust", () => {
		expect(archiveNameCollisions(["shared/K.js", "shared/k.js"])).toHaveLength(
			1,
		);
		expect(
			archiveNameCollisions([
				"bundle.json",
				"shared/a.js",
				"widgets/a/index.html",
			]),
		).toEqual([]);
	});
});

describe("cli", () => {
	test("--connect exits non-zero with the migration error", () => {
		for (const args of [
			["--connect", "https://api.example.com"],
			["--connect=https://api.example.com"],
			["--connect"],
		]) {
			const result = Bun.spawnSync([
				process.execPath,
				CLI_PATH,
				"pack",
				"--project",
				tmpDir("flwb-cli"),
				...args,
			]);
			expect(result.exitCode).not.toBe(0);
			expect(result.stderr.toString()).toContain(
				'The --connect flag was removed: declare network sources per widget in widget.config.ts "csp"',
			);
		}
	});
});
