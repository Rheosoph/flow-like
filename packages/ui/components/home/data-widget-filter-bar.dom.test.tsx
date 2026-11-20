import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
	CrossAppSqlPayload,
	CrossAppSqlResult,
	SqlQueryDescription,
} from "../../state/backend-state/query-state";
import { HomeDataFilterBar } from "./data-widget-filter-bar";
import {
	type HomeDataConfig,
	normalizeHomeDataConfig,
} from "./home-data-query";
import { useHomeData } from "./use-home-data";
import { useHomeParamOverrides } from "./use-home-param-overrides";

const fixture = vi.hoisted(() => {
	const description: SqlQueryDescription = {
		columns: [],
		parameters: [
			{
				name: "region",
				data_type: "Utf8",
				bindings: [{ table: "crm.orders", column: "region", operator: "eq" }],
			},
		],
	};
	return {
		store: new Map<string, Record<string, unknown>>(),
		execute: vi.fn<(p: CrossAppSqlPayload) => Promise<CrossAppSqlResult>>(),
		describe: vi.fn(async () => description),
		options: vi.fn(async () => ({ values: ["EU", "US"], truncated: false })),
		backend: {
			profile: { id: "profile" },
			dbState: {},
			graphState: {},
			queryState: {} as Record<string, unknown>,
		},
		auth: { user: { profile: { sub: "viewer" } } },
	};
});
fixture.backend.queryState = {
	executeCrossAppSql: fixture.execute,
	describeCrossAppSql: fixture.describe,
	crossAppSqlParameterOptions: fixture.options,
};
vi.mock("../../state/backend-state", () => ({
	useBackend: () => fixture.backend,
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => fixture.auth }));
vi.mock("../../db/home-widget-params-db", () => ({
	homeWidgetParamsKey: (viewer: string | undefined, id: string) =>
		`${viewer}:${id}`,
	readHomeWidgetParams: async (key: string) => fixture.store.get(key) ?? {},
	writeHomeWidgetParams: async (
		key: string,
		values: Record<string, unknown>,
	) => {
		if (Object.keys(values).length) fixture.store.set(key, values);
		else fixture.store.delete(key);
	},
}));

const config: HomeDataConfig = normalizeHomeDataConfig({
	sourceKind: "sql",
	visualization: "table",
	sql: "SELECT region FROM crm.orders WHERE region = $region",
	sources: [{ alias: "crm", appId: "app-crm", scope: "project" }],
	queryParams: { region: "EU" },
	paramControls: { region: { interactive: true, label: "Area" } },
});

function Harness() {
	const { overrides, loaded, set, reset } = useHomeParamOverrides("widget-1");
	useHomeData(config, loaded, overrides);
	return (
		<HomeDataFilterBar
			config={config}
			overrides={overrides}
			onChange={set}
			onReset={reset}
		/>
	);
}

let root: Root;
let container: HTMLDivElement;

const flush = async () => act(async () => vi.advanceTimersByTimeAsync(300));
const select = () => container.querySelector("select") as HTMLSelectElement;
const lastParams = () =>
	fixture.execute.mock.calls.at(-1)?.[0].params as Record<string, unknown>;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers();
	fixture.store.clear();
	fixture.execute.mockReset();
	fixture.execute.mockImplementation(async (payload) => ({
		columns: [],
		rows: [],
		row_count: 0,
		truncated: false,
		tables: [],
		computed_at: Date.now(),
		cached: false,
		expires_at: null,
		warnings: [String(payload.params?.region)],
	}));
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.useRealTimers();
});

async function mount() {
	await act(async () => root.render(<Harness />));
	await flush();
}

async function choose(value: string) {
	await act(async () => {
		select().dispatchEvent(new Event("focusin", { bubbles: true }));
	});
	await flush();
	await act(async () => {
		select().value = value;
		select().dispatchEvent(new Event("change", { bubbles: true }));
	});
	await flush();
}

describe("viewer filter bar", () => {
	it("shows the labelled control with the author default and runs with it", async () => {
		await mount();
		expect(container.textContent).toContain("Area");
		expect(select().value).toBe("EU");
		expect(lastParams()).toEqual({ region: "EU" });
		expect(container.textContent).not.toContain("Reset");
	});

	it("loads options lazily, merges the choice into the request and stores it", async () => {
		await mount();
		expect(fixture.options).not.toHaveBeenCalled();
		await choose("US");
		expect(fixture.options).toHaveBeenCalledTimes(1);
		expect(lastParams()).toEqual({ region: "US" });
		expect(fixture.store.get("viewer:widget-1")).toEqual({ region: "US" });
		expect(container.textContent).toContain("Reset");
	});

	it("restores the choice on a later mount and resets to the default", async () => {
		fixture.store.set("viewer:widget-1", { region: "US" });
		await mount();
		expect(lastParams()).toEqual({ region: "US" });
		const reset = [...container.querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Reset"),
		);
		await act(async () => reset?.click());
		await flush();
		expect(lastParams()).toEqual({ region: "EU" });
		expect(fixture.store.has("viewer:widget-1")).toBe(false);
	});
});
