import type { IWidget } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), fetcher: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher }));

import { WidgetState } from "../../components/tauri-provider/widget-state";
import { ApiResponseError } from "../api-error";

function backend(localOnly = false) {
	return {
		isLocalOnly: vi.fn().mockResolvedValue(localOnly),
		profile: { hub: "hub.example" },
		auth: { isAuthenticated: true, user: { access_token: "token" } },
	};
}

function widget(overrides: Partial<IWidget> = {}): IWidget {
	return {
		id: "widget-1",
		name: "Current widget",
		rootComponentId: "root",
		components: [],
		dataModel: [],
		customizationOptions: [],
		tags: [],
		version: [1, 0, 0],
		createdAt: "2026-08-01T00:00:00Z",
		updatedAt: "2026-08-01T00:00:00Z",
		...overrides,
	};
}

describe("native widget synchronization", () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test("loads a hosted inventory before the app exists on a new device", async () => {
		const inventory = [["app-1", "widget-1", { name: "Current widget" }]];
		mocks.invoke.mockRejectedValue(new Error("App not installed"));
		mocks.fetcher.mockResolvedValue(inventory);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidgets("app-1")).resolves.toBe(inventory);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("does not re-upload deleted cached widgets when listing a hosted app", async () => {
		mocks.invoke.mockResolvedValue([widget({ name: "Deleted elsewhere" })]);
		mocks.fetcher.mockResolvedValue([]);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidgets("app-1")).resolves.toEqual([]);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(mocks.fetcher.mock.calls[0][2]).toEqual({ method: "GET" });
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("uses the server definition despite a newer device clock or removed components", async () => {
		const local = widget({
			name: "Stale cache",
			updatedAt: "2099-01-01T00:00:00Z",
			components: [
				{
					id: "root",
					component: {
						id: "root",
						type: "text",
						content: { literalString: "Old" },
					},
				},
			],
		});
		const remote = widget();
		mocks.invoke.mockResolvedValue(local);
		mocks.fetcher.mockResolvedValue(remote);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidget("app-1", "widget-1")).resolves.toBe(remote);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).toHaveBeenCalledWith("update_widget", {
			appId: "app-1",
			widget: remote,
		});
	});

	test("requests and caches the exact hosted snapshot without overwriting the working copy", async () => {
		const remote = widget({ version: [3, 2, 1] });
		mocks.invoke.mockRejectedValueOnce(new Error("Snapshot not cached"));
		mocks.fetcher.mockResolvedValue(remote);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidget("app-1", "widget-1", [3, 2, 1])).resolves.toBe(
			remote,
		);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/widgets/widget-1?version=3_2_1",
		);
		expect(mocks.invoke).toHaveBeenLastCalledWith("cache_widget_version", {
			appId: "app-1",
			widget: remote,
		});
		expect(
			mocks.invoke.mock.calls.some(([command]) => command === "update_widget"),
		).toBe(false);
	});

	test("uses the cached exact version when the phone loses its connection", async () => {
		const local = widget({ version: [3, 2, 1] });
		mocks.invoke.mockResolvedValue(local);
		mocks.fetcher.mockRejectedValue(new Error("Network unavailable"));
		const state = new WidgetState(backend() as never);

		await expect(state.getWidget("app-1", "widget-1", [3, 2, 1])).resolves.toBe(
			local,
		);
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).toHaveBeenCalledWith("get_widget", {
			appId: "app-1",
			widgetId: "widget-1",
			version: [3, 2, 1],
		});
	});

	test("does not substitute a cached widget for a server deletion", async () => {
		mocks.invoke.mockResolvedValue(widget());
		const error = new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "Widget not found",
		});
		mocks.fetcher.mockRejectedValue(error);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidget("app-1", "widget-1")).rejects.toBe(error);
	});

	test("loads locally cached widgets for explicitly local apps", async () => {
		const local = widget();
		mocks.invoke
			.mockResolvedValueOnce([local])
			.mockResolvedValueOnce(undefined);
		const state = new WidgetState(backend(true) as never);

		await expect(state.getWidgets("app-1")).resolves.toEqual([
			["app-1", "widget-1", expect.objectContaining({ name: local.name })],
		]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("reads version history from the hub on a device with no local snapshots", async () => {
		mocks.fetcher.mockResolvedValue([
			[3, 2, 1],
			[1, 0, 0],
		]);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidgetVersions("app-1", "widget-1")).resolves.toEqual(
			[
				[3, 2, 1],
				[1, 0, 0],
			],
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("publishes on the hub and caches the snapshot returned by its version", async () => {
		const published = widget({ version: [3, 2, 1] });
		mocks.fetcher.mockResolvedValueOnce([3, 2, 1]).mockResolvedValue(published);
		const state = new WidgetState(backend() as never);

		await expect(
			state.createWidgetVersion("app-1", "widget-1", "Patch"),
		).resolves.toEqual([3, 2, 1]);
		expect(mocks.fetcher.mock.calls[0]).toEqual([
			{ hub: "hub.example" },
			"apps/app-1/widgets/widget-1/versions",
			{ method: "POST", body: JSON.stringify({ version_type: "Patch" }) },
			expect.objectContaining({ isAuthenticated: true }),
		]);
		expect(mocks.invoke).toHaveBeenCalledWith("cache_widget_version", {
			appId: "app-1",
			widget: published,
		});
		expect(
			mocks.invoke.mock.calls.some(
				([command]) => command === "create_widget_version",
			),
		).toBe(false);
		expect(
			mocks.fetcher.mock.calls.some(
				([, , options]) => options.method === "PUT",
			),
		).toBe(false);
	});

	test("surfaces a failed hosted publication without creating a device-only version", async () => {
		const error = new Error("Network unavailable");
		mocks.fetcher.mockRejectedValue(error);
		const state = new WidgetState(backend() as never);

		await expect(
			state.createWidgetVersion("app-1", "widget-1", "Patch"),
		).rejects.toBe(error);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("continues publishing local-only widgets on the device", async () => {
		mocks.invoke.mockResolvedValue([1, 0, 1]);
		const state = new WidgetState(backend(true) as never);

		await expect(
			state.createWidgetVersion("app-1", "widget-1", "Patch"),
		).resolves.toEqual([1, 0, 1]);
		expect(mocks.invoke).toHaveBeenCalledWith("create_widget_version", {
			appId: "app-1",
			widgetId: "widget-1",
			versionType: "Patch",
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("does not replace the cache with an edit rejected by the hub", async () => {
		const error = new Error("Update rejected");
		mocks.fetcher.mockRejectedValue(error);
		const state = new WidgetState(backend() as never);

		await expect(state.updateWidget("app-1", widget())).rejects.toBe(error);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("does not report an unsynced hosted edit as saved before sign-in", async () => {
		const session = backend();
		session.auth.isAuthenticated = false;
		const state = new WidgetState(session as never);

		await expect(state.updateWidget("app-1", widget())).rejects.toThrow(
			"authenticated hub session",
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("does not delete the cache when the hosted deletion fails", async () => {
		const failure = new Error("Delete rejected");
		mocks.fetcher.mockRejectedValue(failure);
		const state = new WidgetState(backend() as never);

		await expect(state.deleteWidget("app-1", "widget-1")).rejects.toBe(failure);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("fetches every definition before replacing the native execution inventory", async () => {
		const first = widget();
		const second = widget({ id: "widget-2" });
		let finishSecond!: (value: IWidget) => void;
		mocks.fetcher
			.mockResolvedValueOnce([
				["app-1", "widget-1"],
				["app-1", "widget-2"],
			])
			.mockResolvedValueOnce(first)
			.mockReturnValueOnce(
				new Promise<IWidget>((resolve) => {
					finishSecond = resolve;
				}),
			);
		const state = new WidgetState(backend() as never);
		const sync = state.syncWidgetsForExecution("app-1");
		await vi.waitFor(() => expect(mocks.fetcher).toHaveBeenCalledTimes(3));
		expect(mocks.invoke).not.toHaveBeenCalled();
		finishSecond(second);
		await sync;

		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("cache_widgets", {
			appId: "app-1",
			widgets: [first, second],
		});
	});

	test("clears the native execution inventory when all remote widgets were deleted", async () => {
		mocks.fetcher.mockResolvedValue([]);
		const state = new WidgetState(backend() as never);
		await state.syncWidgetsForExecution("app-1");

		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("cache_widgets", {
			appId: "app-1",
			widgets: [],
		});
	});

	test("fails execution preparation instead of accepting partially fetched widgets", async () => {
		const failure = new Error("Widget download failed");
		mocks.fetcher
			.mockResolvedValueOnce([["app-1", "widget-1"]])
			.mockRejectedValueOnce(failure);
		const state = new WidgetState(backend() as never);

		await expect(state.syncWidgetsForExecution("app-1")).rejects.toBe(failure);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("keeps local-only workflow preparation independent of the hub", async () => {
		const state = new WidgetState(backend(true) as never);
		await state.syncWidgetsForExecution("app-1");
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("reads hosted widget metadata without preferring an old local name", async () => {
		const metadata = { name: "Renamed on another device" };
		mocks.fetcher.mockResolvedValue(metadata);
		const state = new WidgetState(backend() as never);

		await expect(state.getWidgetMeta("app-1", "widget-1", "de")).resolves.toBe(
			metadata,
		);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/meta?language=de&widget_id=widget-1",
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("saves widget metadata on the hub before caching it", async () => {
		const metadata = { name: "Renamed" };
		const state = new WidgetState(backend() as never);

		await state.pushWidgetMeta("app-1", "widget-1", metadata as never, "de");
		expect(mocks.fetcher.mock.calls[0][2]).toEqual({
			method: "PUT",
			body: JSON.stringify(metadata),
		});
		expect(mocks.invoke).toHaveBeenCalledWith("push_widget_meta", {
			appId: "app-1",
			widgetId: "widget-1",
			metadata,
			language: "de",
		});
		expect(mocks.fetcher.mock.invocationCallOrder[0]).toBeLessThan(
			mocks.invoke.mock.invocationCallOrder[0],
		);
	});
});
