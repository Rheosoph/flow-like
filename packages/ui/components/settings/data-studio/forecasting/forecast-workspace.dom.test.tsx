// @vitest-environment happy-dom
import React, { act, type ComponentProps, type ReactElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import type {
	SavedStudioForecast,
	StudioForecastPayload,
	StudioForecastResult,
} from "../../../../state/backend-state/query-state";
import type { ForecastChart } from "./forecast-chart";
import { formatForecastTime } from "./forecast-data";
import type { ForecastLibrary } from "./forecast-library";
import {
	ForecastWorkspace,
	type ForecastWorkspaceProps,
} from "./forecast-workspace";

type ChartProps = ComponentProps<typeof ForecastChart>;
type LibraryProps = ComponentProps<typeof ForecastLibrary>;
const fixture = vi.hoisted(() => ({
	query: { executeSql: vi.fn(), forecast: vi.fn() },
	chart: undefined as ChartProps | undefined,
	library: undefined as LibraryProps | undefined,
	save: vi.fn(),
}));
vi.mock("../../../../state/backend-state", () => ({
	useBackend: () => ({ queryState: fixture.query }),
}));
vi.mock("./forecast-chart", () => ({
	ForecastChart: (props: ChartProps) => {
		fixture.chart = props;
		return <output data-testid="chart">{JSON.stringify(props.points)}</output>;
	},
}));
vi.mock("./forecast-library", () => ({
	ForecastLibrary: (props: LibraryProps) => {
		fixture.library = props;
		return (
			<button
				type="button"
				disabled={!props.run || props.running || props.stale || !props.canWrite}
				onClick={() => fixture.save(props.run)}
			>
				Save snapshot
			</button>
		);
	},
}));
vi.mock("../../../ui/sql-editor", () => ({
	SqlEditor: ({
		value,
		onChange,
	}: { value: string; onChange: (value: string) => void }) => (
		<textarea
			aria-label="SQL"
			value={value}
			onChange={(event) => onChange(event.target.value)}
		/>
	),
}));
vi.mock("../../../ui/switch", () => ({
	Switch: ({
		checked,
		onCheckedChange,
		...props
	}: ComponentProps<"input"> & {
		onCheckedChange: (checked: boolean) => void;
	}) => (
		<input
			{...props}
			type="checkbox"
			checked={checked}
			onChange={(event) => onCheckedChange(event.target.checked)}
		/>
	),
}));
vi.mock("../../../ui/select", () => ({
	Select: ({
		value,
		onValueChange,
		disabled,
		children,
	}: {
		value: string;
		onValueChange: (value: string) => void;
		disabled?: boolean;
		children: React.ReactNode;
	}) => {
		const [trigger, content] = React.Children.toArray(
			children,
		) as ReactElement<{
			"aria-label"?: string;
			children?: React.ReactNode;
		}>[];
		return (
			<select
				aria-label={trigger.props["aria-label"]}
				value={value}
				disabled={disabled}
				onChange={(event) => onValueChange(event.target.value)}
			>
				{content.props.children}
			</select>
		);
	},
	SelectTrigger: () => null,
	SelectValue: () => null,
	SelectContent: () => null,
	SelectItem: (props: ComponentProps<"option">) => <option {...props} />,
}));
// Keep the real cutoff controls, replacing only Radix's pointer implementation.
// Input and release are separate events so changing the slider cannot silently run a model.
vi.mock("@radix-ui/react-slider", () => ({
	Root: ({
		value,
		min,
		max,
		disabled,
		onValueChange,
		onValueCommit,
	}: {
		value: number[];
		min: number;
		max: number;
		disabled: boolean;
		onValueChange: (value: number[]) => void;
		onValueCommit: (value: number[]) => void;
	}) => (
		<input
			aria-label="Historical cutoff"
			type="range"
			min={min}
			max={max}
			value={value[0]}
			disabled={disabled}
			onInput={(event) => onValueChange([Number(event.currentTarget.value)])}
			onPointerUp={(event) =>
				onValueCommit([Number(event.currentTarget.value)])
			}
			onChange={() => {}}
		/>
	),
	Track: () => null,
	Range: () => null,
	Thumb: () => null,
}));

const day = (index: number) => Date.UTC(2025, 0, index);
const model: IBit = {
	id: "model-a",
	type: IBitTypes.Forecasting,
	authors: [],
	created: "2025-01-01T00:00:00Z",
	updated: "2025-01-01T00:00:00Z",
	dependencies: [],
	dependency_tree_hash: "fixture-dependencies",
	hash: "fixture-model",
	hub: "https://hub.example",
	parameters: { model_type: "timestral" },
	meta: {
		en: {
			name: "Timestral",
			description: "Forecasting test fixture",
			created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			preview_media: [],
			tags: [],
		},
	},
};
const request = (offset = 0): StudioForecastPayload => ({
	profile_id: "profile-a",
	model_bit_id: "model-a",
	sql: 'SELECT * FROM "history"',
	mapping: {
		timestamp: "time",
		timestamp_format: "auto",
		series_id: "region",
		targets: ["sales", "cost"],
		past_covariates: [],
		known_covariates: [],
	},
	horizon: 2,
	quantile_levels: [],
	backtest_horizon: 2,
	backtest_offset: offset,
});

function result(offset = 0): StudioForecastResult {
	const combinations = ["sales", "cost"].flatMap((target) =>
		["north", "south"].map((series_id) => ({ target, series_id })),
	);
	return {
		run_id: `run-${offset}`,
		model: {
			bit_id: "model-a",
			family: "timestral",
			repository: null,
			revision: null,
		},
		capabilities: {
			family: "timestral",
			max_context: 128,
			max_horizon: 64,
			native_horizon: 8,
			quantile_levels: [0.1, 0.5, 0.9],
			joint_multivariate: true,
			past_covariates: true,
			known_covariates: true,
		},
		input_rows: 16,
		horizon: 2,
		quantile_levels: [0.1, 0.5, 0.9],
		point_statistic: "median",
		history: combinations.flatMap((selection) =>
			Array.from({ length: 8 }, (_, index) => ({
				...selection,
				timestamp_ms: day(index + 1),
				value: index + 10,
			})),
		),
		forecasts: combinations.flatMap((selection) =>
			[9, 10].map((index) => ({
				...selection,
				timestamp_ms: day(index),
				point: 20,
				quantiles: [18, 20, 22],
				actual: null,
			})),
		),
		covariates: [],
		backtest: {
			horizon: 2,
			offset,
			forecasts: combinations.flatMap((selection) =>
				[7 - offset, 8 - offset].map((index) => ({
					...selection,
					timestamp_ms: day(index),
					point: 20,
					quantiles: [18, 20, 22],
					actual: index + 9,
				})),
			),
			metrics: combinations.map((selection) => ({
				...selection,
				observations: 2,
				mae: offset + 1,
				rmse: offset + 2,
				mape: 10 + offset,
				wape:
					20 +
					offset +
					(selection.target === "cost" ? 10 : 0) +
					(selection.series_id === "south" ? 5 : 0),
			})),
		},
	};
}

function snapshot(offset: number): SavedStudioForecast {
	const { profile_id: _profile, ...stored } = request(offset);
	return {
		id: "saved-a",
		app_id: "app-a",
		name: "Earlier evaluation",
		created_at: "2025-01-11T00:00:00Z",
		created_by: "author",
		request: stored,
		result: result(offset),
	};
}

let root: Root;
let container: HTMLDivElement;
function required<T>(value: T | null | undefined): T {
	if (value === null || value === undefined)
		throw new Error("Expected test value");
	return value;
}
function button(text: string) {
	const found = [...container.querySelectorAll("button")].findLast(
		(item) => item.textContent === text,
	);
	if (!found) throw new Error(`Missing button: ${text}`);
	return found;
}
function slider() {
	return required(
		container.querySelector<HTMLInputElement>(
			'input[aria-label="Historical cutoff"]',
		),
	);
}
function metrics() {
	return container.querySelector(
		'section[aria-label="Measured forecast error"]',
	);
}
async function click(text: string) {
	await act(async () => button(text).click());
}
async function change(selector: string, value: string) {
	const element = required(
		container.querySelector<
			HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
		>(selector),
	);
	const prototype =
		element instanceof HTMLSelectElement
			? HTMLSelectElement.prototype
			: element instanceof HTMLTextAreaElement
				? HTMLTextAreaElement.prototype
				: HTMLInputElement.prototype;
	await act(async () => {
		required(Object.getOwnPropertyDescriptor(prototype, "value")?.set).call(
			element,
			value,
		);
		element.dispatchEvent(
			new Event(element instanceof HTMLSelectElement ? "change" : "input", {
				bubbles: true,
			}),
		);
	});
}
async function moveSlider(offset: number, commit = false) {
	await act(async () => {
		const input = slider();
		input.value = String(Number(input.max) - offset);
		input.dispatchEvent(
			new Event(commit ? "pointerup" : "input", { bubbles: true }),
		);
	});
}
async function mount(props: Partial<ForecastWorkspaceProps> = {}) {
	await act(async () =>
		root.render(
			<ForecastWorkspace
				appId="app-a"
				profileId="profile-a"
				projectTables={["history"]}
				userTables={[]}
				userScoped={false}
				onScopeChange={() => {}}
				models={[model]}
				canWrite
				{...props}
			/>,
		),
	);
}
async function firstEvaluation() {
	await mount();
	await click("Next: Columns");
	await change('select[aria-label="Series ID"]', "region");
	const cost = required(
		[...container.querySelectorAll<HTMLButtonElement>("fieldset button")].find(
			(item) => item.textContent === "cost",
		),
	);
	await act(async () => cost.click());
	await click("Next: Model");
	await change("#forecast-horizon", "2");
	await act(async () =>
		required(
			container.querySelector<HTMLInputElement>("#forecast-evaluate"),
		).click(),
	);
	await change("#forecast-holdout", "2");
	await click("Next: Review");
	expect(button("Run forecast").disabled).toBe(false);
	await click("Run forecast");
	expect(fixture.chart?.backtest).toBe(false);
	expect(button("Outlook").getAttribute("aria-pressed")).toBe("true");
	await click("Holdout");
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true, React });
	fixture.query.executeSql.mockReset().mockResolvedValue({
		columns: [
			{ name: "time", type_name: "Int64", position: 0 },
			{ name: "region", type_name: "Utf8", position: 1 },
			{ name: "sales", type_name: "Float32", position: 2 },
			{ name: "cost", type_name: "Float32", position: 3 },
		],
		rows: [],
		row_count: 0,
		truncated: false,
	});
	fixture.query.forecast
		.mockReset()
		.mockImplementation(
			async (_app: string, payload: StudioForecastPayload) => {
				const forecast = result(payload.backtest_offset);
				if (!payload.backtest_horizon) forecast.backtest = null;
				return forecast;
			},
		);
	fixture.chart = undefined;
	fixture.library = undefined;
	fixture.save.mockReset();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("forecast workspace historical evaluation", () => {
	it("starts with future predictions and offers holdout evaluation after the forecast", async () => {
		await mount();
		await click("Next: Columns");
		await click("Next: Model");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-evaluate")?.checked,
		).toBe(false);
		await change("#forecast-horizon", "2");
		await click("Next: Review");
		await click("Run forecast");
		expect(fixture.query.forecast.mock.calls[0][1]).toMatchObject({
			backtest_horizon: 0,
			backtest_offset: 0,
		});
		expect(fixture.chart).toMatchObject({ backtest: false, cutoff: day(8) });
		expect(
			container.querySelector('input[aria-label="Historical cutoff"]'),
		).toBeNull();
		await click("Holdout");
		expect(fixture.query.forecast).toHaveBeenCalledTimes(1);
		expect(slider().disabled).toBe(false);
		expect(fixture.chart?.backtest).toBe(false);
		expect(container.textContent).toContain("History: 6 stepsHoldout: 2 steps");
		const windowLength =
			'section[aria-label="Historical forecast cutoff"] input[type="number"]';
		await change(windowLength, "0");
		expect(button("Evaluate window").disabled).toBe(true);
		expect(
			container.querySelector<HTMLInputElement>(windowLength)?.disabled,
		).toBe(false);
		await change(windowLength, "2");
		expect(button("Evaluate window").disabled).toBe(false);
		expect(fixture.query.forecast).toHaveBeenCalledTimes(1);
		await click("Evaluate window");
		expect(fixture.query.forecast.mock.calls[1][1]).toMatchObject({
			backtest_horizon: 2,
			backtest_offset: 0,
		});
		expect(fixture.chart?.backtest).toBe(true);
		await click("Run forecast");
		expect(fixture.chart).toMatchObject({ backtest: false, cutoff: day(8) });
		expect(button("Outlook").getAttribute("aria-pressed")).toBe("true");
	});

	it("requests native point-only output for TinyTimeMixer and prevents unsupported covariates", async () => {
		const pointResult = result();
		pointResult.model.family = "tiny_time_mixer";
		pointResult.capabilities = {
			...pointResult.capabilities,
			family: "tiny_time_mixer",
			quantile_levels: [],
			past_covariates: false,
			known_covariates: false,
		};
		pointResult.quantile_levels = [];
		pointResult.point_statistic = "mean";
		pointResult.forecasts = pointResult.forecasts.map((point) => ({
			...point,
			quantiles: [],
		}));
		pointResult.backtest = null;
		fixture.query.forecast.mockResolvedValue(pointResult);
		await mount({
			models: [{ ...model, parameters: { model_type: "tiny_time_mixer" } }],
		});
		await click("Next: Columns");
		expect(container.textContent).toContain(
			"This checkpoint does not accept covariates.",
		);
		expect(
			container.querySelector<HTMLInputElement>(
				'input[aria-label="Include cost"]',
			)?.disabled,
		).toBe(true);
		await click("Next: Model");
		expect(container.textContent).toContain("Point forecast only");
		expect(container.textContent).not.toContain("P10");
		await change("#forecast-horizon", "2");
		await click("Next: Review");
		expect(container.textContent).toContain("Point forecast only");
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenCalledExactlyOnceWith(
			"app-a",
			expect.objectContaining({
				quantile_levels: [],
				mapping: expect.objectContaining({
					past_covariates: [],
					known_covariates: [],
				}),
			}),
			false,
		);
		expect(fixture.chart).toMatchObject({
			quantileLevels: [],
			pointStatistic: "mean",
		});
		expect(
			required(fixture.chart).points.some((point) => point.forecast !== null),
		).toBe(true);
		expect(
			required(fixture.chart).points.every((point) => point.interval === null),
		).toBe(true);
	});
	it("waits for release, submits the committed offset and retains the chosen metric and series", async () => {
		await firstEvaluation();
		expect(fixture.query.forecast).toHaveBeenCalledExactlyOnceWith(
			"app-a",
			request(),
			false,
		);
		expect(metrics()?.textContent).toContain("20%");
		await click("cost");
		await change('select[aria-label="Chart series"]', JSON.stringify("south"));
		const previousPoints = required(fixture.chart).points;
		await moveSlider(2);
		expect(fixture.chart?.points).toBe(previousPoints);
		expect(fixture.query.forecast).toHaveBeenCalledTimes(1);
		expect(metrics()).toBeNull();
		expect(fixture.chart?.stale).toBe(true);
		expect(button("Save snapshot").disabled).toBe(true);
		// The release can carry a newer value than the preceding change event.
		await moveSlider(3, true);
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			request(3),
			false,
		);
		expect(fixture.chart).toMatchObject({
			metric: "cost",
			seriesLabel: "south",
			cutoff: day(3),
			backtest: true,
			stale: false,
			loading: false,
		});
		expect(
			fixture.chart?.points
				.filter((point) => point.forecast !== null)
				.map((point) => point.timestamp),
		).toEqual([day(4), day(5)]);
		expect(container.textContent).toContain(
			`Evaluated ${formatForecastTime(day(4), true)} to ${formatForecastTime(day(5), true)} UTC`,
		);
		expect(metrics()?.textContent).toContain("38%");
		await click("Save snapshot");
		expect(fixture.save.mock.calls[0][0].request.backtest_offset).toBe(3);
		expect(fixture.save.mock.calls[0][0].result.run_id).toBe("run-3");
		await click("Latest window");
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			request(0),
			false,
		);
		expect(fixture.chart?.cutoff).toBe(day(6));
		expect(metrics()?.textContent).toContain("35%");
	});

	it("keeps the prior chart marked stale and hides metrics and saving when reevaluation fails", async () => {
		await firstEvaluation();
		const previousPoints = required(fixture.chart).points;
		let reject!: (reason: Error) => void;
		fixture.query.forecast.mockReturnValueOnce(
			new Promise((_resolve, failure) => {
				reject = failure;
			}),
		);
		await moveSlider(2, true);
		expect(fixture.chart).toMatchObject({
			loading: true,
			stale: true,
			points: previousPoints,
		});
		expect(metrics()).toBeNull();
		expect(button("Save snapshot").disabled).toBe(true);
		await act(async () =>
			reject(new Error("The selected history has too few observations.")),
		);
		expect(container.textContent).toContain(
			"The selected history has too few observations.",
		);
		expect(fixture.chart).toMatchObject({
			loading: false,
			stale: true,
			failedRefresh: true,
			points: previousPoints,
		});
		expect(metrics()).toBeNull();
		expect(fixture.library).toMatchObject({ running: false, stale: true });
		await click("Save snapshot");
		expect(fixture.save).not.toHaveBeenCalled();
		// Returning to the old settings does not erase a failed refresh.
		await moveSlider(0);
		expect(fixture.chart?.stale).toBe(true);
		expect(metrics()).toBeNull();
		expect(button("Save snapshot").disabled).toBe(true);
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			request(0),
			false,
		);
		expect(fixture.chart).toMatchObject({ stale: false, failedRefresh: false });
		expect(metrics()?.textContent).toContain("20%");
	});

	it("restores a saved cutoff without rerunning and uses it for the next run", async () => {
		await mount();
		await act(async () => required(fixture.library).onOpen(snapshot(2)));
		expect(fixture.query.forecast).not.toHaveBeenCalled();
		expect(fixture.chart).toMatchObject({ cutoff: day(8), backtest: false });
		await click("Holdout");
		expect(Number(slider().max) - Number(slider().value)).toBe(2);
		expect(slider().disabled).toBe(false);
		expect(fixture.chart).toMatchObject({
			cutoff: day(4),
			backtest: true,
			stale: false,
		});
		expect(metrics()?.textContent).toContain("22%");
		expect(button("Save snapshot").disabled).toBe(false);
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			request(2),
			false,
		);
		const legacy = snapshot(0);
		const { backtest_offset: _requestOffset, ...legacyRequest } =
			legacy.request;
		const { offset: _resultOffset, ...legacyBacktest } = required(
			legacy.result.backtest,
		);
		await act(async () =>
			required(fixture.library).onOpen({
				...legacy,
				request: legacyRequest,
				result: { ...legacy.result, backtest: legacyBacktest },
			}),
		);
		await click("Holdout");
		expect(Number(slider().max) - Number(slider().value)).toBe(0);
		expect(fixture.chart?.stale).toBe(false);
		expect(metrics()?.textContent).toContain("20%");
	});

	it("disables historical evaluation after the source query changes until that source runs", async () => {
		await firstEvaluation();
		await click("Edit setup");
		await click("SQL query");
		await change('textarea[aria-label="SQL"]', 'SELECT * FROM "other_history"');
		expect(slider().disabled).toBe(true);
		expect(button("Next: Columns").disabled).toBe(true);
		expect(button("Save snapshot").disabled).toBe(true);
		expect(metrics()).toBeNull();
		expect(fixture.chart?.stale).toBe(true);
		expect(fixture.query.forecast).toHaveBeenCalledTimes(1);
	});
});

describe("forecast setup wizard", () => {
	it("explains grouping and preserves the query when opening SQL for an overall forecast", async () => {
		await mount();
		expect(container.textContent).toContain("overall total or average");
		await click("Next: Columns");
		expect(container.textContent).toContain("Rows are used as one series.");
		await change('select[aria-label="Series ID"]', "region");
		expect(container.textContent).toContain("Forecast each region separately");
		expect(container.textContent).toContain(
			"All groups are included in the run and saved together.",
		);
		await click("Next: Model");
		await click("Next: Review");
		expect(container.textContent).toContain("Each region separately");
		expect(container.textContent).toContain(
			"The chart shows one group at a time",
		);
		await act(async () =>
			required(
				container.querySelector<HTMLButtonElement>(
					'button[aria-label="Step 2: Columns"]',
				),
			).click(),
		);
		await click("Edit source SQL");
		expect(
			container.querySelector<HTMLTextAreaElement>('textarea[aria-label="SQL"]')
				?.value,
		).toBe('SELECT * FROM "history"');
		expect(fixture.query.forecast).not.toHaveBeenCalled();
	});

	it("keeps chart grouping tied to the completed run while setup changes", async () => {
		await firstEvaluation();
		expect(fixture.chart).toMatchObject({
			seriesColumn: "region",
			seriesCount: 2,
			seriesLabel: "north",
			hasEvaluationMetrics: true,
		});
		await click("Edit setup");
		await click("Next: Columns");
		await change('select[aria-label="Series ID"]', "__none__");
		expect(container.textContent).toContain("Rows are used as one series.");
		expect(fixture.chart).toMatchObject({
			seriesColumn: "region",
			seriesCount: 2,
			seriesLabel: "north",
			hasEvaluationMetrics: false,
			stale: true,
		});
		expect(fixture.query.forecast).toHaveBeenCalledTimes(1);
	});

	it("loads the chosen SQL before allowing mapping, and blocks missing targets and future inputs", async () => {
		await mount();
		expect(container.querySelector("#forecast-horizon")).toBeNull();
		expect(
			container
				.querySelector('[aria-current="step"]')
				?.getAttribute("aria-label"),
		).toBe("Step 1: Source");
		await click("SQL query");
		await change(
			'textarea[aria-label="SQL"]',
			'SELECT time, region, sales, cost FROM "history"',
		);
		expect(button("Next: Columns").disabled).toBe(true);
		expect(container.textContent).toContain(
			"Load the source columns before continuing.",
		);
		await click("Load columns");
		expect(fixture.query.executeSql).toHaveBeenLastCalledWith(
			"app-a",
			expect.objectContaining({
				sql: 'SELECT time, region, sales, cost FROM "history"',
				surface: "native",
			}),
			false,
		);
		await click("Next: Columns");
		expect(document.activeElement?.textContent).toBe("Step 2 of 4: Columns");
		await click("sales");
		expect(button("Next: Model").disabled).toBe(true);
		expect(container.textContent).toContain(
			"Select at least one metric to forecast.",
		);
		await click("cost");
		await change('select[aria-label="Series ID"]', "region");
		await act(async () =>
			required(
				container.querySelector<HTMLInputElement>(
					'input[aria-label="Include sales"]',
				),
			).click(),
		);
		await change('select[aria-label="Availability of sales"]', "known");
		expect(button("Next: Model").disabled).toBe(true);
		expect(container.textContent).toContain(
			"Choose future values for the known covariates.",
		);
		await change('select[aria-label="Future table"]', "history");
		await click("Next: Model");
		await change("#forecast-horizon", "0");
		expect(button("Next: Review").disabled).toBe(true);
		expect(container.textContent).toContain(
			"Use a forecast horizon between 1 and 65,536 steps.",
		);
		await change("#forecast-horizon", "2");
		await click("Next: Review");
		expect(fixture.query.forecast).not.toHaveBeenCalled();
		expect(
			container.querySelector('section[aria-label="Forecast setup"]')
				?.textContent,
		).toContain("Known future covariatessales");
		await click("Back");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-horizon")?.value,
		).toBe("2");
		await click("Next: Review");
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenCalledExactlyOnceWith(
			"app-a",
			expect.objectContaining({
				sql: 'SELECT time, region, sales, cost FROM "history"',
				mapping: {
					timestamp: "time",
					timestamp_format: "auto",
					series_id: "region",
					targets: ["cost"],
					past_covariates: [],
					known_covariates: ["sales"],
					frequency_ms: undefined,
				},
				future_sql: 'SELECT * FROM "history"',
				horizon: 2,
				backtest_horizon: 0,
				backtest_offset: 0,
			}),
			false,
		);
		expect(
			container.querySelector('section[aria-label="Forecast setup"]'),
		).toBeNull();
		expect(button("Edit setup").disabled).toBe(false);
	});

	it("stays at the source on a failed preview and preserves choices while moving back", async () => {
		fixture.query.executeSql.mockRejectedValueOnce(
			new Error("Source access denied"),
		);
		await mount();
		expect(button("Next: Columns").disabled).toBe(true);
		expect(container.textContent).toContain("Source access denied");
		expect(
			container.querySelector<HTMLButtonElement>(
				'button[aria-label="Step 2: Columns"]',
			)?.disabled,
		).toBe(true);
		expect(fixture.query.forecast).not.toHaveBeenCalled();
		await click("Load columns");
		await click("Next: Columns");
		await change('select[aria-label="Series ID"]', "region");
		await click("cost");
		await click("Next: Model");
		await change("#forecast-horizon", "7");
		await click("Back");
		expect(
			container.querySelector<HTMLSelectElement>(
				'select[aria-label="Series ID"]',
			)?.value,
		).toBe("region");
		expect(button("cost").getAttribute("aria-pressed")).toBe("true");
		await click("Next: Model");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-horizon")?.value,
		).toBe("7");
		expect(fixture.query.forecast).not.toHaveBeenCalled();
	});

	it("opens FlowPilot explicitly and preserves a saved cutoff when editing or starting over", async () => {
		const askFlowPilot = vi.fn();
		await mount({ onAskFlowPilot: askFlowPilot });
		await click("Ask FlowPilot");
		expect(askFlowPilot).toHaveBeenCalledTimes(1);
		expect(fixture.query.forecast).not.toHaveBeenCalled();
		const saved = snapshot(2);
		saved.request.quantile_levels = [0.1, 0.5, 0.9];
		await act(async () => required(fixture.library).onOpen(saved));
		expect(
			container.querySelector('section[aria-label="Forecast setup"]'),
		).toBeNull();
		const previews = fixture.query.executeSql.mock.calls.length;
		await click("Edit setup");
		expect(fixture.query.executeSql).toHaveBeenCalledTimes(previews + 1);
		await click("Next: Columns");
		expect(
			required(
				[...container.querySelectorAll("fieldset button")].find(
					(item) => item.textContent === "cost",
				),
			).getAttribute("aria-pressed"),
		).toBe("true");
		await click("Next: Model");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-horizon")?.value,
		).toBe("2");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-holdout")?.value,
		).toBe("2");
		await click("Next: Review");
		expect(container.textContent).toContain(
			"2 holdout steps · 2 steps after the window",
		);
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			{ ...request(2), quantile_levels: [0.1, 0.5, 0.9] },
			false,
		);
		await act(async () => required(fixture.library).onNew());
		expect(
			container
				.querySelector('[aria-current="step"]')
				?.getAttribute("aria-label"),
		).toBe("Step 1: Source");
		expect(fixture.library?.run).toBeNull();
		await click("Next: Columns");
		await click("Next: Model");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-horizon")?.value,
		).toBe("24");
		expect(
			container.querySelector<HTMLInputElement>("#forecast-evaluate")?.checked,
		).toBe(false);
		await click("Next: Review");
		await click("Run forecast");
		expect(fixture.query.forecast).toHaveBeenLastCalledWith(
			"app-a",
			expect.objectContaining({ quantile_levels: [] }),
			false,
		);
	});
});

describe("forecast source formats", () => {
	it("offers sensor measurements stored as numeric text and sends the chosen timestamp format", async () => {
		fixture.query.executeSql.mockResolvedValue({
			columns: [
				{
					name: "timestamp_utc",
					type_name: 'Timestamp(ms, "UTC")',
					position: 0,
				},
				{ name: "machine_id", type_name: "LargeUtf8", position: 1 },
				{ name: "temperature_c", type_name: "LargeUtf8", position: 2 },
			],
			rows: [
				{
					timestamp_utc: day(1),
					machine_id: "MACHINE_01",
					temperature_c: "62.43",
				},
				{
					timestamp_utc: day(2),
					machine_id: "MACHINE_01",
					temperature_c: "63.14",
				},
			],
			row_count: 2,
			truncated: false,
		});
		await mount();
		await click("Next: Columns");
		expect(container.textContent).toContain("temperature_cnumeric text");
		expect(container.textContent).toContain("2025-01-01T00:00:00.000Z");
		expect(button("Next: Model").disabled).toBe(false);
		await change('select[aria-label="Series ID"]', "machine_id");
		await click("Next: Model");
		await click("Next: Review");
		await click("Run forecast");
		expect(fixture.query.forecast.mock.calls[0][1].mapping).toMatchObject({
			timestamp: "timestamp_utc",
			timestamp_format: "auto",
			series_id: "machine_id",
			targets: ["temperature_c"],
		});
	});
	it("shows the ambiguous date and enables Next after selecting its order", async () => {
		fixture.query.executeSql.mockResolvedValue({
			columns: [
				{ name: "date", type_name: "Utf8", position: 0 },
				{ name: "sales", type_name: "Float64", position: 1 },
			],
			rows: [
				{ date: "04/05/2026", sales: 10 },
				{ date: "05/05/2026", sales: 12 },
			],
			row_count: 2,
			truncated: false,
		});
		await mount();
		await click("Next: Columns");
		expect(button("Next: Model").disabled).toBe(true);
		expect(container.textContent).toContain("Date order is ambiguous");
		await change('select[aria-label="Timestamp format"]', "day_first");
		expect(button("Next: Model").disabled).toBe(false);
		expect(container.textContent).toContain("2026-05-04T00:00:00.000Z");
	});
	it("explains a missing metric with its actual type and sample", async () => {
		fixture.query.executeSql.mockResolvedValue({
			columns: [
				{ name: "time", type_name: "Date32", position: 0 },
				{ name: "value", type_name: "LargeUtf8", position: 1 },
			],
			rows: [{ time: 1, value: "sensor offline" }],
			row_count: 1,
			truncated: false,
		});
		await mount();
		await click("Next: Columns");
		expect(button("Next: Model").disabled).toBe(true);
		expect(container.textContent).toContain("No usable numeric metrics found");
		expect(container.textContent).toContain(
			"value (LargeUtf8): sensor offline",
		);
	});
});
