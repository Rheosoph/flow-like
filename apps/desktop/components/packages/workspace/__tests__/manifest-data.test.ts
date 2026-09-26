import { describe, expect, test } from "vitest";
import { normalizeManifest } from "../manifest-data";

describe("normalizeManifest", () => {
	test("fills the fields a manifest may omit", () => {
		const manifest = normalizeManifest({
			manifest_version: 1,
			id: "com.custom.torrent",
			name: "Torrent",
			version: "0.3.0",
			description: "BitTorrent tools",
			license: "MIT",
			keywords: ["torrent"],
			permissions: { memory: "intensive", timeout: "long_running" },
		});

		expect(manifest.authors).toEqual([]);
		expect(manifest.keywords).toEqual(["torrent"]);
		expect(manifest.permissions).toEqual({
			memory: "intensive",
			timeout: "long_running",
		});
	});

	test("defaults a missing permissions table to the standard tiers", () => {
		const manifest = normalizeManifest({ id: "com.acme.pkg" });

		expect(manifest.permissions).toEqual({
			memory: "standard",
			timeout: "standard",
		});
		expect(manifest.description).toBe("");
		expect(manifest.keywords).toEqual([]);
	});

	test("keeps keys the editor does not author", () => {
		const nodes = [{ id: "add_torrent", name: "Add Torrent" }];
		const manifest = normalizeManifest({
			id: "com.custom.torrent",
			nodes,
			permissions: { memory: "heavy", network: { allowed_hosts: ["a.com"] } },
		});

		expect((manifest as unknown as { nodes: unknown }).nodes).toEqual(nodes);
		expect(manifest.permissions.network).toEqual({ allowed_hosts: ["a.com"] });
	});

	test("drops malformed author and keyword entries", () => {
		const manifest = normalizeManifest({
			authors: ["Jane <jane@example.com>", { name: "Joe", email: "j@x.io" }],
			keywords: ["ok", 3],
		});

		expect(manifest.authors).toEqual([{ name: "Joe", email: "j@x.io" }]);
		expect(manifest.keywords).toEqual(["ok"]);
	});
});
