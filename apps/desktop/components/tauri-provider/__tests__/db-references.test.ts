import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	apiGet: vi.fn(),
	apiPost: vi.fn(),
	apiDelete: vi.fn(),
	apiPut: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({ fetcher: mocks.fetcher }));
vi.mock("../../../../web/lib/web-states/api-utils", () => ({
	apiGet: mocks.apiGet,
	apiPost: mocks.apiPost,
	apiDelete: mocks.apiDelete,
	apiPut: mocks.apiPut,
}));

import { WebDatabaseState } from "../../../../web/lib/web-states/database-state";
import { DatabaseState } from "../db-state";

const auth = { user: { access_token: "test-token" } };
const profile = { id: "profile-1" };
const selector = { branch: "experiment/a b", version: 7 };
const web = new WebDatabaseState({ auth } as never);
const online = new DatabaseState({
	isOffline: async () => false,
	auth,
	profile,
} as never);
const offline = new DatabaseState({ isOffline: async () => true } as never);

beforeEach(() => vi.resetAllMocks());

describe("database references across transports", () => {
	test("keeps scope, pagination and a pinned reference in hosted reads", async () => {
		await web.listItems("app", "my table", 10, 25, true, selector);
		await online.listItems("app", "my table", 10, 25, true, selector);
		const webUrl = new URL(
			mocks.apiGet.mock.calls[0][0],
			"https://example.test/",
		);
		const desktopUrl = new URL(
			mocks.fetcher.mock.calls[0][1],
			"https://example.test/",
		);
		for (const url of [webUrl, desktopUrl]) {
			expect(url.pathname).toBe("/apps/app/db/my%20table");
			expect(Object.fromEntries(url.searchParams)).toEqual({
				scope: "user",
				branch: "experiment/a b",
				version: "7",
				offset: "10",
				limit: "25",
			});
		}
	});

	test("passes the selected writable branch to row deletion", async () => {
		const branch = { branch: "experiment" };
		await offline.removeItems("app", "rows", "id = 42", true, branch);
		await web.removeItems("app", "rows", "id = 42", true, branch);
		expect(mocks.invoke).toHaveBeenCalledWith("db_delete", {
			appId: "app",
			tableName: "rows",
			query: "id = 42",
			userScoped: true,
			selector: branch,
		});
		expect(mocks.apiDelete).toHaveBeenCalledWith(
			"apps/app/db/rows?scope=user&branch=experiment",
			auth,
			{ query: "id = 42" },
		);
	});

	test("does not hide failed historical reads as empty tables", async () => {
		const error = new Error("Version no longer exists");
		mocks.apiGet.mockRejectedValue(error);
		mocks.apiPost.mockRejectedValue(error);
		await expect(
			web.listItems("app", "rows", 0, 25, false, selector),
		).rejects.toThrow(error);
		await expect(
			web.countItems("app", "rows", false, selector),
		).rejects.toThrow(error);
		await expect(
			web.queryItems(
				"app",
				"rows",
				{ filter: "id = 1" },
				0,
				25,
				false,
				selector,
			),
		).rejects.toThrow(error);
	});

	test("sends tag movement as an explicit action on a resolved source", async () => {
		const action = { action: "update_tag", name: "training" } as const;
		await offline.databaseAction("app", "rows", action, false, selector);
		await online.databaseAction("app", "rows", action, false, selector);
		expect(mocks.invoke).toHaveBeenCalledWith("db_reference_action", {
			appId: "app",
			tableName: "rows",
			action,
			userScoped: false,
			selector,
		});
		expect(mocks.fetcher).toHaveBeenCalledWith(
			profile,
			"apps/app/db/rows/references?branch=experiment%2Fa+b&version=7",
			{ method: "POST", body: JSON.stringify(action) },
			auth,
		);
	});

	test("keeps baseline and candidate selectors distinct in comparison", async () => {
		const target = { tag: "training" };
		await web.databaseCompare("app", "rows", target, "id", 20, true, selector);
		await offline.databaseCompare(
			"app",
			"rows",
			target,
			"id",
			20,
			true,
			selector,
		);
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"apps/app/db/rows/compare?scope=user&branch=experiment%2Fa+b&version=7",
			{ other: target, key: "id", limit: 20 },
			auth,
		);
		expect(mocks.invoke).toHaveBeenCalledWith("db_compare", {
			appId: "app",
			tableName: "rows",
			other: target,
			key: "id",
			limit: 20,
			userScoped: true,
			selector,
		});
	});
});
