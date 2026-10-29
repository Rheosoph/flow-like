import { describe, expect, mock, test } from "bun:test";
import type {
	CreateStudioForecastPayload,
	IQueryState,
	SavedStudioForecast,
	StudioForecastResult,
} from "../state/backend-state/query-state";
import { type ForecastToolHost, ForecastToolSession } from "./forecast-tool";
import { type IBit, IBitTypes } from "./schema/bit/bit";

const context = { appId: "app", profileId: "profile" };
function request(): SavedStudioForecast["request"] {
	return {
		model_bit_id: "model",
		sql: "SELECT time, sales FROM history",
		params: { multiplier: 2 },
		mapping: { timestamp: "time", targets: ["sales"] },
		horizon: 2,
		backtest_horizon: 3,
		backtest_offset: 1,
		quantile_levels: [0.1, 0.5, 0.9],
	};
}
function result(id = "real-run"): StudioForecastResult {
	const point = (time: number, actual: number | null) => ({
		series_id: "series",
		target: "sales",
		timestamp_ms: time,
		point: 2,
		quantiles: [1, 2, 3],
		actual,
	});
	return {
		run_id: id,
		model: {
			bit_id: "model",
			family: "timestral",
			repository: null,
			revision: null,
		},
		capabilities: {
			family: "timestral",
			max_context: 8192,
			max_horizon: 1000,
			native_horizon: 64,
			quantile_levels: [0.1, 0.5, 0.9],
			joint_multivariate: true,
			past_covariates: true,
			known_covariates: true,
		},
		input_rows: 7,
		horizon: 2,
		quantile_levels: [0.1, 0.5, 0.9],
		point_statistic: "median",
		history: [10, 20, 30, 40, 50, 60, 70].map((timestamp_ms) => ({
			series_id: "series",
			target: "sales",
			timestamp_ms,
			value: 3,
		})),
		forecasts: [point(80, null), point(90, null)],
		covariates: [],
		backtest: {
			horizon: 3,
			offset: 1,
			forecasts: [point(40, 3), point(50, null), point(60, 3)],
			metrics: [
				{
					series_id: "series",
					target: "sales",
					observations: 2,
					mae: 1,
					rmse: 1,
					mape: 100 / 3,
					wape: 100 / 3,
				},
			],
		},
	};
}
function fixture(now?: () => number) {
	const output = result();
	const bit = {
		id: "model",
		hub: "hub.example",
		type: IBitTypes.Forecasting,
		meta: { en: { name: "Timestral" } },
		parameters: { model_type: "timestral" },
	} as unknown as IBit;
	let settingsId = "profile";
	const query = {
		forecast: mock<IQueryState["forecast"]>(async () => output),
		createForecast: mock<IQueryState["createForecast"]>(
			async (
				appId: string,
				payload: CreateStudioForecastPayload,
			): Promise<SavedStudioForecast> => ({
				...payload,
				id: "saved",
				app_id: appId,
				created_by: "viewer",
				created_at: "2026-10-19T12:00:00Z",
			}),
		),
		getForecast: mock<IQueryState["getForecast"]>(
			async (): Promise<SavedStudioForecast> => ({
				id: "saved",
				app_id: "app",
				name: "Forecast",
				created_by: "viewer",
				created_at: "2026-10-19T12:00:00Z",
				request: request(),
				result: output,
			}),
		),
		listForecasts: mock<IQueryState["listForecasts"]>(async () => []),
	};
	const host: ForecastToolHost = {
		identity: "hub/profile/viewer",
		profileId: "profile",
		available: true,
		backend: {
			queryState: query,
			bitState: {
				getProfileBits: async () => [bit, { ...bit, id: "inactive" }],
			},
			userState: {
				getSettingsProfile: async () => ({
					hub_profile: { id: settingsId, hub: "hub.example", bits: ["model"] },
				}),
			},
		} as unknown as ForecastToolHost["backend"],
	};
	const session = new ForecastToolSession(now);
	session.bind(host);
	return {
		session,
		query,
		host,
		output,
		changeSettings: (id: string) => {
			settingsId = id;
		},
	};
}

describe("FlowPilot forecast_tool", () => {
	test("lists activated models and runs with the host profile", async () => {
		const { session, query } = fixture();
		const models = await session.execute({ operation: "list_models" }, context);
		expect(models).toMatchObject({
			total: 1,
			models: [{ id: "model", family: "timestral" }],
		});
		const output = await session.execute(
			{ operation: "run", request: request(), user_scoped: true, limit: 1 },
			context,
		);
		expect(query.forecast.mock.calls[0]).toEqual([
			"app",
			{ ...request(), profile_id: "profile" },
			true,
		]);
		expect(output).toMatchObject({
			run_id: "real-run",
			saved: false,
			forecasts: { total: 2, truncated: true },
			backtest: {
				offset: 1,
				evaluation_windows: [
					{
						cutoff_ms: 30,
						start_ms: 40,
						end_ms: 60,
						horizon: 3,
						observations: 2,
					},
				],
			},
		});
	});

	test("saves the exact completed snapshot and deduplicates retries", async () => {
		const { session, query, output } = fixture();
		const draft = request();
		const expected = structuredClone({ request: draft, result: output });
		await session.execute({ operation: "run", request: draft }, context);
		draft.horizon = 100;
		output.forecasts[0].point = 999;
		const args = {
			operation: "save",
			run_id: "real-run",
			name: "Planning",
			description: "Original run",
		};
		const [one, two] = await Promise.all([
			session.execute(args, context),
			session.execute(args, context),
		]);
		expect(query.createForecast).toHaveBeenCalledTimes(1);
		expect(query.createForecast.mock.calls[0]).toEqual([
			"app",
			{ ...expected, name: "Planning", description: "Original run" },
			false,
		]);
		expect(one).toEqual(two);
		expect(one).toMatchObject({
			saved: { id: "saved", run_id: "real-run" },
			href: "/library/config/explore?id=app&view=forecast&forecast=saved",
		});
	});

	test("does not let model-supplied snapshots or profiles reach the backend", async () => {
		const { session, query } = fixture();
		for (const args of [
			{ operation: "run", request: { ...request(), profile_id: "other" } },
			{ operation: "run", request: request(), profile_id: "other" },
			{ operation: "save", run_id: "invented", name: "Fake", result: result() },
			{
				operation: "save",
				run_id: "invented",
				name: "Fake",
				request: request(),
			},
			{ operation: "save", run_id: "invented", name: "Fake" },
		])
			await expect(session.execute(args, context)).rejects.toThrow();
		expect(query.forecast).not.toHaveBeenCalled();
		expect(query.createForecast).not.toHaveBeenCalled();
	});

	test("a run cannot be saved in another app or project/personal scope", async () => {
		const { session, query } = fixture();
		await session.execute(
			{ operation: "run", request: request(), user_scoped: true },
			context,
		);
		for (const scope of [
			{ app_id: "other", user_scoped: true },
			{ user_scoped: false },
		])
			await expect(
				session.execute(
					{ operation: "save", run_id: "real-run", name: "Saved", ...scope },
					context,
				),
			).rejects.toThrow("unavailable");
		expect(query.createForecast).not.toHaveBeenCalled();
	});

	test("profile, account, Hub and executor changes invalidate retained runs", async () => {
		for (const change of [
			(host: ForecastToolHost) => ({ ...host, profileId: "other" }),
			(host: ForecastToolHost) => ({
				...host,
				identity: "hub/profile/other-viewer",
			}),
			(host: ForecastToolHost) => ({
				...host,
				identity: "other-hub/profile/viewer",
			}),
			(host: ForecastToolHost) => ({ ...host, available: false }),
			(host: ForecastToolHost) => ({
				...host,
				backend: {
					...host.backend,
					queryState: { ...host.backend.queryState },
				},
			}),
		]) {
			const { session, host, query } = fixture();
			await session.execute({ operation: "run", request: request() }, context);
			session.bind(change(host));
			session.bind(host);
			await expect(
				session.execute(
					{ operation: "save", run_id: "real-run", name: "Saved" },
					context,
				),
			).rejects.toThrow("unavailable");
			expect(query.createForecast).not.toHaveBeenCalled();
		}
	});

	test("rejects missing or changed host profile before inference", async () => {
		const { session, query, changeSettings } = fixture();
		await expect(
			session.execute({ operation: "run", request: request() }, {}),
		).rejects.toThrow("profile");
		changeSettings("other");
		await expect(
			session.execute({ operation: "run", request: request() }, context),
		).rejects.toThrow("profile changed");
		expect(query.forecast).not.toHaveBeenCalled();
	});

	test("late inference cannot publish or retain after identity changes", async () => {
		const { session, host, query } = fixture();
		let complete!: (value: StudioForecastResult) => void;
		query.forecast.mockImplementation(
			() =>
				new Promise((resolve) => {
					complete = resolve;
				}),
		);
		const pending = session.execute(
			{ operation: "run", request: request() },
			context,
		);
		while (!complete) await Promise.resolve();
		session.bind({ ...host, identity: "changed" });
		complete(result());
		await expect(pending).rejects.toThrow("changed");
		session.bind(host);
		await expect(
			session.execute(
				{ operation: "save", run_id: "real-run", name: "Late" },
				context,
			),
		).rejects.toThrow("unavailable");
	});

	test("cancelled requests cannot save after async profile lookup", async () => {
		const { session, query } = fixture();
		await session.execute({ operation: "run", request: request() }, context);
		let active = true;
		const pending = session.execute(
			{ operation: "save", run_id: "real-run", name: "Cancelled" },
			{
				...context,
				assertActive: () => {
					if (!active) throw new Error("Cancelled");
				},
			},
		);
		active = false;
		await expect(pending).rejects.toThrow("Cancelled");
		expect(query.createForecast).not.toHaveBeenCalled();
	});

	test("saved reads require no active model and return compact exact setup", async () => {
		const { session, host, query } = fixture();
		host.backend.bitState.getProfileBits = async () => [];
		const saved = await session.execute(
			{ operation: "get", forecast_id: "saved", user_scoped: true, limit: 1 },
			context,
		);
		expect(saved).toMatchObject({
			request: request(),
			backtest: { offset: 1 },
			href: "/library/config/explore?id=app&view=forecast&scope=user&forecast=saved",
		});
		expect(query.getForecast.mock.calls[0]).toEqual(["app", "saved", true]);
		expect(query.forecast).not.toHaveBeenCalled();
		await session.execute(
			{ operation: "list", limit: 5, user_scoped: true },
			context,
		);
		expect(query.listForecasts.mock.calls[0]).toEqual(["app", true, 6]);
	});

	test("only three recent runs remain saveable", async () => {
		const { session, query } = fixture();
		for (let i = 1; i <= 4; i++) {
			query.forecast.mockImplementation(async () => result(`run-${i}`));
			await session.execute({ operation: "run", request: request() }, context);
		}
		await expect(
			session.execute(
				{ operation: "save", run_id: "run-1", name: "Expired" },
				context,
			),
		).rejects.toThrow("unavailable");
		await session.execute(
			{ operation: "save", run_id: "run-4", name: "Recent" },
			context,
		);
		expect(query.createForecast).toHaveBeenCalledTimes(1);
	});

	test("completed runs expire after thirty minutes", async () => {
		let now = 100;
		const { session, query } = fixture(() => now);
		await session.execute({ operation: "run", request: request() }, context);
		now += 30 * 60 * 1000;
		await expect(
			session.execute(
				{ operation: "save", run_id: "real-run", name: "Expired" },
				context,
			),
		).rejects.toThrow("unavailable");
		expect(query.createForecast).not.toHaveBeenCalled();
	});

	test("bounds saved-copy receipts while preserving retry idempotency", async () => {
		const { session, query } = fixture();
		await session.execute({ operation: "run", request: request() }, context);
		for (let i = 0; i < 32; i++)
			await session.execute(
				{ operation: "save", run_id: "real-run", name: `Copy ${i}` },
				context,
			);
		await expect(
			session.execute(
				{ operation: "save", run_id: "real-run", name: "One more" },
				context,
			),
		).rejects.toThrow("32 saved-copy limit");
		await session.execute(
			{ operation: "save", run_id: "real-run", name: "Copy 0" },
			context,
		);
		expect(query.createForecast).toHaveBeenCalledTimes(32);
	});

	test("unusually large identifiers cannot overflow the chat preview", async () => {
		const { session, output } = fixture();
		output.forecasts[0].series_id = "large".repeat(30_000);
		const preview = await session.execute(
			{ operation: "run", request: request() },
			context,
		);
		expect(preview).toMatchObject({
			run_id: "real-run",
			response_truncated: true,
		});
		expect(
			new TextEncoder().encode(JSON.stringify(preview)).length,
		).toBeLessThan(64 * 1024);
	});
});
