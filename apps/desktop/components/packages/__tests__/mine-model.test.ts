import { PackagePermissionBits } from "@flow-like/flow-like-ui/lib/permission/wasm-package-permission";
import {
	PackageStatus,
	type PackageSummary,
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import { describe, expect, test } from "vitest";
import {
	type MineProjectInput,
	compareVersions,
	deriveMine,
	findWorkspaceEntry,
	idsToLookUp,
	isPlaceholderId,
	localSummary,
	matchesMineFilter,
	mineActionTab,
	mineIssueTone,
	sortMine,
	workspaceHeaderAction,
} from "../mine-model";

function project(
	id: string,
	manifest: MineProjectInput["manifest"],
	extra: Partial<Omit<MineProjectInput, "project" | "manifest">> = {},
): MineProjectInput {
	return {
		project: {
			id,
			path: `/work/${id}`,
			language: "rust",
			name: id,
			createdAt: "2026-01-01T00:00:00Z",
		},
		manifest,
		...extra,
	};
}

function summary(
	id: string,
	latestVersion: string,
	extra: Partial<PackageSummary> = {},
): PackageSummary {
	return {
		id,
		name: `${id} name`,
		description: `${id} description`,
		latestVersion,
		downloadCount: 10,
		status: PackageStatus.Active,
		keywords: [],
		verified: false,
		price: 0,
		visibility: "public",
		viewerPermission: PackagePermissionBits.Owner,
		...extra,
	};
}

function entryFor(model: ReturnType<typeof deriveMine>, key: string) {
	const entry = model.entries.find((candidate) => candidate.key === key);
	if (!entry) throw new Error(`No entry ${key} in ${model.entries.length}`);
	return entry;
}

describe("compareVersions", () => {
	test("orders core versions numerically", () => {
		expect(compareVersions("0.10.0", "0.9.1")).toBe(1);
		expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
		expect(compareVersions("v1.2", "1.2.0")).toBe(0);
		expect(compareVersions("0.2.1", "0.3.0")).toBe(-1);
	});

	test("puts a pre-release below its release and above the previous one", () => {
		expect(compareVersions("1.5.0-dev", "1.5.0")).toBe(-1);
		expect(compareVersions("1.5.0-dev", "1.4.0")).toBe(1);
		expect(compareVersions("1.0.0-alpha.2", "1.0.0-alpha.10")).toBe(-1);
		expect(compareVersions("1.0.0-alpha", "1.0.0-alpha.1")).toBe(-1);
		expect(compareVersions("1.0.0-1", "1.0.0-alpha")).toBe(-1);
		expect(compareVersions("1.0.0+build.5", "1.0.0")).toBe(0);
	});

	test("returns null for non-versions", () => {
		expect(compareVersions("latest", "1.0.0")).toBeNull();
	});
});

describe("deriveMine", () => {
	test("groups checkouts with the same manifest id, newest first", () => {
		const model = deriveMine(
			[
				project("a", { id: "pkg.rust", version: "1.4.0" }),
				project("b", { id: "pkg.rust", version: "1.5.0-dev" }),
				project("c", null),
			],
			{ maintained: [summary("pkg.rust", "1.4.0")], lookedUp: [] },
		);

		expect(model.entries).toHaveLength(2);
		const rust = entryFor(model, "pkg.rust");
		expect(rust.checkouts.map((checkout) => checkout.projectId)).toEqual([
			"b",
			"a",
		]);
		expect(rust.checkouts[0].newest).toBe(true);
		expect(rust.checkouts[1].matchesLive).toBe(true);
		expect(rust.localVersion).toBe("1.5.0-dev");
		expect(rust.state).toBe("unpublished-changes");
		expect(rust.primaryAction).toEqual({
			kind: "publish",
			path: "/work/b",
			version: "1.5.0-dev",
			first: false,
		});

		const folder = entryFor(model, "project:c");
		expect(folder.packageId).toBeNull();
		expect(folder.name).toBe("c");
		expect(folder.state).toBe("local-only");
	});

	test("derives every state", () => {
		const model = deriveMine(
			[
				project("local", { id: "pkg.local", version: "0.1.0" }),
				project("ahead", { id: "pkg.ahead", version: "0.3.0" }),
				project("review", { id: "pkg.review", version: "0.9.0" }),
				project("live", { id: "pkg.live", version: "0.4.2" }),
				project("behind", { id: "pkg.behind", version: "0.1.0" }),
			],
			{
				maintained: [
					summary("pkg.ahead", "0.2.1"),
					summary("pkg.review", "0.9.0", {
						status: PackageStatus.PendingReview,
					}),
					summary("pkg.live", "0.4.2"),
					summary("pkg.behind", "0.2.0"),
					summary("pkg.remote", "2.1.0", {
						viewerPermission: PackagePermissionBits.Maintainer,
					}),
				],
				lookedUp: [],
			},
		);

		expect(entryFor(model, "pkg.local").state).toBe("local-only");
		expect(entryFor(model, "pkg.local").primaryAction).toMatchObject({
			kind: "publish",
			first: true,
		});
		expect(entryFor(model, "pkg.ahead").state).toBe("unpublished-changes");
		expect(entryFor(model, "pkg.review").state).toBe("in-review");
		expect(entryFor(model, "pkg.review").primaryAction).toEqual({
			kind: "view-review",
			packageId: "pkg.review",
		});
		expect(entryFor(model, "pkg.live").state).toBe("live");
		expect(entryFor(model, "pkg.live").primaryAction).toEqual({
			kind: "open",
			packageId: "pkg.live",
		});
		expect(entryFor(model, "pkg.behind").state).toBe("live");

		const remote = entryFor(model, "pkg.remote");
		expect(remote.state).toBe("not-on-this-machine");
		expect(remote.checkouts).toEqual([]);
		expect(remote.primaryAction).toEqual({
			kind: "link-folder",
			packageId: "pkg.remote",
		});
	});

	test("drops listed packages the caller only uses", () => {
		const model = deriveMine([], {
			maintained: [
				summary("pkg.bought", "1.0.0", {
					viewerPermission: PackagePermissionBits.Buyer,
				}),
				summary("pkg.legacy", "1.0.0", { viewerPermission: undefined }),
			],
			lookedUp: [],
		});
		expect(model.entries.map((entry) => entry.key)).toEqual(["pkg.legacy"]);
	});

	test("flags an id someone else holds", () => {
		const model = deriveMine(
			[project("mine", { id: "pkg.taken", version: "0.1.0" })],
			{
				maintained: [],
				lookedUp: [
					summary("pkg.taken", "3.0.0", {
						viewerPermission: PackagePermissionBits.User,
					}),
					summary("pkg.unrelated", "1.0.0"),
				],
			},
		);

		expect(model.entries).toHaveLength(1);
		const taken = entryFor(model, "pkg.taken");
		expect(taken.idTaken).toBe(true);
		expect(taken.state).toBe("local-only");
		expect(taken.registry).toBeNull();
		expect(taken.issues).toEqual([{ kind: "id-taken" }]);
		expect(taken.primaryAction).toEqual({
			kind: "rename-id",
			path: "/work/mine",
		});
	});

	test("treats a looked-up package the caller maintains as theirs", () => {
		const model = deriveMine(
			[project("mine", { id: "pkg.overflow", version: "1.0.0" })],
			{
				maintained: [],
				lookedUp: [
					summary("pkg.overflow", "1.0.0", {
						viewerPermission: PackagePermissionBits.Maintainer,
					}),
				],
			},
		);
		const entry = entryFor(model, "pkg.overflow");
		expect(entry.idTaken).toBe(false);
		expect(entry.state).toBe("live");
	});

	test("a lookup hit without a viewer permission counts as taken", () => {
		const model = deriveMine(
			[project("mine", { id: "pkg.legacy", version: "1.0.0" })],
			{
				maintained: [],
				lookedUp: [
					summary("pkg.legacy", "1.0.0", { viewerPermission: undefined }),
				],
			},
		);
		expect(entryFor(model, "pkg.legacy").idTaken).toBe(true);
	});

	test("lint errors win over staleness, staleness over the state action", () => {
		const model = deriveMine(
			[
				project(
					"lint",
					{ id: "pkg.lint", version: "0.1.0" },
					{
						lint: { errors: 2, warnings: 1 },
						stale: true,
					},
				),
				project(
					"stale",
					{ id: "pkg.stale", version: "0.2.0" },
					{ stale: true },
				),
			],
			null,
		);

		const lint = entryFor(model, "pkg.lint");
		expect(lint.issues).toEqual([
			{ kind: "lint", errors: 2 },
			{ kind: "stale" },
		]);
		expect(lint.primaryAction).toEqual({
			kind: "fix-errors",
			path: "/work/lint",
			errors: 2,
		});
		expect(entryFor(model, "pkg.stale").primaryAction).toEqual({
			kind: "reload",
			path: "/work/stale",
		});
	});

	test("without the registry every checkout is local only", () => {
		const model = deriveMine(
			[project("a", { id: "pkg.a", version: "1.0.0" })],
			null,
		);
		expect(entryFor(model, "pkg.a").state).toBe("local-only");
		expect(model.counts["not-on-this-machine"]).toBe(0);
	});

	test("counts every filter", () => {
		const model = deriveMine(
			[
				project(
					"local",
					{ id: "pkg.local", version: "0.1.0" },
					{
						lint: { errors: 1, warnings: 0 },
					},
				),
				project("ahead", { id: "pkg.ahead", version: "0.3.0" }),
				project("live", { id: "pkg.live", version: "1.0.0" }, { stale: true }),
			],
			{
				maintained: [
					summary("pkg.ahead", "0.2.0"),
					summary("pkg.live", "1.0.0"),
					summary("pkg.remote", "1.0.0"),
				],
				lookedUp: [],
			},
		);

		expect(model.counts).toEqual({
			all: 4,
			"unpublished-changes": 1,
			"in-review": 0,
			live: 1,
			"local-only": 1,
			"not-on-this-machine": 1,
			disabled: 0,
			issues: 2,
		});
		expect(
			model.entries.filter((entry) => matchesMineFilter(entry, "issues")),
		).toHaveLength(2);
	});

	test("sorts entries that need attention first", () => {
		const model = deriveMine(
			[
				project("draft", { id: "a.draft", version: "0.1.0" }),
				project("live", { id: "b.live", version: "1.0.0" }),
				project("review", { id: "c.review", version: "1.0.0" }),
				project("ahead", { id: "d.ahead", version: "2.0.0" }),
				project("stale", { id: "e.stale", version: "1.0.0" }, { stale: true }),
				project(
					"lint",
					{ id: "f.lint", version: "1.0.0" },
					{
						lint: { errors: 3, warnings: 0 },
					},
				),
			],
			{
				maintained: [
					summary("b.live", "1.0.0", { name: "B" }),
					summary("c.review", "1.0.0", {
						name: "C",
						status: PackageStatus.PendingReview,
					}),
					summary("d.ahead", "1.0.0", { name: "D" }),
					summary("g.remote", "1.0.0", { name: "G" }),
				],
				lookedUp: [],
			},
		);

		expect(model.entries.map((entry) => entry.key)).toEqual([
			"f.lint",
			"e.stale",
			"d.ahead",
			"c.review",
			"b.live",
			"g.remote",
			"a.draft",
		]);
		expect(sortMine(model.entries, "name").map((entry) => entry.name)).toEqual([
			"B",
			"C",
			"D",
			"draft",
			"G",
			"lint",
			"stale",
		]);
	});
});

describe("placeholder ids", () => {
	test("recognises the template prefix only", () => {
		expect(isPlaceholderId("com.example.my-package")).toBe(true);
		expect(isPlaceholderId(" Com.Example.Widgets ")).toBe(true);
		expect(isPlaceholderId("com.examples.real")).toBe(false);
		expect(isPlaceholderId("dev.example.pkg")).toBe(false);
		expect(isPlaceholderId(null)).toBe(false);
	});

	test("a template id is a warning whose fix is choosing an id", () => {
		const model = deriveMine(
			[project("tpl", { id: "com.example.my-package", version: "0.1.0" })],
			{ maintained: [], lookedUp: [] },
		);
		const entry = entryFor(model, "com.example.my-package");
		expect(entry.state).toBe("local-only");
		expect(entry.issues).toEqual([{ kind: "placeholder-id" }]);
		expect(mineIssueTone(entry.issues[0])).toBe("warning");
		expect(entry.primaryAction).toEqual({
			kind: "choose-id",
			path: "/work/tpl",
		});
		expect(model.counts.issues).toBe(1);
	});

	test("takes precedence over id-taken and lint errors", () => {
		const model = deriveMine(
			[
				project(
					"tpl",
					{ id: "com.example.hello", version: "0.1.0" },
					{ lint: { errors: 2, warnings: 0 } },
				),
			],
			{
				maintained: [],
				lookedUp: [
					summary("com.example.hello", "1.0.0", {
						viewerPermission: undefined,
					}),
				],
			},
		);
		const entry = entryFor(model, "com.example.hello");
		expect(entry.idTaken).toBe(true);
		expect(entry.issues).toEqual([
			{ kind: "placeholder-id" },
			{ kind: "lint", errors: 2 },
		]);
		expect(entry.primaryAction).toEqual({
			kind: "choose-id",
			path: "/work/tpl",
		});
	});

	test("is not flagged once the caller maintains the id on the registry", () => {
		const model = deriveMine(
			[project("tpl", { id: "com.example.kept", version: "1.0.0" })],
			{ maintained: [summary("com.example.kept", "1.0.0")], lookedUp: [] },
		);
		const entry = entryFor(model, "com.example.kept");
		expect(entry.issues).toEqual([]);
		expect(entry.state).toBe("live");
	});

	test("ranks with warnings, after errors", () => {
		const model = deriveMine(
			[
				project("tpl", { id: "com.example.a", version: "0.1.0" }),
				project(
					"lint",
					{ id: "z.lint", version: "0.1.0" },
					{ lint: { errors: 1, warnings: 0 } },
				),
				project("draft", { id: "b.draft", version: "0.1.0" }),
			],
			null,
		);
		expect(model.entries.map((entry) => entry.key)).toEqual([
			"z.lint",
			"com.example.a",
			"b.draft",
		]);
	});
});

describe("disabled packages", () => {
	const registry = {
		maintained: [
			summary("pkg.off", "1.0.0", { status: PackageStatus.Disabled }),
			summary("pkg.local-off", "2.0.0", { status: PackageStatus.Disabled }),
			summary("pkg.on", "1.0.0"),
		],
		lookedUp: [],
	};
	const projects = [
		project("local", { id: "pkg.local-off", version: "2.1.0" }),
	];

	test("without a checkout are hidden unless asked for", () => {
		const model = deriveMine(projects, registry);
		expect(model.entries.map((entry) => entry.key).sort()).toEqual([
			"pkg.local-off",
			"pkg.on",
		]);
		expect(model.counts.disabled).toBe(1);
	});

	test("with a checkout always show as disabled, never as unpublished", () => {
		const local = entryFor(deriveMine(projects, registry), "pkg.local-off");
		expect(local.state).toBe("disabled");
		expect(local.registry?.id).toBe("pkg.local-off");
		expect(local.primaryAction).toEqual({
			kind: "view-releases",
			packageId: "pkg.local-off",
		});
	});

	test("show a Disabled state that leads to Releases", () => {
		const model = deriveMine(projects, registry, { includeDisabled: true });

		const remote = entryFor(model, "pkg.off");
		expect(remote.state).toBe("disabled");
		expect(remote.primaryAction).toEqual({
			kind: "view-releases",
			packageId: "pkg.off",
		});

		const local = entryFor(model, "pkg.local-off");
		expect(local.state).toBe("disabled");
		expect(local.registry?.id).toBe("pkg.local-off");
		expect(local.primaryAction).toEqual({
			kind: "view-releases",
			packageId: "pkg.local-off",
		});

		expect(model.counts.disabled).toBe(2);
		expect(
			model.entries.filter((entry) => matchesMineFilter(entry, "disabled")),
		).toHaveLength(2);
		expect(model.entries.at(-1)?.state).toBe("disabled");
	});
});

describe("mineActionTab", () => {
	test("maps each action to the workspace tab that fixes it", () => {
		expect(mineActionTab({ kind: "choose-id", path: "/p" })).toBe("manifest");
		expect(mineActionTab({ kind: "rename-id", path: "/p" })).toBe("manifest");
		expect(mineActionTab({ kind: "fix-errors", path: "/p", errors: 1 })).toBe(
			"nodes",
		);
		expect(mineActionTab({ kind: "view-review", packageId: "p" })).toBe(
			"releases",
		);
		expect(mineActionTab({ kind: "view-releases", packageId: "p" })).toBe(
			"releases",
		);
		expect(mineActionTab({ kind: "open", packageId: "p" })).toBe("overview");
	});

	test("actions that run in place or leave the workspace have no tab", () => {
		expect(mineActionTab({ kind: "reload", path: "/p" })).toBeNull();
		expect(
			mineActionTab({
				kind: "publish",
				path: "/p",
				version: null,
				first: true,
			}),
		).toBeNull();
		expect(mineActionTab({ kind: "link-folder", packageId: "p" })).toBeNull();
	});
});

describe("workspaceHeaderAction", () => {
	const model = deriveMine(
		[
			project("old", { id: "pkg.a", version: "1.0.0" }),
			project("new", { id: "pkg.a", version: "1.1.0" }),
			project(
				"lint",
				{ id: "pkg.lint", version: "0.1.0" },
				{ lint: { errors: 2, warnings: 0 } },
			),
			project("review", { id: "pkg.review", version: "1.0.0" }),
			project("live", { id: "pkg.live", version: "1.0.0" }),
		],
		{
			maintained: [
				summary("pkg.a", "1.0.0"),
				summary("pkg.review", "1.0.0", {
					status: PackageStatus.PendingReview,
				}),
				summary("pkg.live", "1.0.0"),
			],
			lookedUp: [],
		},
	);

	test("runs the newest checkout's action on it, on any tab but Releases", () => {
		const entry = entryFor(model, "pkg.a");
		const publish = {
			kind: "publish",
			path: "/work/new",
			version: "1.1.0",
			first: false,
		};
		expect(workspaceHeaderAction(entry, "/work/new/", "overview")).toEqual(
			publish,
		);
		expect(workspaceHeaderAction(entry, "/work/new", "releases")).toEqual(
			publish,
		);
	});

	test("an older checkout gets no action built from the newest one", () => {
		expect(
			workspaceHeaderAction(entryFor(model, "pkg.a"), "/work/old", "overview"),
		).toBeNull();
	});

	test("is hidden on the tab it would open", () => {
		const lint = entryFor(model, "pkg.lint");
		expect(workspaceHeaderAction(lint, "/work/lint", "overview")?.kind).toBe(
			"fix-errors",
		);
		expect(workspaceHeaderAction(lint, "/work/lint", "nodes")).toBeNull();
	});

	test("only Publish fills the Releases tab's next-release slot", () => {
		const review = entryFor(model, "pkg.review");
		expect(
			workspaceHeaderAction(review, "/work/review", "overview")?.kind,
		).toBe("view-review");
		expect(
			workspaceHeaderAction(review, "/work/review", "releases"),
		).toBeNull();
		expect(
			workspaceHeaderAction(
				entryFor(model, "pkg.lint"),
				"/work/lint",
				"releases",
			),
		).toBeNull();
	});

	test("Open is where the workspace already is", () => {
		expect(
			workspaceHeaderAction(entryFor(model, "pkg.live"), "/work/live", "nodes"),
		).toBeNull();
	});
});

describe("findWorkspaceEntry", () => {
	const model = deriveMine(
		[
			project("a", { id: "pkg.a", version: "1.0.0" }),
			project("b", { id: "pkg.a", version: "1.1.0" }),
			project("c", null),
		],
		{ maintained: [summary("pkg.remote", "1.0.0")], lookedUp: [] },
	);

	test("finds a checkout by its folder, ignoring a trailing slash", () => {
		expect(
			findWorkspaceEntry(model.entries, { project: "/work/a/" })?.key,
		).toBe("pkg.a");
		expect(findWorkspaceEntry(model.entries, { project: "/work/c" })?.key).toBe(
			"project:c",
		);
	});

	test("prefers the folder over the id", () => {
		expect(
			findWorkspaceEntry(model.entries, {
				id: "pkg.remote",
				project: "/work/b",
			})?.key,
		).toBe("pkg.a");
	});

	test("falls back to the package id without a folder", () => {
		expect(findWorkspaceEntry(model.entries, { id: "pkg.remote" })?.key).toBe(
			"pkg.remote",
		);
	});

	test("an unknown folder is not matched through its id", () => {
		expect(
			findWorkspaceEntry(model.entries, { id: "pkg.a", project: "/elsewhere" }),
		).toBeUndefined();
		expect(findWorkspaceEntry(model.entries, {})).toBeUndefined();
	});
});

describe("idsToLookUp", () => {
	test("asks only for local ids the maintainer listing missed", () => {
		expect(
			idsToLookUp(
				[
					project("a", { id: "pkg.b" }),
					project("b", { id: "pkg.a" }),
					project("c", { id: "pkg.a" }),
					project("d", { id: "pkg.known" }),
					project("e", null),
				],
				[summary("pkg.known", "1.0.0")],
			),
		).toEqual(["pkg.a", "pkg.b"]);
	});
});

describe("localSummary", () => {
	test("builds a card summary from the local manifest", () => {
		const model = deriveMine(
			[
				project("a", {
					id: "pkg.a",
					name: "Pkg A",
					version: "0.1.0",
					description: "Local package",
				}),
			],
			null,
		);
		expect(localSummary(entryFor(model, "pkg.a"))).toMatchObject({
			id: "pkg.a",
			name: "Pkg A",
			description: "Local package",
			latestVersion: "0.1.0",
			downloadCount: 0,
		});
	});
});
