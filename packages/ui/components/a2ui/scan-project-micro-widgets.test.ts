import { describe, expect, mock, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPage, PageListItem } from "../../state/backend-state/page-state";
import { scanProjectMicroWidgets } from "./scan-project-micro-widgets";
import type { MicroWidgetInstanceComponent, SurfaceComponent } from "./types";

const contract: WidgetContract = {
	contractVersion: 1,
	id: "chart",
	inputs: {},
	events: {},
	queries: {},
};

function installedWidget(
	packageId: string,
	widgetId: string,
): AppPackageWidget {
	return {
		packageId,
		packageName: packageId,
		packageVersion: "1.0.0",
		bundleHash: "new-hash",
		widget: {
			id: widgetId,
			name: widgetId,
			description: "",
			icon: null,
			thumbnail: null,
			keywords: [],
			contract: { ...contract, id: widgetId },
		},
	};
}

const installed = [
	installedWidget("charts", "chart"),
	installedWidget("charts", "legend"),
	installedWidget("learning", "quiz"),
];

function instance(
	id: string,
	options: Partial<MicroWidgetInstanceComponent> = {},
): SurfaceComponent {
	return {
		id,
		component: {
			id,
			type: "microWidgetInstance",
			instanceId: `${id}-instance`,
			packageId: "charts",
			widgetId: "chart",
			packageVersion: "1.0.0",
			bundleHash: "old-hash",
			contract,
			props: {},
			...options,
		},
	};
}

function page(
	id: string,
	boardId: string,
	components: SurfaceComponent[],
): IPage {
	return {
		id,
		name: `Page ${id}`,
		boardId,
		content: [],
		layoutType: "freeform",
		createdAt: "2020-01-01T00:00:00.000Z",
		updatedAt: "2020-01-01T00:00:00.000Z",
		components,
	};
}

function stateFor(pages: IPage[]) {
	return {
		getPagesAuthoritative: mock(
			async (_appId: string): Promise<PageListItem[]> =>
				pages.map((page) => ({
					appId: "app",
					pageId: page.id,
					boardId: page.boardId,
					name: page.name,
				})),
		),
		getPageAuthoritative: mock(
			async (_appId: string, pageId: string, _boardId?: string) => {
				const found = pages.find((page) => page.id === pageId);
				if (!found) throw new Error(`Missing ${pageId}`);
				return found;
			},
		),
		updatePage: mock(async (_appId: string, _page: IPage) => {}),
	};
}

describe("scanProjectMicroWidgets", () => {
	test("counts each package separately across boards and deduplicates page listings", async () => {
		const first = page("first", "board-a", [
			instance("first-chart"),
			instance("first-legend", { widgetId: "legend" }),
			instance("first-quiz", { packageId: "learning", widgetId: "quiz" }),
		]);
		const second = page("second", "board-b", [
			instance("second-chart"),
			instance("second-quiz", {
				packageId: "learning",
				widgetId: "quiz",
				bundleHash: "new-hash",
			}),
		]);
		const state = stateFor([first, first, second]);
		const original = structuredClone([first, second]);

		const result = await scanProjectMicroWidgets(state, "app", installed);

		expect(result.byPackage).toEqual(
			new Map([
				["charts", { widgets: 3, pages: 2 }],
				["learning", { widgets: 1, pages: 1 }],
			]),
		);
		expect(result.failures).toEqual([]);
		expect(state.getPagesAuthoritative.mock.calls).toEqual([["app"]]);
		expect(state.getPageAuthoritative.mock.calls).toEqual([
			["app", "first", "board-a"],
			["app", "second", "board-b"],
		]);
		expect(state.updatePage).not.toHaveBeenCalled();
		expect([first, second]).toEqual(original);
	});

	test("includes inline content and widgets embedded in page references", async () => {
		const saved = page("embedded", "board", [instance("component")]);
		saved.content = [
			{ Component: instance("inline") },
			{ ComponentRef: "component" },
		];
		saved.widgetRefs = {
			instance: {
				id: "embedded-widget",
				name: "Embedded widget",
				rootComponentId: "embedded-chart",
				components: [instance("embedded-chart")],
				tags: [],
				createdAt: saved.createdAt,
				updatedAt: saved.updatedAt,
			},
		};

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed,
		);

		expect(result.byPackage.get("charts")).toEqual({ widgets: 3, pages: 1 });
	});

	test("ignores current builds and uninstalled packages", async () => {
		const saved = page("current", "board", [
			instance("current", { bundleHash: "new-hash" }),
			instance("unknown-package", { packageId: "unknown" }),
		]);

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed,
			new Map([["unknown", "2.0.0"]]),
		);

		expect(result.byPackage.size).toBe(0);
		expect(result.failures).toEqual([]);
	});

	test("reports unavailable widgets once per page while keeping counts of available widgets", async () => {
		const saved = page("missing-widget", "board", [
			instance("available"),
			instance("missing", { widgetId: "removed" }),
			instance("unrelated", { packageId: "unknown" }),
		]);
		saved.content = [
			{ Component: instance("inline-missing", { widgetId: "removed" }) },
		];
		const result = await scanProjectMicroWidgets(
			stateFor([saved, saved]),
			"app",
			installed,
		);

		expect(result.byPackage.get("charts")).toEqual({ widgets: 1, pages: 1 });
		expect(result.failures).toEqual([
			{
				pageId: saved.id,
				name: saved.name,
				error: new Error(
					"These widgets are unavailable in the installed packages: charts/removed.",
				),
			},
		]);
	});

	test("reports unavailable widgets even when no available widgets need updating", async () => {
		const saved = page("unavailable", "board", [
			instance("removed", { widgetId: "removed" }),
			instance("current", { bundleHash: "new-hash" }),
		]);

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed,
		);

		expect(result.byPackage.size).toBe(0);
		expect(result.failures.map(({ pageId }) => pageId)).toEqual([saved.id]);
	});

	test("counts a newer release before installation without comparing its hash to the installed build", async () => {
		const saved = page("release", "board", [
			instance("on-installed-build", { bundleHash: "new-hash" }),
			instance("on-latest-release", {
				packageVersion: "2.0.0",
				bundleHash: "latest-release-hash",
			}),
			instance("removed-widget", { widgetId: "removed" }),
			instance("other-package", { packageId: "learning", widgetId: "quiz" }),
		]);

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed,
			new Map([["charts", "2.0.0"]]),
		);

		expect(result.byPackage).toEqual(
			new Map([
				["charts", { widgets: 1, pages: 1 }],
				["learning", { widgets: 1, pages: 1 }],
			]),
		);
		expect(result.failures).toHaveLength(1);
		expect(result.failures[0].pageId).toBe(saved.id);
	});

	test("detects local rebuilds when the latest release is already installed", async () => {
		const saved = page("rebuilt", "board", [instance("local")]);

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed,
			new Map([["charts", "1.0.0"]]),
		);

		expect(result.byPackage.get("charts")).toEqual({ widgets: 1, pages: 1 });
	});

	test("compares versions on hosts without bundle hashes", async () => {
		const saved = page("versions", "board", [
			instance("current", { bundleHash: "unavailable-to-host" }),
			instance("old-version", { packageVersion: "0.9.0" }),
		]);

		const result = await scanProjectMicroWidgets(
			stateFor([saved]),
			"app",
			installed.map((entry) => ({ ...entry, bundleHash: undefined })),
		);

		expect(result.byPackage.get("charts")).toEqual({ widgets: 1, pages: 1 });
	});

	test("keeps counts from readable pages and reports read failures once", async () => {
		const unreadable = page("unreadable", "board-a", []);
		unreadable.name = "";
		const good = page("good", "board-b", [instance("good-chart")]);
		const state = stateFor([unreadable, good, unreadable]);
		const error = new Error("Page unavailable");
		state.getPageAuthoritative.mockImplementation(async (_appId, pageId) => {
			if (pageId === unreadable.id) throw error;
			return good;
		});

		const result = await scanProjectMicroWidgets(state, "app", installed);

		expect(result.byPackage.get("charts")).toEqual({ widgets: 1, pages: 1 });
		expect(result.failures).toEqual([
			{ pageId: unreadable.id, name: unreadable.id, error },
		]);
		expect(state.getPageAuthoritative).toHaveBeenCalledTimes(2);
		expect(state.updatePage).not.toHaveBeenCalled();
	});

	test("propagates page inventory failures", async () => {
		const state = stateFor([]);
		const error = new Error("Hub unavailable");
		state.getPagesAuthoritative.mockImplementation(async () => {
			throw error;
		});

		await expect(scanProjectMicroWidgets(state, "app", installed)).rejects.toBe(
			error,
		);
		expect(state.getPageAuthoritative).not.toHaveBeenCalled();
		expect(state.updatePage).not.toHaveBeenCalled();
	});
});
