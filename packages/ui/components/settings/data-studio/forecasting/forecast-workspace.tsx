"use client";

import {
	ArrowRight,
	Check,
	Database,
	FlaskConical,
	Loader2,
	Play,
	RefreshCw,
	SlidersHorizontal,
	Sparkles,
	SquareTerminal,
	TrendingUp,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getErrorMessage } from "../../../../lib/error-message";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { cn } from "../../../../lib/utils";
import { useBackend } from "../../../../state/backend-state";
import type {
	ExecuteSqlResult,
	ForecastTimestampFormat,
	SavedStudioForecast,
	StudioForecastPayload,
} from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { SqlEditor } from "../../../ui/sql-editor";
import { Switch } from "../../../ui/switch";
import { forecastBacktestRange } from "./forecast-backtest";
import { ForecastBacktestControls } from "./forecast-backtest-controls";
import { type CovariateTimeline, ForecastChart } from "./forecast-chart";
import {
	buildForecastPlot,
	finiteValue,
	forecastCovariateSupport,
	forecastOutputLabel,
	forecastingColumns,
	formatForecastTime,
	numericInputValue,
	sampleForecastPlot,
	sampleTimeline,
	tableForecastSql,
	timestampValue,
	visibleCovariatePoints,
} from "./forecast-data";
import { ForecastLibrary } from "./forecast-library";
import { ForecastMetrics } from "./forecast-metrics";
import {
	ForecastSetupReview,
	type ForecastSetupStep,
	ForecastSetupWizard,
} from "./forecast-setup-wizard";
import {
	type ForecastRunSnapshot,
	captureForecastRequest,
	forecastRequestFingerprint,
	previewForecastTargets,
	restoreForecastSetup,
	withoutSeriesTarget,
} from "./forecast-snapshots";
import {
	inspectForecastTimestamps,
	nativeTimestampUnit,
	timestampFormatOptions,
} from "./forecast-timestamps";

export interface ForecastWorkspaceProps {
	appId: string;
	profileId: string;
	projectTables: string[];
	userTables: string[];
	userScoped: boolean;
	onScopeChange: (userScoped: boolean) => void;
	models: IBit[];
	canWrite: boolean;
	savedForecastId?: string;
	onSavedForecastChange?: (id?: string) => void;
	onAskFlowPilot?: () => void;
}

export function ForecastWorkspace(props: Readonly<ForecastWorkspaceProps>) {
	return (
		<ForecastWorkspaceBody
			key={`${props.appId}:${props.profileId}:${props.userScoped}`}
			{...props}
		/>
	);
}

function ForecastWorkspaceBody({
	appId,
	profileId,
	projectTables,
	userTables,
	userScoped,
	onScopeChange,
	models,
	canWrite,
	savedForecastId,
	onSavedForecastChange,
	onAskFlowPilot,
}: Readonly<ForecastWorkspaceProps>) {
	const backend = useBackend();
	const tables = userScoped ? userTables : projectTables;
	const forecastingModels = models.filter(
		(model) => model.type === IBitTypes.Forecasting,
	);
	const [modelId, setModelId] = useState(forecastingModels[0]?.id ?? "");
	const [table, setTable] = useState(tables[0] ?? "");
	const [sourceMode, setSourceMode] = useState<"table" | "sql">(
		savedForecastId ? "sql" : "table",
	);
	const [sql, setSql] = useState(tableForecastSql(tables[0] ?? ""));
	const [futureSql, setFutureSql] = useState("");
	const [futureTable, setFutureTable] = useState("");
	const [futureMode, setFutureMode] = useState<"table" | "sql">("table");
	const [preview, setPreview] = useState<{
		sql: string;
		result: ExecuteSqlResult;
	} | null>(null);
	const [previewing, setPreviewing] = useState(false);
	const [previewError, setPreviewError] = useState<string | null>(null);
	const [timestamp, setTimestamp] = useState("");
	const [timestampFormat, setTimestampFormat] = useState<
		ForecastTimestampFormat | undefined
	>("auto");
	const [seriesId, setSeriesId] = useState("");
	const [targets, setTargets] = useState<string[]>([]);
	const [covariates, setCovariates] = useState<
		Record<string, { enabled: boolean; role: "past" | "known" }>
	>({});
	const [horizon, setHorizon] = useState("24");
	const [evaluate, setEvaluate] = useState(false);
	const [holdout, setHoldout] = useState("12");
	const [backtestOffset, setBacktestOffset] = useState(0);
	const [frequency, setFrequency] = useState("");
	const [running, setRunning] = useState(false);
	const [runError, setRunError] = useState<string | null>(null);
	const [run, setRun] = useState<ForecastRunSnapshot | null>(null);
	const [savedOpening, setSavedOpening] = useState(false);
	const [sourceRevision, setSourceRevision] = useState(0);
	const [setupOpen, setSetupOpen] = useState(!savedForecastId);
	const [setupStep, setSetupStep] = useState<ForecastSetupStep>(0);
	const [requestExtras, setRequestExtras] = useState<
		Pick<
			StudioForecastPayload,
			"params" | "future_params" | "quantile_levels" | "budget"
		>
	>({ quantile_levels: [] });
	const [activeTarget, setActiveTarget] = useState("");
	const [activeSeries, setActiveSeries] = useState("");
	const [view, setView] = useState<"forecast" | "evaluation">("forecast");
	const previewSequence = useRef(0);
	const runSequence = useRef(0);
	const runInFlight = useRef(false);
	const mappingRef = useRef({ timestamp, seriesId });
	mappingRef.current = { timestamp, seriesId };
	const sourceSql = sourceMode === "table" ? tableForecastSql(table) : sql;
	const model = forecastingModels.find((item) => item.id === modelId);
	const modelMeta = model?.meta.en ?? (model && Object.values(model.meta)[0]);
	const modelName =
		model?.meta.en?.name ||
		(model && Object.values(model.meta)[0]?.name) ||
		modelId;
	const modelType = model?.parameters?.model_type;
	const covariateSupport = forecastCovariateSupport(
		modelId,
		run?.result,
		modelType,
	);
	const pointOnly =
		run?.result.model.bit_id === modelId
			? run.result.capabilities.quantile_levels.length === 0
			: modelType === "tiny_time_mixer";
	const canUseCovariates = covariateSupport.past || covariateSupport.known;
	const covariateRole = (name: string) =>
		covariates[name]?.role ?? (covariateSupport.past ? "past" : "known");
	const sourceReady = preview?.sql === sourceSql && !previewing;
	const columns = preview?.result.columns ?? [];
	const timestampType = columns.find(
		(column) => column.name === timestamp,
	)?.type_name;
	const inferred = useMemo(
		() => forecastingColumns(columns, preview?.result.rows),
		[columns, preview?.result.rows],
	);
	const timestampInspection = useMemo(
		() =>
			inspectForecastTimestamps(
				(preview?.result.rows ?? []).map((row) => row[timestamp]),
				timestampType ?? "",
				timestampFormat,
			),
		[preview?.result.rows, timestamp, timestampType, timestampFormat],
	);
	const metricCandidates = inferred.numeric.filter(
		(column) => column.name !== timestamp && column.name !== seriesId,
	);
	const covariateCandidates = [
		...new Set([
			...inferred.numeric.map((column) => column.name),
			...Object.keys(covariates),
		]),
	]
		.filter(
			(name) =>
				name !== timestamp && name !== seriesId && !targets.includes(name),
		)
		.map((name) => ({ name }));
	const selectedCovariates = Object.keys(covariates)
		.filter(
			(name) =>
				covariates[name]?.enabled &&
				name !== timestamp &&
				name !== seriesId &&
				!targets.includes(name),
		)
		.map((name) => ({ name }));
	const knownCovariates = selectedCovariates
		.filter((column) => covariateRole(column.name) === "known")
		.map((column) => column.name);
	const selectedFutureSql =
		futureMode === "table" ? tableForecastSql(futureTable) : futureSql;
	const payload: StudioForecastPayload = {
		...requestExtras,
		profile_id: profileId,
		model_bit_id: modelId,
		sql: sourceSql,
		mapping: {
			timestamp,
			timestamp_format: timestampFormat,
			series_id: seriesId || undefined,
			targets,
			past_covariates: selectedCovariates
				.filter((column) => covariateRole(column.name) === "past")
				.map((column) => column.name),
			known_covariates: knownCovariates,
			frequency_ms: frequency ? Number(frequency) : undefined,
		},
		horizon: Number(horizon),
		future_sql: knownCovariates.length > 0 ? selectedFutureSql : undefined,
		backtest_horizon: evaluate ? Number(holdout) : 0,
		backtest_offset: evaluate ? backtestOffset : 0,
	};
	const fingerprint = forecastRequestFingerprint(payload);
	const retainedSource =
		run !== null &&
		forecastRequestFingerprint({
			...payload,
			backtest_horizon: 0,
			backtest_offset: 0,
		}) ===
			forecastRequestFingerprint({
				...run.request,
				backtest_horizon: 0,
				backtest_offset: 0,
			});
	const stale =
		run !== null && (run.fingerprint !== fingerprint || runError !== null);
	const catalog = useMemo(
		() => ({
			tables: tables.map((name) => ({
				name,
				scope: userScoped ? ("user" as const) : ("project" as const),
				columns: name === table && sourceMode === "table" ? columns : undefined,
			})),
			views: [],
			params: Object.keys(requestExtras.params ?? {}),
		}),
		[tables, userScoped, table, sourceMode, columns, requestExtras.params],
	);

	useEffect(
		() => () => {
			previewSequence.current++;
			runSequence.current++;
		},
		[],
	);
	useEffect(() => {
		if (!modelId && !savedForecastId && !savedOpening)
			setModelId(forecastingModels[0]?.id ?? "");
	}, [forecastingModels, modelId, savedForecastId, savedOpening]);
	useEffect(() => {
		if (!tables.includes(table)) setTable(tables[0] ?? "");
	}, [tables, table]);

	const loadSource = useCallback(async () => {
		if (!sourceSql.trim()) return;
		const sequence = ++previewSequence.current;
		setPreviewing(true);
		setPreviewError(null);
		try {
			const result = await backend.queryState.executeSql(
				appId,
				{
					sql: sourceSql,
					params: requestExtras.params,
					surface: "native",
					limit: 500,
				},
				userScoped,
			);
			if (sequence !== previewSequence.current) return;
			setPreview({ sql: sourceSql, result });
			const names = new Set(result.columns.map((column) => column.name));
			const defaults = forecastingColumns(result.columns, result.rows);
			const mappedTimestamp = names.has(mappingRef.current.timestamp)
				? mappingRef.current.timestamp
				: defaults.timestamp;
			const mappedSeries = names.has(mappingRef.current.seriesId)
				? mappingRef.current.seriesId
				: "";
			setTimestamp(mappedTimestamp);
			if (mappedTimestamp !== mappingRef.current.timestamp)
				setTimestampFormat("auto");
			setSeriesId(mappedSeries);
			setTargets((current) =>
				previewForecastTargets(
					current,
					defaults.numeric.map((column) => column.name),
					mappedTimestamp,
					mappedSeries,
				),
			);
			setCovariates((current) =>
				Object.fromEntries(
					Object.entries(current).filter(([name]) => names.has(name)),
				),
			);
		} catch (error) {
			if (sequence === previewSequence.current) {
				setPreview(null);
				setPreviewError(getErrorMessage(error));
			}
		} finally {
			if (sequence === previewSequence.current) setPreviewing(false);
		}
	}, [appId, backend.queryState, sourceSql, userScoped, requestExtras.params]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: New prediction refreshes an unchanged table selection.
	useEffect(() => {
		if (sourceMode === "table" && sourceSql) void loadSource();
	}, [loadSource, sourceMode, sourceSql, sourceRevision]);

	const sourceError = !sourceSql.trim()
		? "Choose a source table or enter a query."
		: !sourceReady && !retainedSource
			? "Load the source columns before continuing."
			: null;
	let mappingError: string | null = null;
	if (!timestamp) mappingError = "Choose a timestamp column.";
	else if (sourceReady && timestampInspection.error)
		mappingError = timestampInspection.error;
	else if (targets.length === 0)
		mappingError =
			metricCandidates.length === 0
				? "No usable numeric metrics found. Check the column values below or edit the source SQL."
				: "Select at least one metric to forecast.";
	else if (
		targets.includes(timestamp) ||
		targets.includes(seriesId) ||
		timestamp === seriesId
	)
		mappingError =
			"Timestamp, series ID and metrics must use different columns.";
	else if (
		frequency &&
		(!Number.isSafeInteger(Number(frequency)) || Number(frequency) <= 0)
	)
		mappingError =
			"The time interval must be a positive whole number of milliseconds.";
	else if (
		selectedCovariates.some(
			(column) => !covariateSupport[covariateRole(column.name)],
		)
	)
		mappingError =
			"This checkpoint does not support a selected covariate role. Change its availability or switch it off.";
	else if (knownCovariates.length > 0 && !selectedFutureSql.trim())
		mappingError = "Choose future values for the known covariates.";
	let modelError: string | null = null;
	if (!model)
		modelError =
			"Activate a forecasting model in your profile to run a prediction.";
	else if (
		!Number.isSafeInteger(Number(horizon)) ||
		Number(horizon) < 1 ||
		Number(horizon) > 65536
	)
		modelError = "Use a forecast horizon between 1 and 65,536 steps.";
	else if (
		evaluate &&
		(!Number.isSafeInteger(Number(holdout)) ||
			Number(holdout) < 1 ||
			Number(holdout) > 65536)
	)
		modelError = "Use a holdout between 1 and 65,536 steps.";
	const disabledReason = savedOpening
		? "Wait for the saved prediction to open."
		: (sourceError ?? mappingError ?? modelError);

	const runForecast = async (selectedOffset?: number) => {
		if (disabledReason || runInFlight.current) return;
		const sequence = ++runSequence.current;
		const submitted = {
			...payload,
			backtest_offset: evaluate ? (selectedOffset ?? backtestOffset) : 0,
		};
		const request = captureForecastRequest(submitted);
		const submittedFingerprint = forecastRequestFingerprint(submitted);
		runInFlight.current = true;
		setRunning(true);
		setRunError(null);
		try {
			const result = await backend.queryState.forecast(
				appId,
				submitted,
				userScoped,
			);
			if (sequence !== runSequence.current) return;
			setRun({ result, fingerprint: submittedFingerprint, request });
			setActiveTarget((current) =>
				result.forecasts.some((point) => point.target === current)
					? current
					: (result.forecasts[0]?.target ?? targets[0] ?? ""),
			);
			setActiveSeries((current) =>
				result.forecasts.some((point) => point.series_id === current)
					? current
					: (result.forecasts[0]?.series_id ?? ""),
			);
			setView(selectedOffset !== undefined ? "evaluation" : "forecast");
			setSetupOpen(false);
		} catch (error) {
			if (sequence === runSequence.current) setRunError(getErrorMessage(error));
		} finally {
			if (sequence === runSequence.current) {
				runInFlight.current = false;
				setRunning(false);
			}
		}
	};

	const result = run?.result;
	const resultTargets = result
		? [...new Set(result.forecasts.map((point) => point.target))]
		: targets;
	const resultSeries = result
		? [...new Set(result.forecasts.map((point) => point.series_id))]
		: [];
	const target = resultTargets.includes(activeTarget)
		? activeTarget
		: (resultTargets[0] ?? "");
	const series = resultSeries.includes(activeSeries)
		? activeSeries
		: (resultSeries[0] ?? "");
	const history = useMemo(
		() =>
			result?.history.filter(
				(point) => point.target === target && point.series_id === series,
			) ?? [],
		[result?.history, target, series],
	);
	const backtestRange = useMemo(
		() =>
			forecastBacktestRange(
				result?.history ?? [],
				target,
				series,
				Number(holdout),
				backtestOffset,
			),
		[result?.history, target, series, holdout, backtestOffset],
	);
	const viewingEvaluation = view === "evaluation" && Boolean(result?.backtest);
	const predictions = useMemo(
		() =>
			(viewingEvaluation
				? result?.backtest?.forecasts
				: result?.forecasts
			)?.filter(
				(point) => point.target === target && point.series_id === series,
			) ?? [],
		[
			viewingEvaluation,
			result?.backtest?.forecasts,
			result?.forecasts,
			target,
			series,
		],
	);
	const firstPrediction = predictions.reduce(
		(min, point) => Math.min(min, point.timestamp_ms),
		Number.POSITIVE_INFINITY,
	);
	const evaluationEnd = viewingEvaluation
		? predictions.reduce(
				(last, point) => Math.max(last, point.timestamp_ms),
				Number.NEGATIVE_INFINITY,
			)
		: Number.POSITIVE_INFINITY;
	const cutoff = history
		.filter((point) => point.timestamp_ms < firstPrediction)
		.reduce<number | undefined>(
			(max, point) =>
				max === undefined
					? point.timestamp_ms
					: Math.max(max, point.timestamp_ms),
			undefined,
		);
	const plot = useMemo(
		() =>
			buildForecastPlot(
				history
					.filter((point) => point.timestamp_ms <= evaluationEnd)
					.map((point) => ({
						timestamp: point.timestamp_ms,
						value: point.value,
					})),
				predictions.map((point) => ({
					timestamp: point.timestamp_ms,
					point: point.point,
					quantiles: point.quantiles,
				})),
				cutoff ?? Number.POSITIVE_INFINITY,
				result?.quantile_levels ?? [],
			),
		[history, predictions, cutoff, result?.quantile_levels, evaluationEnd],
	);
	const displayedPlot = useMemo(() => sampleForecastPlot(plot), [plot]);
	const openEvaluation = () => {
		if (!evaluate) {
			setEvaluate(true);
			setHoldout(
				String(
					Math.max(
						1,
						Math.min(
							Number(holdout),
							Number(horizon),
							backtestRange.maxHorizon,
						),
					),
				),
			);
			setBacktestOffset(0);
		}
		setView("evaluation");
	};
	const selectedMetrics = result?.backtest?.metrics.find(
		(metric) => metric.target === target && metric.series_id === series,
	);
	const measuredRange = useMemo(
		() =>
			result?.backtest
				? forecastBacktestRange(
						result.history,
						target,
						series,
						result.backtest.horizon,
						result.backtest.offset ?? 0,
					)
				: undefined,
		[result, target, series],
	);
	const timelineColumns = [
		...new Set([
			...covariateCandidates.map((column) => column.name),
			...(result?.covariates.map((point) => point.name) ?? []),
		]),
	].map((name) => ({ name }));
	const covariateTimelines: CovariateTimeline[] = timelineColumns.map(
		(column) => {
			const previewSeries = result
				? series
				: String(preview?.result.rows[0]?.[seriesId] ?? "");
			const values = result?.covariates.filter(
				(point) => point.series_id === series && point.name === column.name,
			);
			const timelineRole = values?.[0]?.kind ?? covariateRole(column.name);
			const points = values?.length
				? values.map((point) => ({
						timestamp: point.timestamp_ms,
						value: finiteValue(point.value),
					}))
				: (preview?.result.rows ?? [])
						.filter(
							(row) => !seriesId || String(row[seriesId]) === previewSeries,
						)
						.flatMap((row) => {
							const time = timestampValue(
								row[timestamp],
								timestampType,
								timestampFormat,
							);
							return time === null
								? []
								: [
										{
											timestamp: time,
											value: numericInputValue(row[column.name]),
										},
									];
						});
			return {
				name: column.name,
				role: timelineRole,
				enabled: Boolean(covariates[column.name]?.enabled),
				disabled:
					!covariateSupport[covariateRole(column.name)] &&
					!covariates[column.name]?.enabled,
				points: sampleTimeline(
					visibleCovariatePoints(
						points.filter((point) => point.timestamp <= evaluationEnd),
						timelineRole,
						viewingEvaluation,
						cutoff,
					).sort((a, b) => a.timestamp - b.timestamp),
					(point) => point.value,
					300,
				),
			};
		},
	);
	const toggleCovariate = (name: string, enabled: boolean) =>
		setCovariates((current) => ({
			...current,
			[name]: { role: covariateRole(name), enabled },
		}));
	const openSaved = (saved: SavedStudioForecast) => {
		previewSequence.current++;
		runSequence.current++;
		runInFlight.current = false;
		setRunning(false);
		setPreviewing(false);
		setPreview(null);
		setPreviewError(null);
		setRunError(null);
		const setup = restoreForecastSetup(saved.request);
		const request = setup.request;
		setModelId(request.model_bit_id);
		setSourceMode("sql");
		setSql(request.sql);
		setFutureMode("sql");
		setFutureSql(request.future_sql ?? "");
		setTimestamp(request.mapping.timestamp);
		setTimestampFormat(request.mapping.timestamp_format);
		setSeriesId(request.mapping.series_id ?? "");
		setTargets([...request.mapping.targets]);
		setCovariates(setup.covariates);
		setHorizon(String(request.horizon));
		setFrequency(
			request.mapping.frequency_ms ? String(request.mapping.frequency_ms) : "",
		);
		setEvaluate(Boolean(request.backtest_horizon));
		setHoldout(String(request.backtest_horizon || 12));
		setBacktestOffset(request.backtest_offset ?? 0);
		setRequestExtras(setup.extras);
		setRun({
			request,
			result: saved.result,
			fingerprint: forecastRequestFingerprint(request),
		});
		setActiveTarget(
			saved.result.forecasts[0]?.target ?? request.mapping.targets[0] ?? "",
		);
		setActiveSeries(saved.result.forecasts[0]?.series_id ?? "");
		setView("forecast");
		setSetupOpen(false);
		setSetupStep(3);
	};
	const newPrediction = () => {
		previewSequence.current++;
		runSequence.current++;
		runInFlight.current = false;
		setRun(null);
		setRunning(false);
		setRunError(null);
		setPreview(null);
		setPreviewing(false);
		setPreviewError(null);
		setModelId(forecastingModels[0]?.id ?? "");
		setTable(tables[0] ?? "");
		setSql(tableForecastSql(tables[0] ?? ""));
		setSourceMode("table");
		setFutureSql("");
		setFutureTable("");
		setFutureMode("table");
		setTimestamp("");
		setTimestampFormat("auto");
		setSeriesId("");
		setTargets([]);
		setCovariates({});
		setHorizon("24");
		setFrequency("");
		setEvaluate(false);
		setHoldout("12");
		setBacktestOffset(0);
		setRequestExtras({ quantile_levels: [] });
		setActiveTarget("");
		setActiveSeries("");
		setView("forecast");
		setSourceRevision((value) => value + 1);
		setSetupOpen(true);
		setSetupStep(0);
	};

	const sourcePanel = (
		<>
			<section className="space-y-4 rounded-xl border bg-card p-4">
				<SectionTitle
					icon={<Database className="size-3.5" />}
					title="Historical source"
				/>
				<ModeSwitch
					value={sourceMode}
					onChange={(mode) => {
						if (mode === "sql" && !sql) setSql(tableForecastSql(table));
						setSourceMode(mode);
					}}
				/>
				<p className="text-xs leading-relaxed text-muted-foreground">
					Use a table to forecast each machine or group separately. For an
					overall total or average, use SQL to aggregate by timestamp first.
					Choose the grouping column in the next step.
				</p>
				{sourceMode === "table" ? (
					<FieldSelect
						label="Historical table"
						value={table}
						onChange={(value) => {
							setTable(value);
							setSql(tableForecastSql(value));
						}}
						options={tables.map((name) => ({ value: name, label: name }))}
						placeholder="Choose a table"
					/>
				) : (
					<p className="text-xs leading-relaxed text-muted-foreground">
						Prepare your history with the SQL editor below. Join tables from
						this data scope.
					</p>
				)}
				{tables.length === 0 && (
					<p className="text-xs text-muted-foreground">
						There are no tables in this scope yet.
					</p>
				)}
				<div className="flex items-center justify-between gap-2">
					<span className="text-[11px] text-muted-foreground">
						{previewing
							? "Reading columns…"
							: sourceReady
								? `${columns.length} columns available`
								: "Source needs a preview"}
					</span>
					<Button
						size="sm"
						variant="ghost"
						className="h-7 gap-1 px-2 text-xs"
						disabled={!sourceSql.trim() || previewing}
						onClick={() => void loadSource()}
					>
						{previewing ? (
							<Loader2 className="size-3 animate-spin" />
						) : (
							<RefreshCw className="size-3" />
						)}
						Load columns
					</Button>
				</div>
				{previewError && (
					<p role="alert" className="break-words text-xs text-destructive">
						{previewError}
					</p>
				)}
			</section>
			{sourceMode === "sql" && (
				<section className="overflow-hidden rounded-xl border bg-card">
					<div className="flex items-center justify-between gap-3 border-b px-4 py-3">
						<div>
							<p className="flex items-center gap-2 text-xs font-medium">
								<SquareTerminal className="size-3.5" />
								Historical data query
							</p>
							<p className="mt-1 text-[11px] text-muted-foreground">
								Read-only DataFusion SQL. Use joins and aliases to prepare the
								columns you need.
							</p>
						</div>
						<Button
							size="sm"
							variant="outline"
							disabled={running || previewing || !sql.trim()}
							onClick={() => void loadSource()}
							className="h-8 gap-1.5 text-xs"
						>
							{previewing ? (
								<Loader2 className="size-3 animate-spin" />
							) : (
								<Play className="size-3" />
							)}
							Preview columns
						</Button>
					</div>
					<div className="h-44">
						<SqlEditor
							value={sql}
							onChange={setSql}
							catalog={catalog}
							readOnly={running}
							onRun={() => void loadSource()}
						/>
					</div>
				</section>
			)}
		</>
	);
	const mappingPanel = (
		<>
			<section className="space-y-4 rounded-xl border bg-card p-4">
				<SectionTitle
					icon={<SlidersHorizontal className="size-3.5" />}
					title="Column mapping"
				/>
				{(columns.length > 0 || timestamp || targets.length > 0) && (
					<>
						<FieldSelect
							label="Timestamp"
							value={timestamp}
							onChange={(value) => {
								setTimestamp(value);
								setTimestampFormat("auto");
								setTargets((current) =>
									current.filter((name) => name !== value),
								);
							}}
							options={[
								...new Set([
									...inferred.temporal.map((column) => column.name),
									...(timestamp ? [timestamp] : []),
								]),
							].map((name) => ({
								value: name,
								label: name,
							}))}
							placeholder="Choose a time column"
						/>
						<p className="-mt-2 text-[10px] text-muted-foreground">
							Native dates, date text, or Unix timestamps in seconds,
							milliseconds, microseconds, or nanoseconds.
						</p>
						{timestamp && !nativeTimestampUnit(timestampType ?? "") && (
							<FieldSelect
								label="Timestamp format"
								value={timestampFormat ?? "__legacy__"}
								onChange={(value) =>
									setTimestampFormat(
										value === "__legacy__"
											? undefined
											: (value as ForecastTimestampFormat),
									)
								}
								options={[
									...(timestampFormat === undefined
										? [
												{
													value: "__legacy__",
													label: "Saved interpretation (numeric milliseconds)",
												},
											]
										: []),
									...timestampFormatOptions,
								]}
							/>
						)}
						{timestamp && sourceReady && (
							<div
								className="space-y-2 rounded-lg border bg-muted/20 p-3"
								aria-label="Timestamp preview"
							>
								<p className="text-xs font-medium">
									{timestampInspection.label} · UTC
								</p>
								{timestampInspection.samples.map((sample, index) => (
									<p
										key={index}
										className="break-all font-mono text-[11px] text-muted-foreground"
									>
										{sample.raw} →{" "}
										{Math.abs(sample.timestamp) <= 8.64e15
											? new Date(sample.timestamp).toISOString()
											: String(sample.timestamp)}
									</p>
								))}
								{timestampInspection.error && (
									<p role="alert" className="text-xs text-destructive">
										{timestampInspection.error}
									</p>
								)}
								<p className="text-[10px] text-muted-foreground">
									Preview checks up to 500 rows. Dates without a timezone use
									UTC. All rows are validated when the forecast runs.
								</p>
							</div>
						)}
						<FieldSelect
							label="Series ID"
							value={seriesId || "__none__"}
							onChange={(value) => {
								const id = value === "__none__" ? "" : value;
								setSeriesId(id);
								setTargets((current) => withoutSeriesTarget(current, id));
							}}
							options={[
								{ value: "__none__", label: "Single series" },
								...(seriesId &&
								!columns.some((column) => column.name === seriesId)
									? [{ value: seriesId, label: seriesId }]
									: []),
								...columns
									.filter(
										(column) =>
											column.name !== timestamp &&
											/^(utf8|largeutf8|utf8view|u?int|string)/i.test(
												column.type_name,
											),
									)
									.map((column) => ({
										value: column.name,
										label: column.name,
									})),
							]}
						/>
						<div className="space-y-2 rounded-lg border border-primary/20 bg-primary/5 p-3">
							<p className="text-xs font-medium">
								{seriesId
									? `Forecast each ${seriesId} separately`
									: "Forecast one series"}
							</p>
							<p className="text-xs leading-relaxed text-muted-foreground">
								{seriesId
									? "Each distinct ID gets an independent prediction. Holdout evaluation measures accuracy separately for each group. All groups are included in the run and saved together."
									: "Rows are used as one series. For an overall total or average, aggregate the values by timestamp in your source SQL first."}
							</p>
							<p className="text-xs leading-relaxed text-muted-foreground">
								Choose a Series ID such as machine_id to split a shared table.
								Each group needs one row per timestamp. Aggregate or pivot
								multiple sensor readings at the same time before forecasting.
							</p>
							<Button
								type="button"
								variant="link"
								size="sm"
								className="h-auto p-0 text-xs"
								onClick={() => {
									if (sourceMode === "table") setSql(sourceSql);
									setSourceMode("sql");
									setSetupStep(0);
								}}
							>
								Edit source SQL
							</Button>
						</div>
						<div className="space-y-2">
							<Label className="text-xs">Metrics to forecast</Label>
							<div className="flex flex-wrap gap-1.5">
								{[
									...new Set([
										...inferred.numeric.map((column) => column.name),
										...targets,
									]),
								]
									.map((name) => ({ name }))
									.filter(
										(column) =>
											column.name !== timestamp && column.name !== seriesId,
									)
									.map((column) => (
										<button
											key={column.name}
											type="button"
											aria-pressed={targets.includes(column.name)}
											onClick={() =>
												setTargets((current) =>
													current.includes(column.name)
														? current.filter((name) => name !== column.name)
														: [...current, column.name],
												)
											}
											className={cn(
												"flex items-center gap-1 rounded-md border px-2 py-1.5 text-[11px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
												targets.includes(column.name)
													? "border-primary/30 bg-primary/10 text-primary"
													: "text-muted-foreground hover:bg-muted",
											)}
										>
											{targets.includes(column.name) && (
												<Check className="size-3" />
											)}
											{column.name}
											{inferred.numericText.some(
												(item) => item.name === column.name,
											) && (
												<span className="ml-1 text-[10px] opacity-70">
													numeric text
												</span>
											)}
										</button>
									))}
							</div>
							{sourceReady && metricCandidates.length === 0 && (
								<p className="text-xs leading-normal text-muted-foreground">
									No numeric metric was found. Text columns are accepted when
									every non-null preview value is a number.
								</p>
							)}
							{inferred.numericText.length > 0 && (
								<p className="text-[11px] text-muted-foreground">
									Numeric text is converted for this forecast. Source data stays
									unchanged; invalid values cause an error.
								</p>
							)}
							{sourceReady && metricCandidates.length === 0 && (
								<div
									className="text-[11px] text-muted-foreground"
									aria-label="Available source columns"
								>
									{columns
										.filter(
											(column) =>
												column.name !== timestamp && column.name !== seriesId,
										)
										.map((column) => (
											<p key={column.name}>
												{column.name} ({column.type_name}):{" "}
												{String(
													preview?.result.rows.find(
														(row) => row[column.name] != null,
													)?.[column.name] ?? "no values",
												).slice(0, 80)}
											</p>
										))}
								</div>
							)}
							{columns.some((column) => /^decimal/i.test(column.type_name)) && (
								<p className="text-[11px] leading-normal text-muted-foreground">
									Cast decimal metrics to DOUBLE in SQL to include them.
								</p>
							)}
						</div>
						<details className="text-xs">
							<summary className="cursor-pointer text-muted-foreground">
								Time interval override
							</summary>
							<div className="mt-2 space-y-1.5">
								<Label htmlFor="forecast-frequency" className="text-xs">
									Milliseconds per step
								</Label>
								<Input
									id="forecast-frequency"
									value={frequency}
									type="number"
									min={1}
									step={1}
									placeholder="Infer from timestamps"
									onChange={(event) => setFrequency(event.target.value)}
									className="h-8"
								/>
								<p className="text-[10px] text-muted-foreground">
									Timestamps must be regularly spaced within each series.
								</p>
							</div>
						</details>
					</>
				)}
			</section>
			{covariateCandidates.length > 0 && (
				<section className="space-y-3 rounded-xl border bg-card p-4">
					<SectionTitle icon={<ActivityIcon />} title="Covariates" />
					<p className="text-[11px] leading-relaxed text-muted-foreground">
						{canUseCovariates
							? "Add related numeric signals to the forecast. Known covariates also need values for future steps."
							: "This checkpoint does not accept covariates."}
						{!covariateSupport.verified &&
							" Availability is checked when the checkpoint runs."}
					</p>
					{covariateCandidates.map((column) => (
						<div key={column.name} className="flex items-center gap-2">
							<Switch
								checked={Boolean(covariates[column.name]?.enabled)}
								onCheckedChange={(enabled) =>
									toggleCovariate(column.name, enabled)
								}
								disabled={
									!covariateSupport[covariateRole(column.name)] &&
									!covariates[column.name]?.enabled
								}
								aria-label={`Include ${column.name}`}
							/>
							<span
								className="min-w-0 flex-1 truncate text-xs"
								title={column.name}
							>
								{column.name}
							</span>
							<Select
								value={covariateRole(column.name)}
								onValueChange={(value) =>
									setCovariates((current) => ({
										...current,
										[column.name]: {
											enabled: current[column.name]?.enabled ?? false,
											role: value === "known" ? "known" : "past",
										},
									}))
								}
								disabled={!canUseCovariates}
							>
								<SelectTrigger
									className="h-7 w-[100px] text-[10px]"
									aria-label={`Availability of ${column.name}`}
								>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="past" disabled={!covariateSupport.past}>
										Past only
									</SelectItem>
									<SelectItem value="known" disabled={!covariateSupport.known}>
										Known future
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
					))}
				</section>
			)}
			{knownCovariates.length > 0 && (
				<section className="space-y-3 rounded-xl border bg-card p-4">
					<div className="flex flex-wrap items-center justify-between gap-3">
						<div>
							<p className="text-xs font-medium">Known future values</p>
							<p className="mt-1 text-[11px] text-muted-foreground">
								One row per future step and series, with matching timestamp, ID
								and covariate names.
							</p>
						</div>
						<ModeSwitch value={futureMode} onChange={setFutureMode} />
					</div>
					{futureMode === "table" ? (
						<FieldSelect
							label="Future table"
							value={futureTable}
							onChange={setFutureTable}
							options={tables.map((name) => ({
								value: name,
								label: name,
							}))}
							placeholder="Choose future covariate values"
							disabled={running}
						/>
					) : (
						<div className="h-36 overflow-hidden rounded-md border">
							<SqlEditor
								value={futureSql}
								onChange={setFutureSql}
								catalog={catalog}
								readOnly={running}
							/>
						</div>
					)}
				</section>
			)}
		</>
	);
	const modelPanel = (
		<>
			<section className="space-y-4 rounded-xl border bg-card p-4">
				<SectionTitle
					icon={<SlidersHorizontal className="size-3.5" />}
					title="Model & horizon"
				/>
				<FieldSelect
					label="Forecasting model"
					value={modelId}
					onChange={setModelId}
					options={[
						...forecastingModels.map((item) => ({
							value: item.id,
							label:
								item.meta.en?.name ||
								Object.values(item.meta)[0]?.name ||
								item.id,
						})),
						...(modelId && !model
							? [{ value: modelId, label: `${modelId} (not active)` }]
							: []),
					]}
					placeholder="Choose a model"
				/>
				{modelMeta && (
					<div className="flex items-start gap-3 rounded-lg border bg-muted/20 p-3">
						{modelMeta.icon && (
							<img
								src={modelMeta.icon}
								alt=""
								className="size-10 shrink-0 rounded-lg"
							/>
						)}
						<div className="min-w-0">
							<p className="text-xs font-medium">{modelName}</p>
							<p className="mt-1 text-xs leading-relaxed text-muted-foreground">
								{modelMeta.description}
							</p>
						</div>
					</div>
				)}
				{forecastingModels.length === 0 && (
					<p className="text-xs text-muted-foreground">
						Add a Forecasting model to your profile in the model catalog.
					</p>
				)}
				<div className="grid grid-cols-2 gap-3">
					<div className="space-y-1.5">
						<Label htmlFor="forecast-horizon" className="text-xs">
							Future steps
						</Label>
						<Input
							id="forecast-horizon"
							type="number"
							min={1}
							max={65536}
							step={1}
							value={horizon}
							onChange={(event) => setHorizon(event.target.value)}
							className="h-9"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">Forecast output</Label>
						<div className="flex h-9 items-center rounded-md border bg-muted/30 px-3 text-xs text-muted-foreground">
							{forecastOutputLabel(requestExtras.quantile_levels, pointOnly)}
						</div>
					</div>
				</div>
				<p className="text-[11px] leading-relaxed text-muted-foreground">
					Steps follow the time interval in your data.{" "}
					{pointOnly
						? "This model returns a point forecast without a quantile band."
						: "Models that provide quantiles display them as a shaded band. Use holdout evaluation to measure accuracy."}
				</p>
				<div className="border-t pt-3">
					<div className="flex items-center justify-between gap-3">
						<Label
							htmlFor="forecast-evaluate"
							className="flex items-center gap-1.5 text-xs"
						>
							<FlaskConical className="size-3.5" />
							Evaluate on held-out data
						</Label>
						<Switch
							id="forecast-evaluate"
							checked={evaluate}
							onCheckedChange={setEvaluate}
						/>
					</div>
					{evaluate && (
						<div className="mt-3 space-y-1.5">
							<Label htmlFor="forecast-holdout" className="text-xs">
								Holdout steps
							</Label>
							<Input
								id="forecast-holdout"
								type="number"
								min={1}
								max={65536}
								step={1}
								value={holdout}
								onChange={(event) => setHoldout(event.target.value)}
								className="h-9"
							/>
							<p className="text-[11px] leading-relaxed text-muted-foreground">
								Predict this many observed steps using only earlier history.
								After a run, move the cutoff slider to try an earlier window.
							</p>
						</div>
					)}
				</div>
			</section>
		</>
	);

	return (
		<div className="h-full overflow-y-auto bg-muted/10">
			<div className="mx-auto max-w-[1600px] space-y-5 p-4 lg:p-6">
				<div className="flex flex-wrap items-center justify-between gap-3">
					<div>
						<h2 className="flex items-center gap-2 text-lg font-semibold tracking-tight">
							<TrendingUp className="size-5 text-primary" />
							Forecasting
						</h2>
						<p className="mt-1 text-xs text-muted-foreground">
							Explore future values and the uncertainty around them.
						</p>
					</div>
					<div className="flex flex-wrap items-center gap-2">
						{onAskFlowPilot && (
							<Button
								type="button"
								variant="outline"
								onClick={onAskFlowPilot}
								disabled={running || savedOpening}
							>
								<Sparkles className="size-3.5" />
								Ask FlowPilot
							</Button>
						)}
						<Select
							value={userScoped ? "user" : "project"}
							onValueChange={(value) => onScopeChange(value === "user")}
							disabled={running || savedOpening}
						>
							<SelectTrigger
								className="h-9 w-32"
								aria-label="Forecast data scope"
							>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="project">Project data</SelectItem>
								<SelectItem value="user">Personal data</SelectItem>
							</SelectContent>
						</Select>
						{!setupOpen && (
							<Button
								onClick={() => void runForecast()}
								disabled={Boolean(disabledReason) || running}
								className="gap-2"
								title={disabledReason ?? undefined}
							>
								{running ? (
									<Loader2 className="size-4 animate-spin" />
								) : (
									<Play className="size-3.5" />
								)}
								{running ? "Forecasting…" : "Run forecast"}
							</Button>
						)}
					</div>
				</div>
				<ForecastLibrary
					appId={appId}
					userScoped={userScoped}
					canWrite={canWrite}
					run={run}
					running={running}
					stale={stale}
					savedForecastId={savedForecastId}
					onSavedForecastChange={onSavedForecastChange}
					onOpen={openSaved}
					onNew={newPrediction}
					onOpeningChange={setSavedOpening}
				/>

				<div className="space-y-4">
					{setupOpen ? (
						<ForecastSetupWizard
							step={setupStep}
							onStepChange={setSetupStep}
							errors={[sourceError, mappingError, modelError, disabledReason]}
							busy={running || savedOpening || previewing}
							running={running}
							onRun={() => void runForecast()}
							onClose={run ? () => setSetupOpen(false) : undefined}
						>
							{setupStep === 0 && sourcePanel}
							{setupStep === 1 && mappingPanel}
							{setupStep === 2 && modelPanel}
							{setupStep === 3 && (
								<ForecastSetupReview
									request={payload}
									modelName={modelName}
									userScoped={userScoped}
									pointOnly={pointOnly}
								/>
							)}
						</ForecastSetupWizard>
					) : (
						<div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3">
							<p className="min-w-0 break-words text-xs text-muted-foreground">
								{targets.join(", ")} · {modelName} · {horizon} future steps
								{evaluate && <> · {holdout} holdout steps</>}
							</p>
							<Button
								variant="outline"
								size="sm"
								disabled={running || savedOpening}
								onClick={() => {
									setSetupStep(0);
									setSetupOpen(true);
									if (!sourceReady) void loadSource();
								}}
							>
								Edit setup
							</Button>
						</div>
					)}
					{(run || runError) && (
						<div className="min-w-0 space-y-4">
							{runError && (
								<div
									role="alert"
									className="rounded-lg border border-destructive/20 bg-destructive/5 px-4 py-3 text-sm text-destructive"
								>
									<p className="font-medium">The forecast could not run</p>
									<p className="mt-1 break-words text-xs leading-relaxed">
										{runError}
									</p>
								</div>
							)}
							<div className="flex flex-wrap items-center justify-between gap-3">
								<div className="flex min-w-0 flex-wrap gap-1">
									{resultTargets.map((name) => (
										<button
											key={name}
											type="button"
											onClick={() => setActiveTarget(name)}
											aria-pressed={target === name}
											className={cn(
												"rounded-lg border px-3 py-2 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
												target === name
													? "border-foreground/10 bg-foreground text-background"
													: "border-transparent text-muted-foreground hover:bg-muted",
											)}
										>
											{name}
										</button>
									))}
								</div>
								<div className="flex flex-wrap items-center gap-2">
									{resultSeries.length > 1 && (
										<div className="flex items-center gap-2 text-xs text-muted-foreground">
											<span>View group</span>
											<Select
												value={JSON.stringify(series)}
												onValueChange={(value) =>
													setActiveSeries(JSON.parse(value) as string)
												}
											>
												<SelectTrigger
													className="h-8 max-w-40 text-xs"
													aria-label="Chart series"
												>
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													{resultSeries.map((id) => (
														<SelectItem key={id} value={JSON.stringify(id)}>
															{id || "(empty ID)"}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
									)}
									{result && (
										<fieldset
											className="flex rounded-lg border bg-card p-0.5"
											aria-label="Forecast view"
										>
											<Button
												variant={view === "forecast" ? "secondary" : "ghost"}
												size="sm"
												className="h-7 px-2 text-[11px]"
												aria-pressed={view === "forecast"}
												onClick={() => setView("forecast")}
											>
												Outlook
											</Button>
											<Button
												variant={view === "evaluation" ? "secondary" : "ghost"}
												size="sm"
												className="h-7 px-2 text-[11px]"
												aria-pressed={view === "evaluation"}
												disabled={running}
												onClick={openEvaluation}
											>
												Holdout
											</Button>
										</fieldset>
									)}
								</div>
							</div>
							{view === "evaluation" && (
								<ForecastBacktestControls
									range={backtestRange}
									offset={backtestOffset}
									horizon={Number(holdout)}
									disabled={
										Boolean(disabledReason) || running || !retainedSource
									}
									reason={
										disabledReason ||
										(!retainedSource
											? "Run the current setup to load its full history before moving the cutoff."
											: undefined)
									}
									running={running}
									onHorizonChange={(value) => {
										setHoldout(String(value));
										setBacktestOffset(0);
									}}
									onChange={setBacktestOffset}
									onCommit={(offset) => {
										setBacktestOffset(offset);
										void runForecast(offset);
									}}
								/>
							)}
							{view === "evaluation" && !result?.backtest && (
								<p className="px-1 text-xs text-muted-foreground">
									Choose a holdout window and evaluate it to compare predictions
									with observed values. The chart shows your future outlook
									until the evaluation finishes.
								</p>
							)}
							<ForecastChart
								points={displayedPlot}
								cutoff={cutoff}
								metric={target}
								seriesLabel={
									run?.request.mapping.series_id || resultSeries.length > 1
										? series || "(empty ID)"
										: undefined
								}
								seriesColumn={run?.request.mapping.series_id ?? undefined}
								seriesCount={resultSeries.length}
								hasEvaluationMetrics={
									Boolean(selectedMetrics) && !stale && !running
								}
								quantileLevels={result?.quantile_levels ?? []}
								pointStatistic={result?.point_statistic}
								loading={running}
								stale={stale}
								failedRefresh={runError !== null}
								backtest={viewingEvaluation}
								covariates={covariateTimelines}
								onCovariateToggle={toggleCovariate}
								canUseCovariates={canUseCovariates}
							/>
							{!result && covariateTimelines.length > 0 && (
								<p className="px-1 text-[10px] leading-normal text-muted-foreground">
									Covariate previews show up to 500 source rows
									{seriesId ? " from the first series" : ""}. Forecasting reads
									the full query within the runtime limits.
								</p>
							)}
							{displayedPlot.length < plot.length && (
								<p className="px-1 text-[10px] leading-normal text-muted-foreground">
									The plot is reduced for display, preserving extremes.
									Evaluation uses all observed holdout values.
								</p>
							)}
							{result && (
								<div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 px-1 text-[11px] text-muted-foreground">
									<span>
										{result.input_rows.toLocaleString()} input rows{" "}
										<span className="px-1">·</span> {resultSeries.length} series{" "}
										<span className="px-1">·</span> {result.horizon} future
										steps
									</span>
									<span className="flex items-center gap-1.5">
										<span className="size-1.5 rounded-full bg-primary" />
										{result.model.family}{" "}
										<span className="text-muted-foreground/50">/</span>{" "}
										{result.point_statistic === "mean" ? "Mean" : "Median"}{" "}
										forecast
									</span>
								</div>
							)}
							{selectedMetrics && !stale && !running ? (
								<div className="space-y-2">
									{measuredRange?.cutoff !== undefined &&
										measuredRange.start !== undefined &&
										measuredRange.end !== undefined && (
											<p className="px-1 text-xs leading-relaxed text-muted-foreground">
												Evaluated{" "}
												{formatForecastTime(measuredRange.start, true)} to{" "}
												{formatForecastTime(measuredRange.end, true)} UTC ·{" "}
												{result?.backtest?.horizon} steps. History ended{" "}
												{formatForecastTime(measuredRange.cutoff, true)} UTC.
											</p>
										)}
									<ForecastMetrics metrics={selectedMetrics} />
								</div>
							) : (
								<div
									className="flex items-start gap-3 rounded-xl border border-dashed p-4 text-xs text-muted-foreground"
									aria-live="polite"
								>
									<FlaskConical className="mt-0.5 size-4 shrink-0" />
									<p className="leading-relaxed">
										{running
											? "Accuracy metrics will appear when this evaluation finishes."
											: stale
												? "Run the selected setup or release the cutoff slider to refresh the accuracy metrics."
												: "Enable holdout evaluation to compare predictions with values already in your history."}
									</p>
								</div>
							)}
							{disabledReason && !running && (
								<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
									<ArrowRight className="mt-0.5 size-3 shrink-0" />
									{disabledReason}
								</p>
							)}
							<output aria-live="polite" className="sr-only">
								{running
									? "Running forecast"
									: runError
										? "Forecast failed"
										: result
											? `Forecast ready with ${result.forecasts.length} predictions${stale ? ". Settings have changed since this run." : ""}`
											: "Configure a source and model to run a forecast"}
							</output>
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

function SectionTitle({
	icon,
	title,
}: { icon: React.ReactNode; title: string }) {
	return (
		<h3 className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
			{icon}
			{title}
		</h3>
	);
}

function FieldSelect({
	label,
	value,
	onChange,
	options,
	placeholder,
	disabled,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	options: { value: string; label: string }[];
	placeholder?: string;
	disabled?: boolean;
}) {
	return (
		<div className="space-y-1.5">
			<Label className="text-xs">{label}</Label>
			<Select value={value} onValueChange={onChange} disabled={disabled}>
				<SelectTrigger className="h-9 text-xs" aria-label={label}>
					<SelectValue placeholder={placeholder} />
				</SelectTrigger>
				<SelectContent>
					{options.map((option) => (
						<SelectItem key={option.value} value={option.value}>
							{option.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}

function ModeSwitch({
	value,
	onChange,
}: { value: "table" | "sql"; onChange: (value: "table" | "sql") => void }) {
	return (
		<div className="inline-flex rounded-lg bg-muted p-0.5">
			{(["table", "sql"] as const).map((mode) => (
				<button
					key={mode}
					type="button"
					onClick={() => onChange(mode)}
					aria-pressed={value === mode}
					className={cn(
						"flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
						value === mode
							? "bg-background text-foreground shadow-sm"
							: "text-muted-foreground",
					)}
				>
					{mode === "table" ? (
						<Database className="size-3" />
					) : (
						<SquareTerminal className="size-3" />
					)}
					{mode === "table" ? "Table" : "SQL query"}
				</button>
			))}
		</div>
	);
}

function ActivityIcon() {
	return <SlidersHorizontal className="size-3.5" />;
}
