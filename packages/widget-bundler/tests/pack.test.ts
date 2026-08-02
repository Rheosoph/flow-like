import { beforeAll, describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { unzipSync } from "fflate";
import { entryHash, pack, readPackageInfo, sha256Hex } from "../src/pack";
import { type ProjectFixture, makeProjectFixture, tmpDir } from "./helpers";

const DECODER = new TextDecoder();

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
			connectHosts: ["https://api.example.com"],
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
		expect(html).toContain("connect-src https://api.example.com");
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
});
