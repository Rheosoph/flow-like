import { describe, expect, test } from "bun:test";
import {
	PACKAGE_WORKSPACE_PATH,
	WORKSPACE_TABS,
	packageWorkspaceHref,
	workspaceTabFromParam,
} from "./workspace-href";

function query(href: string) {
	return new URL(href, "https://app.example").searchParams;
}

describe("packageWorkspaceHref", () => {
	test("is the bare workspace path without input", () => {
		expect(packageWorkspaceHref()).toBe("/store/package-workspace");
		expect(PACKAGE_WORKSPACE_PATH).toBe("/store/package-workspace");
	});

	test("links a registry package", () => {
		expect(packageWorkspaceHref({ id: "simple-math" })).toBe(
			"/store/package-workspace?id=simple-math",
		);
	});

	test("carries project, tab and the publish banner", () => {
		const href = packageWorkspaceHref({
			id: "simple-math",
			project: "/Users/me/Git/simple math",
			tab: "listing",
			published: true,
		});
		const params = query(href);

		expect(href.startsWith(`${PACKAGE_WORKSPACE_PATH}?`)).toBe(true);
		expect(params.get("id")).toBe("simple-math");
		expect(params.get("project")).toBe("/Users/me/Git/simple math");
		expect(params.get("tab")).toBe("listing");
		expect(params.get("published")).toBe("1");
	});

	test("keeps local-only packages addressable by project", () => {
		const params = query(packageWorkspaceHref({ project: "/tmp/pkg" }));

		expect(params.has("id")).toBe(false);
		expect(params.get("project")).toBe("/tmp/pkg");
	});

	test("omits the default tab and a false published flag", () => {
		expect(
			packageWorkspaceHref({ id: "a", tab: "overview", published: false }),
		).toBe("/store/package-workspace?id=a");
	});

	test("never produces a /store/packages/ path (Pages _redirects trap)", () => {
		const hrefs = [
			packageWorkspaceHref(),
			packageWorkspaceHref({ id: "workspace" }),
			packageWorkspaceHref({
				id: "../packages/x",
				project: "/store/packages/y",
			}),
			...WORKSPACE_TABS.map((tab) =>
				packageWorkspaceHref({ id: "x", project: "/p", tab, published: true }),
			),
		];

		for (const href of hrefs) {
			expect(href.startsWith("/store/packages/")).toBe(false);
			expect(new URL(href, "https://app.example").pathname).toBe(
				PACKAGE_WORKSPACE_PATH,
			);
		}
	});
});

describe("workspaceTabFromParam", () => {
	test("accepts every workspace tab", () => {
		for (const tab of WORKSPACE_TABS) {
			expect(workspaceTabFromParam(tab)).toBe(tab);
		}
	});

	test("falls back to overview", () => {
		expect(workspaceTabFromParam(null)).toBe("overview");
		expect(workspaceTabFromParam(undefined)).toBe("overview");
		expect(workspaceTabFromParam("")).toBe("overview");
		expect(workspaceTabFromParam("metadata")).toBe("overview");
		expect(workspaceTabFromParam("Listing")).toBe("overview");
	});
});
