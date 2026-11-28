import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
	CrossAppSqlPayload,
	CrossAppSqlResult,
} from "../../state/backend-state/query-state";
import {
	type HomeDataConfig,
	normalizeHomeDataConfig,
} from "./home-data-query";
import { useHomeData } from "./use-home-data";

const fixture = vi.hoisted(() => {
	const execute =
		vi.fn<(payload: CrossAppSqlPayload) => Promise<CrossAppSqlResult>>();
	return {
		execute,
		backend: {
			profile: { id: "profile" },
			dbState: {},
			graphState: {},
			queryState: { executeCrossAppSql: execute },
		},
		auth: { user: { profile: { sub: "viewer" } } },
	};
});
vi.mock("../../state/backend-state", () => ({
	useBackend: () => fixture.backend,
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => fixture.auth }));

let root: Root;
let container: HTMLDivElement;

const config = (sql: string): HomeDataConfig =>
	normalizeHomeDataConfig({
		sourceKind: "sql",
		visualization: "table",
		sql,
		sources: [{ alias: "crm", appId: "app-crm", scope: "project" }],
		cacheSeconds: 300,
	});

const answer = (
	rows: Record<string, unknown>[],
	cached = false,
): CrossAppSqlResult => ({
	columns: [{ name: "region", type_name: "Utf8", position: 0 }],
	rows,
	row_count: rows.length,
	truncated: false,
	tables: ["crm.orders"],
	computed_at: Date.parse("2026-11-20T10:00:00Z"),
	cached,
	expires_at: Date.now() + 300_000,
});

function Probe({ value }: { value: HomeDataConfig }) {
	const data = useHomeData(value);
	return (
		<>
			<output>
				{JSON.stringify({
					rows: data.result?.rows ?? null,
					loading: data.loading,
					error: data.error,
					stale: data.stale,
					cached: data.cached,
				})}
			</output>
			<button type="button" onClick={data.refresh}>
				Refresh
			</button>
		</>
	);
}

const shown = () =>
	JSON.parse(container.querySelector("output")?.textContent ?? "{}");

async function render(value: HomeDataConfig) {
	await act(async () => root.render(<Probe value={value} />));
}
async function settle() {
	await act(async () => vi.advanceTimersByTimeAsync(250));
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers();
	fixture.execute.mockReset();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.useRealTimers();
});

describe("cross-app home data", () => {
	it("asks for a cached result and reports whether it was reused", async () => {
		fixture.execute.mockResolvedValue(answer([{ region: "West" }], true));
		await render(config("SELECT region FROM crm.orders -- cached"));
		await settle();
		expect(fixture.execute).toHaveBeenCalledTimes(1);
		const payload = fixture.execute.mock.calls[0][0];
		expect(payload.cache_seconds).toBe(300);
		expect(payload.fresh).toBe(false);
		expect(payload.sources).toEqual([
			{ alias: "crm", app_id: "app-crm", scope: "project" },
		]);
		expect(shown()).toMatchObject({
			rows: [{ region: "West" }],
			cached: true,
			stale: false,
		});
	});

	it("bypasses the cache on refresh and keeps rows through a failed refresh", async () => {
		fixture.execute.mockResolvedValueOnce(answer([{ region: "West" }]));
		await render(config("SELECT region FROM crm.orders -- refresh"));
		await settle();
		let reject: (reason: Error) => void = () => {};
		fixture.execute.mockReturnValueOnce(
			new Promise((_, fail) => {
				reject = fail;
			}),
		);
		await act(async () => container.querySelector("button")?.click());
		await settle();
		expect(fixture.execute.mock.calls[1][0].fresh).toBe(true);
		expect(shown()).toMatchObject({
			rows: [{ region: "West" }],
			loading: true,
			stale: true,
		});
		await act(async () => reject(new Error("The query failed: boom")));
		expect(shown()).toMatchObject({
			rows: [{ region: "West" }],
			loading: false,
			error: "The query failed: boom",
			stale: true,
		});

		fixture.execute.mockResolvedValueOnce(answer([{ region: "East" }]));
		await act(async () => container.querySelector("button")?.click());
		await settle();
		expect(fixture.execute.mock.calls[2][0].fresh).toBe(true);
		expect(shown()).toMatchObject({
			rows: [{ region: "East" }],
			error: null,
			stale: false,
		});
	});

	it("shows the last rows at once when the widget mounts again", async () => {
		const value = config("SELECT region FROM crm.orders -- remount");
		fixture.execute.mockResolvedValueOnce(answer([{ region: "West" }]));
		await render(value);
		await settle();
		await act(async () => root.unmount());
		root = createRoot(container);

		fixture.execute.mockReturnValueOnce(new Promise(() => {}));
		await render(value);
		expect(shown()).toMatchObject({ rows: [{ region: "West" }] });
		await settle();
		expect(fixture.execute).toHaveBeenCalledTimes(2);
		expect(fixture.execute.mock.calls[1][0].fresh).toBe(false);
		expect(shown()).toMatchObject({
			rows: [{ region: "West" }],
			loading: true,
			stale: true,
		});
	});
});
