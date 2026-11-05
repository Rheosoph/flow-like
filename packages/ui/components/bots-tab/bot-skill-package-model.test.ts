import { describe, expect, test } from "bun:test";
import type { PackageManifest } from "../../lib/schema/wasm";
import { toCandidate, uncoveredCapabilities } from "./bot-skill-package-model";

describe("uncoveredCapabilities", () => {
	test("network needs internet, oauth needs connectors, storage needs nothing", () => {
		expect(
			uncoveredCapabilities(["storage.node", "cache", "models"], []),
		).toEqual([]);
		expect(uncoveredCapabilities(["net.http", "oauth"], ["internet"])).toEqual([
			"connectors",
		]);
		expect(uncoveredCapabilities(["net.ws", "net.dns"], [])).toEqual([
			"internet",
		]);
		expect(uncoveredCapabilities(["database.write"], ["internet"])).toEqual([
			"data_studio",
		]);
		expect(uncoveredCapabilities(["teleport"], ["internet"])).toEqual([
			"unknown:teleport",
		]);
	});
});

describe("toCandidate", () => {
	const manifest = (extra: object) =>
		({
			id: "com.acme.pdf",
			name: "Pdf",
			version: "1.0.0",
			description: "d",
			permissions: { network: { httpEnabled: true }, oauthScopes: [] },
			...extra,
		}) as unknown as PackageManifest;

	test("only packages that declare skills are offered", () => {
		expect(toCandidate(manifest({}), undefined, false)).toBeNull();
		expect(toCandidate(manifest({ skills: [] }), undefined, false)).toBeNull();
		const found = toCandidate(
			manifest({ skills: [{ name: "pdf-forms", description: "Fill" }] }),
			undefined,
			true,
		);
		expect(found?.skills[0]?.name).toBe("pdf-forms");
		expect(found?.tags).toEqual(["net.http"]);
		expect(found?.installed).toBe(true);
	});
});
