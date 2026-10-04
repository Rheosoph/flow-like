import { describe, expect, mock, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPage, PageListItem } from "../../state/backend-state/page-state";
import { reloadProjectMicroWidgets } from "./reload-project-micro-widgets";

const oldContract: WidgetContract = {
	contractVersion: 1,
	id: "chart",
	inputs: { title: { type: "string", default: "Before" } },
	events: {},
	queries: {},
};
const installed: AppPackageWidget[] = [
	{
		packageId: "charts",
		packageName: "Charts",
		packageVersion: "1.0.0",
		bundleHash: "new-hash",
		widget: {
			id: "chart",
			name: "Chart",
			description: "",
			icon: null,
			thumbnail: null,
			keywords: [],
			contract: {
				...oldContract,
				inputs: { title: { type: "string", default: "After" } },
			},
		},
	},
];

function page(id: string, boardId: string, hash = "old-hash"): IPage {
	return {
		id,
		name: `Page ${id}`,
		boardId,
		content: [],
		layoutType: "freeform",
		createdAt: "2020-01-01T00:00:00.000Z",
		updatedAt: "2020-01-01T00:00:00.000Z",
		components: [
			{
				id: `${id}-chart`,
				component: {
					id: `${id}-chart`,
					type: "microWidgetInstance",
					instanceId: `${id}-instance`,
					packageId: "charts",
					widgetId: "chart",
					packageVersion: "1.0.0",
					bundleHash: hash,
					contract: oldContract,
					props: { title: "Before" },
				},
			},
		],
	};
}

function row(value: IPage): PageListItem {
	return {
		appId: "app",
		pageId: value.id,
		boardId: value.boardId,
		name: value.name,
	};
}

function stateFor(pages: IPage[]) {
	return {
		getPagesAuthoritative: mock(async (_appId: string) => pages.map(row)),
		getPageAuthoritative: mock(
			async (_appId: string, pageId: string, _boardId?: string) => {
				const found = pages.find((item) => item.id === pageId);
				if (!found) throw new Error(`Missing ${pageId}`);
				return found;
			},
		),
		updatePage: mock(async (_appId: string, _page: IPage) => {}),
	};
}

describe("reloadProjectMicroWidgets", () => {
	test("updates drafts across boards, deduplicates pages, and preserves the active working copy", async () => {
		const current = page("current", "board-a");
		const first = page("first", "board-a");
		const second = page("second", "board-b");
		const state = stateFor([current, first, second, first]);
		const before = Date.now();

		const result = await reloadProjectMicroWidgets(state, "app", installed, {
			skipPageId: "current",
		});

		expect(state.getPagesAuthoritative.mock.calls).toEqual([["app"]]);
		expect(state.getPageAuthoritative.mock.calls).toEqual([
			["app", "first", "board-a"],
			["app", "second", "board-b"],
		]);
		expect(result).toEqual({
			count: 2,
			pages: 2,
			boardIds: ["board-a", "board-b"],
			report: {
				converted: [],
				defaulted: ["title"],
				reset: [],
				removed: [],
				undeclaredEvents: [],
			},
			failures: [],
		});
		expect(state.updatePage).toHaveBeenCalledTimes(2);
		for (const [appId, saved] of state.updatePage.mock.calls) {
			expect(appId).toBe("app");
			expect(Date.parse(saved.updatedAt)).toBeGreaterThanOrEqual(before);
			expect(saved.components[0].component).toMatchObject({
				bundleHash: "new-hash",
				props: { title: "After" },
			});
		}
		expect(current.components[0].component).toMatchObject({
			bundleHash: "old-hash",
		});
		expect(first.updatedAt).toBe("2020-01-01T00:00:00.000Z");
	});

	test("saves only changed pages and fills a missing board id from the inventory", async () => {
		const stale = page("stale", "board-a");
		const current = page("current", "board-a", "new-hash");
		const empty = { ...page("empty", "board-b"), components: [] };
		const state = stateFor([stale, current, empty]);
		state.getPageAuthoritative.mockImplementation(async (_appId, pageId) => {
			if (pageId === "stale") return { ...stale, boardId: undefined };
			return pageId === "current" ? current : empty;
		});

		const result = await reloadProjectMicroWidgets(state, "app", installed);

		expect(result.count).toBe(1);
		expect(result.pages).toBe(1);
		expect(result.boardIds).toEqual(["board-a"]);
		expect(state.updatePage).toHaveBeenCalledTimes(1);
		expect(state.updatePage.mock.calls[0][1].boardId).toBe("board-a");
	});

	test("continues after read and save failures and counts only successful saves", async () => {
		const unreadable = page("unreadable", "board-a");
		const unsavable = page("unsavable", "board-b");
		const good = page("good", "board-c");
		const state = stateFor([unreadable, unsavable, good]);
		const readError = new Error("Read failed");
		const saveError = new Error("Write failed");
		state.getPageAuthoritative.mockImplementation(async (_appId, pageId) => {
			if (pageId === "unreadable") throw readError;
			return pageId === "unsavable" ? unsavable : good;
		});
		state.updatePage.mockImplementation(async (_appId, value) => {
			if (value.id === "unsavable") throw saveError;
		});

		const result = await reloadProjectMicroWidgets(state, "app", installed);

		expect(result.count).toBe(1);
		expect(result.pages).toBe(1);
		expect(result.boardIds).toEqual(["board-c"]);
		expect(result.failures).toEqual([
			{ pageId: unreadable.id, name: unreadable.name, error: readError },
			{ pageId: unsavable.id, name: unsavable.name, error: saveError },
		]);
		expect(state.updatePage).toHaveBeenCalledTimes(2);
	});

	test("updates available widgets and reports unavailable widget definitions on the same page", async () => {
		const mixed = page("mixed", "board-a");
		const missing = page("removed-widget", "board-a").components[0];
		if (missing.component.type !== "microWidgetInstance")
			throw new Error("Invalid fixture");
		missing.component.widgetId = "removed";
		mixed.components.push(missing);
		mixed.content.push({ Component: structuredClone(missing) });
		const other = page("other", "board-b");
		const state = stateFor([mixed, other]);

		const result = await reloadProjectMicroWidgets(state, "app", installed);

		expect(result.count).toBe(2);
		expect(result.pages).toBe(2);
		expect(result.boardIds).toEqual(["board-a", "board-b"]);
		expect(result.failures).toEqual([
			{
				pageId: mixed.id,
				name: mixed.name,
				error: new Error(
					"These widgets are unavailable in the installed packages: charts/removed.",
				),
			},
		]);
		expect(state.updatePage).toHaveBeenCalledTimes(2);
		const saved = state.updatePage.mock.calls[0][1];
		expect(saved.components[0].component).toMatchObject({
			bundleHash: "new-hash",
		});
		expect(saved.components[1]).toBe(missing);
		expect(saved.content).toBe(mixed.content);
	});

	test("reports pages containing only unavailable widgets without writing or flagging other packages", async () => {
		const missing = page("missing", "board-a");
		const unrelated = page("unrelated", "board-b");
		const component = missing.components[0].component;
		const otherComponent = unrelated.components[0].component;
		if (
			component.type !== "microWidgetInstance" ||
			otherComponent.type !== "microWidgetInstance"
		)
			throw new Error("Invalid fixture");
		component.widgetId = "removed";
		otherComponent.packageId = "uninstalled-package";
		const state = stateFor([missing, unrelated]);

		const result = await reloadProjectMicroWidgets(state, "app", installed);

		expect(result.count).toBe(0);
		expect(result.pages).toBe(0);
		expect(result.failures.map(({ pageId }) => pageId)).toEqual([missing.id]);
		expect(state.updatePage).not.toHaveBeenCalled();
	});

	test("does not include failed migrations in the report", async () => {
		const unsavable = page("unsavable", "board-a");
		const good = page("good", "board-b");
		const widget = good.components[0].component;
		if (widget.type !== "microWidgetInstance")
			throw new Error("Invalid fixture");
		widget.props = { title: "Custom title" };
		const state = stateFor([unsavable, good]);
		state.updatePage.mockImplementation(async (_appId, value) => {
			if (value.id === "unsavable") throw new Error("Write failed");
		});

		const result = await reloadProjectMicroWidgets(state, "app", installed);

		expect(result.count).toBe(1);
		expect(result.report.defaulted).toEqual([]);
	});

	test("propagates inventory failures without claiming an empty project", async () => {
		const state = stateFor([page("first", "board-a")]);
		const error = new Error("Hub unavailable");
		state.getPagesAuthoritative.mockImplementation(async () => {
			throw error;
		});

		await expect(
			reloadProjectMicroWidgets(state, "app", installed),
		).rejects.toBe(error);
		expect(state.getPageAuthoritative).not.toHaveBeenCalled();
		expect(state.updatePage).not.toHaveBeenCalled();
	});

	test("stops before writing a deferred read when the editor changes", async () => {
		const first = page("first", "board-a");
		const pending = page("pending", "board-b");
		const last = page("last", "board-c");
		const current = page("current", "board-d");
		const state = stateFor([first, pending, last, pending, current]);
		let canContinue = true;
		let releaseRead = (_value: IPage) => {};
		let readStarted = () => {};
		const started = new Promise<void>((resolve) => {
			readStarted = resolve;
		});
		const heldRead = new Promise<IPage>((resolve) => {
			releaseRead = resolve;
		});
		state.getPageAuthoritative.mockImplementation(async (_appId, pageId) => {
			if (pageId === "first") return first;
			readStarted();
			return heldRead;
		});

		const running = reloadProjectMicroWidgets(state, "app", installed, {
			skipPageId: "current",
			shouldContinue: () => canContinue,
		});
		await started;
		canContinue = false;
		releaseRead(pending);
		const result = await running;

		expect(result.count).toBe(1);
		expect(result.pages).toBe(1);
		expect(result.boardIds).toEqual(["board-a"]);
		expect(
			state.getPageAuthoritative.mock.calls.map((args) => args[1]),
		).toEqual(["first", "pending"]);
		expect(state.updatePage).toHaveBeenCalledTimes(1);
		expect(result.failures.map(({ pageId }) => pageId)).toEqual([
			"pending",
			"last",
		]);
		for (const { error } of result.failures) {
			expect(error).toBeInstanceOf(Error);
			expect((error as Error).message).toBe(
				"The project view changed during the update. Run Update all again.",
			);
		}
	});

	test("reports every remaining page without reading when already cancelled", async () => {
		const state = stateFor([page("first", "board-a"), page("last", "board-b")]);
		const result = await reloadProjectMicroWidgets(state, "app", installed, {
			shouldContinue: () => false,
		});

		expect(result.count).toBe(0);
		expect(result.failures.map(({ pageId }) => pageId)).toEqual([
			"first",
			"last",
		]);
		expect(state.getPageAuthoritative).not.toHaveBeenCalled();
		expect(state.updatePage).not.toHaveBeenCalled();
	});
});
