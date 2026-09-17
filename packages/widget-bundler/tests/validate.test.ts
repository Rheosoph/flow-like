import { beforeAll, describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { unzipSync, zipSync } from "fflate";
import { pack } from "../src/pack";
import { validateBundle, validateProject } from "../src/validate-cmd";
import {
	HELLO_WIDGET_CONFIG,
	type ProjectFixture,
	makeProjectFixture,
	tmpDir,
} from "./helpers";

const ENCODER = new TextEncoder();
const DECODER = new TextDecoder();

const CSP_DECLARATION = `id: "hello-widget",
	csp: { connectSrc: ["https://api.maptiler.com"], imgSrc: ["https://tiles.maptiler.com"] },`;

function withCspWidget(fixture: ProjectFixture, declaration: string): void {
	writeFileSync(
		fixture.widgetConfigPath,
		HELLO_WIDGET_CONFIG.replace('id: "hello-widget",', declaration),
	);
}

type Entries = Record<string, Uint8Array>;

function writeBundle(entries: Entries): string {
	const path = join(tmpDir("flwb-edit"), "edited.flwb");
	writeFileSync(path, zipSync(entries));
	return path;
}

function editManifest(
	entries: Entries,
	edit: (manifest: Record<string, unknown>) => void,
): void {
	const manifest = JSON.parse(
		DECODER.decode(entries["bundle.json"] as Uint8Array),
	);
	edit(manifest);
	entries["bundle.json"] = ENCODER.encode(JSON.stringify(manifest));
}

describe("validateProject", () => {
	test("accepts the synthetic project", () => {
		const fixture = makeProjectFixture();
		const result = validateProject(fixture.projectDir);
		expect(result.errors).toEqual([]);
		expect(result.ok).toBeTrue();
		expect(result.widgets).toEqual([{ id: "hello-widget", group: "react" }]);
	}, 60000);

	test("reports a missing flow-like.toml", () => {
		const result = validateProject(tmpDir("flwb-empty"));
		expect(result.ok).toBeFalse();
		expect(result.errors.some((e) => e.includes("flow-like.toml"))).toBeTrue();
	});

	test("accepts a valid csp declaration", () => {
		const fixture = makeProjectFixture();
		withCspWidget(fixture, CSP_DECLARATION);
		const result = validateProject(fixture.projectDir);
		expect(result.errors).toEqual([]);
		expect(result.ok).toBeTrue();
	}, 60000);

	test("reports csp grammar errors", () => {
		const fixture = makeProjectFixture();
		withCspWidget(
			fixture,
			'id: "hello-widget", csp: { connectSrc: ["https://localhost"] },',
		);
		const result = validateProject(fixture.projectDir);
		expect(result.ok).toBeFalse();
		expect(result.errors).toEqual([
			'Invalid widget csp source "https://localhost" in connectSrc for widget hello-widget: host uses a reserved or local-only name',
		]);
	}, 60000);
});

describe("validateBundle", () => {
	let fixture: ProjectFixture;
	let flwbPath: string;
	let bytes: Uint8Array;

	beforeAll(async () => {
		fixture = makeProjectFixture();
		flwbPath = join(fixture.projectDir, "widgets.flwb");
		({ bytes } = await pack(fixture.projectDir, {
			out: flwbPath,
			quiet: true,
		}));
	});

	test("accepts a freshly packed bundle", () => {
		const result = validateBundle(flwbPath);
		expect(result.errors).toEqual([]);
		expect(result.ok).toBeTrue();
		expect(result.manifest?.widgets[0]?.id).toBe("hello-widget");
	}, 60000);

	test("catches a tampered entry", () => {
		const entries = unzipSync(bytes);
		entries["widgets/hello-widget/index.html"] = new TextEncoder().encode(
			"<html>tampered</html>",
		);
		const tamperedPath = join(tmpDir("flwb-tamper"), "tampered.flwb");
		writeFileSync(tamperedPath, zipSync(entries));

		const result = validateBundle(tamperedPath);
		expect(result.ok).toBeFalse();
		expect(
			result.errors.some((e) =>
				e.includes(
					"Hash mismatch for widget entry widgets/hello-widget/index.html",
				),
			),
		).toBeTrue();
	}, 60000);

	test("reports a missing bundle file", () => {
		const result = validateBundle("/nonexistent/widgets.flwb");
		expect(result.ok).toBeFalse();
		expect(result.errors[0]).toContain("not found");
	});

	test("requires the exact entry and contract paths", () => {
		const entries = unzipSync(bytes);
		entries["widgets/hello-widget/main.html"] = entries[
			"widgets/hello-widget/index.html"
		] as Uint8Array;
		editManifest(entries, (manifest) => {
			const [widget] = manifest.widgets as Record<string, unknown>[];
			if (widget) widget.entry = "widgets/hello-widget/main.html";
		});
		const result = validateBundle(writeBundle(entries));
		expect(result.errors).toEqual([
			"Widget 'hello-widget' entry path 'widgets/hello-widget/main.html' must be 'widgets/hello-widget/index.html'",
		]);
	});

	test("rejects invalid package ids and aliasing archive names", () => {
		const entries = unzipSync(bytes);
		entries["widgets/hello-widget/CONTRACT.json"] = ENCODER.encode("{}");
		editManifest(entries, (manifest) => {
			manifest.packageId = 'com.example"; script-src *';
		});
		const result = validateBundle(writeBundle(entries));
		expect(result.errors).toEqual([
			"Invalid bundle packageId \"com.example\\\"; script-src *\": use only letters, digits, '.', '_' and '-'",
			"Widget bundle entries 'widgets/hello-widget/contract.json' and 'widgets/hello-widget/CONTRACT.json' collide on case-insensitive filesystems",
		]);
	});
});

describe("validateBundle csp contracts", () => {
	let bytes: Uint8Array;

	beforeAll(async () => {
		const fixture = makeProjectFixture();
		withCspWidget(fixture, CSP_DECLARATION);
		({ bytes } = await pack(fixture.projectDir, {
			out: join(fixture.projectDir, "widgets.flwb"),
			quiet: true,
		}));
	});

	function withContract(contract: Record<string, unknown>): string {
		const entries = unzipSync(bytes);
		entries["widgets/hello-widget/contract.json"] = ENCODER.encode(
			JSON.stringify(contract, null, 2),
		);
		return writeBundle(entries);
	}

	function packedContract(): Record<string, unknown> {
		return JSON.parse(
			DECODER.decode(
				unzipSync(bytes)["widgets/hello-widget/contract.json"] as Uint8Array,
			),
		);
	}

	test("accepts a packed v2 bundle", () => {
		const path = join(tmpDir("flwb-csp"), "csp.flwb");
		writeFileSync(path, bytes);
		const result = validateBundle(path);
		expect(result.errors).toEqual([]);
		expect(packedContract().contractVersion).toBe(2);
	});

	test("rejects csp on a v1 contract and v2 without csp", () => {
		expect(
			validateBundle(withContract({ ...packedContract(), contractVersion: 1 }))
				.errors,
		).toEqual([
			"Widget 'hello-widget' declares csp and must use contractVersion 2",
		]);
		const { csp: _csp, ...withoutCsp } = packedContract();
		expect(validateBundle(withContract(withoutCsp)).errors).toEqual([
			"Widget 'hello-widget' uses contractVersion 2 without csp; contracts without csp must use contractVersion 1",
		]);
	});

	test("rejects hand-edited csp sources and directives", () => {
		const result = validateBundle(
			withContract({
				...packedContract(),
				csp: {
					connectSrc: ["https://*.maptiler.com"],
					scriptSrc: ["https://cdn.example.org"],
				},
			}),
		);
		expect(result.ok).toBeFalse();
		expect(result.errors).toEqual([
			"Widget 'hello-widget': csp declares unknown directive \"scriptSrc\" (allowed: connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc)",
			"Widget 'hello-widget': Invalid csp source \"https://*.maptiler.com\" in connectSrc: wildcards are not allowed",
		]);
	});
});
