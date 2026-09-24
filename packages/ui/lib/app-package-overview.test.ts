import { describe, expect, test } from "bun:test";
import {
	accessGroupOf,
	capabilityTagsFromNodePermissions,
	filterNodeGroups,
	groupAccessTags,
	groupPackageNodesByCategory,
	hasElevatedAccess,
	packageAccess,
	readManifestAccess,
} from "./app-package-overview";
import { INodePermission } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";

function node(
	name: string,
	category: string,
	permissions: INodePermission[] = [],
	description = "",
): INode {
	return {
		name,
		friendly_name: name,
		category,
		description,
		wasm: { package_id: "pkg", permissions },
	} as unknown as INode;
}

const SNAKE_MANIFEST = {
	description: "Turn notation into audio",
	permissions: {
		memory: "huge",
		timeout: "long_running",
		network: {
			http_enabled: true,
			allowed_hosts: ["cdn.flow-like.com"],
			tcp_enabled: false,
		},
		filesystem: { node_storage: true, user_storage: true, cache_dir: true },
		oauth_scopes: [{ provider: "google", scopes: ["drive"], reason: "x" }],
		models: false,
	},
};

describe("readManifestAccess", () => {
	test("reads the snake_case wire", () => {
		expect(readManifestAccess(SNAKE_MANIFEST)).toEqual({
			description: "Turn notation into audio",
			memory: "huge",
			timeout: "long_running",
			allowedHosts: ["cdn.flow-like.com"],
			oauthProviders: ["google"],
			capabilityTags: [
				"net.http",
				"oauth",
				"storage.user",
				"storage.node",
				"storage.cache",
			],
		});
	});

	test("reads the camelCase mirror", () => {
		const access = readManifestAccess({
			permissions: {
				network: { httpEnabled: true, allowedHosts: ["a.example"] },
				filesystem: { uploadDir: true },
			},
		});
		expect(access?.allowedHosts).toEqual(["a.example"]);
		expect(access?.capabilityTags).toEqual(["net.http", "storage.uploads"]);
	});

	test("tolerates missing and malformed manifests", () => {
		expect(readManifestAccess(null)).toBeUndefined();
		expect(readManifestAccess({ permissions: "nope" })).toEqual({
			description: undefined,
			memory: undefined,
			timeout: undefined,
			allowedHosts: [],
			oauthProviders: [],
			capabilityTags: [],
		});
	});
});

describe("capabilityTagsFromNodePermissions", () => {
	test("expands storage to every directory and orders sensitive first", () => {
		expect(
			capabilityTagsFromNodePermissions([
				INodePermission.Streaming,
				INodePermission.StorageRead,
				INodePermission.NetworkHttp,
				INodePermission.StorageWrite,
				INodePermission.Functions,
			]),
		).toEqual([
			"net.http",
			"storage.user",
			"storage.node",
			"storage.uploads",
			"storage.cache",
			"streaming",
		]);
	});
});

describe("packageAccess", () => {
	test("node permissions override the manifest's capability flags", () => {
		const access = packageAccess(
			[node("Parse", "Media/Music", [INodePermission.Models])],
			readManifestAccess(SNAKE_MANIFEST),
		);
		expect(access.tags).toEqual(["oauth", "models"]);
		expect(access.hosts).toEqual(["cdn.flow-like.com"]);
		expect(access.memory).toBe("huge");
	});

	test("falls back to the manifest when the catalog lists no nodes", () => {
		const access = packageAccess([], readManifestAccess(SNAKE_MANIFEST));
		expect(access.tags[0]).toBe("net.http");
	});

	test("a package without nodes or manifest asks for nothing", () => {
		const access = packageAccess([], undefined);
		expect(access).toEqual({
			tags: [],
			hosts: [],
			memory: undefined,
			timeout: undefined,
			oauthProviders: [],
		});
		expect(hasElevatedAccess(access.tags)).toBe(false);
	});
});

describe("groupAccessTags / hasElevatedAccess", () => {
	test("runtime-only tags are not elevated", () => {
		expect(hasElevatedAccess(["variables", "streaming"])).toBe(false);
		expect(hasElevatedAccess(["storage.cache"])).toBe(true);
		expect(groupAccessTags(["net.http", "oauth", "cache"])).toEqual({
			network: ["net.http"],
			files: [],
			accounts: ["oauth"],
			models: [],
			data: [],
			runtime: ["cache"],
		});
	});

	test("a widget that reaches external sites is network access", () => {
		expect(accessGroupOf("widget.net")).toBe("network");
		expect(hasElevatedAccess(["widget.net"])).toBe(true);
	});
});

describe("groupPackageNodesByCategory", () => {
	const groups = groupPackageNodesByCategory(
		new Map([
			[
				"typst",
				[
					node("Typst Source to PDF", "Document/Typst"),
					node("Markdown to Typst", "Document/Typst"),
				],
			],
			["midi", [node("Parse Music Score", "Media/Music", [], "ABC score")]],
			["misc", [node("Loose", "")]],
			["other", [node("Extra PDF", "Document/Typst")]],
		]),
		"Uncategorized",
	);

	test("sorts categories and nodes, and records every contributing package", () => {
		expect(groups.map((group) => group.label)).toEqual([
			"Document / Typst",
			"Media / Music",
			"Uncategorized",
		]);
		expect(groups[0].nodes.map((n) => n.name)).toEqual([
			"Extra PDF",
			"Markdown to Typst",
			"Typst Source to PDF",
		]);
		expect(groups[0].packageIds).toEqual(["typst", "other"]);
	});

	test("filters by node text, category or package name", () => {
		const names = new Map([["midi", "MIDI Music"]]);
		expect(
			filterNodeGroups(groups, "score", names).map((g) => g.nodes.length),
		).toEqual([1]);
		expect(filterNodeGroups(groups, "midi", names)[0].label).toBe(
			"Media / Music",
		);
		expect(filterNodeGroups(groups, "typst", names)[0].nodes).toHaveLength(3);
		expect(filterNodeGroups(groups, "  ", names)).toHaveLength(3);
	});
});
