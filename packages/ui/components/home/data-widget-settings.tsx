"use client";

import { Loader2, Plus, Trash2 } from "lucide-react";
import {
	type ReactNode,
	cloneElement,
	isValidElement,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../lib/api-url";
import { useBackend } from "../../state/backend-state";
import type { GraphOverlay } from "../../state/backend-state/graph-state";
import type {
	QueryColumn,
	SavedQuery,
} from "../../state/backend-state/query-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import { HomeDataWidget } from "./data-widget";
import {
	HOME_DATA_AGGREGATIONS,
	HOME_DATA_FILTER_OPERATORS,
	HOME_DATA_VISUALIZATIONS,
	type HomeDataConfig,
	extractHomeQueryParameters,
	homeDataColumns,
	homeDataMeasureTitle,
	homeOntologyColumns,
	homeSavedQuerySql,
	normalizeHomeDataConfig,
	resolveHomeQueryParams,
	updateHomeDataMeasure,
} from "./home-data-query";
import type { IHomeWidget } from "./types";

const selectClass =
	"h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm";
function Field({
	label,
	children,
	hint,
}: { label: string; children: ReactNode; hint?: string }) {
	const id = useId();
	return (
		<div className="flex min-w-0 flex-col gap-1.5 text-sm">
			<label htmlFor={id} className="text-xs font-medium">
				{label}
			</label>
			{isValidElement<Record<string, unknown>>(children)
				? cloneElement(children, { id })
				: children}
			{hint && (
				<span className="text-xs font-normal text-muted-foreground">
					{hint}
				</span>
			)}
		</div>
	);
}
function Choice({
	label,
	value,
	options,
	onChange,
	optional,
	hint,
}: {
	label: string;
	value: string;
	options: readonly (readonly [string, string])[];
	onChange: (value: string) => void;
	optional?: string;
	hint?: string;
}) {
	return (
		<Field label={label} hint={hint}>
			<select
				className={selectClass}
				value={value}
				onChange={(event) => onChange(event.target.value)}
			>
				{optional !== undefined && <option value="">{optional}</option>}
				{options.map(([id, name]) => (
					<option key={id} value={id}>
						{name}
					</option>
				))}
			</select>
		</Field>
	);
}
const filterNames = {
	eq: "Equals",
	neq: "Does not equal",
	gt: "Greater than",
	gte: "At least",
	lt: "Less than",
	lte: "At most",
	contains: "Contains text",
	empty: "Is null",
	not_empty: "Is not null",
};

export function HomeDataWidgetSettings({
	widget,
	onChange,
}: {
	widget: IHomeWidget;
	onChange: (config: Record<string, unknown>) => void;
}) {
	const backend = useBackend();
	const auth = useAuth();
	const config = normalizeHomeDataConfig(widget.config);
	const patch = (updates: Partial<HomeDataConfig>) =>
		onChange({ ...config, ...updates });
	const [apps, setApps] = useState<[string, string][]>([]);
	const [tables, setTables] = useState<string[]>([]);
	const [ontologies, setOntologies] = useState<GraphOverlay[]>([]);
	const [queries, setQueries] = useState<SavedQuery[]>([]);
	const [columns, setColumns] = useState<QueryColumn[]>([]);
	const [loading, setLoading] = useState(false);
	const [sourceError, setSourceError] = useState<string | null>(null);
	const [columnError, setColumnError] = useState<string | null>(null);
	const [preview, setPreview] = useState(false);
	const personal = config.scope === "personal";
	const sourceIdentity = JSON.stringify([
		getApiOrigin(backend.profile),
		backend.profile?.id,
		auth?.user?.profile?.sub,
	]);
	const overlay = ontologies.find((item) => item.id === config.ontologyId);
	const savedQuery = queries.find((item) => item.id === config.queryId);
	const paramSignature = JSON.stringify(config.queryParams);
	const stableParams = useMemo(
		() => JSON.parse(paramSignature) as Record<string, unknown>,
		[paramSignature],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: A profile switch must refresh available apps even when its backend instance is reused.
	useEffect(() => {
		let active = true;
		void backend.appState
			.getApps()
			.then((items) => {
				if (active)
					setApps(items.map(([app, meta]) => [app.id, meta?.name || app.id]));
			})
			.catch(() => {
				if (active) setSourceError("Your apps could not be loaded.");
			});
		return () => {
			active = false;
		};
	}, [backend.appState, sourceIdentity]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Sources must reload when a reused backend changes profiles.
	useEffect(() => {
		let active = true;
		setTables([]);
		setOntologies([]);
		setQueries([]);
		setSourceError(null);
		if (!config.appId) return;
		setLoading(true);
		void (async () => {
			try {
				if (config.sourceKind === "table") {
					const result = personal
						? await backend.dbState.listTablesUser(config.appId)
						: await backend.dbState.listTables(config.appId);
					if (active) setTables(result);
				} else if (config.sourceKind === "ontology") {
					const result = await backend.graphState.listOverlays(
						config.appId,
						personal,
					);
					if (active) setOntologies(result);
				} else {
					const result = await backend.queryState.listSavedQueries(
						config.appId,
						personal,
					);
					if (active) setQueries(result);
				}
			} catch (reason) {
				if (active)
					setSourceError(
						reason instanceof Error
							? reason.message
							: "These sources are unavailable. Check your access to the app.",
					);
			} finally {
				if (active) setLoading(false);
			}
		})();
		return () => {
			active = false;
		};
	}, [
		config.appId,
		config.sourceKind,
		personal,
		backend.dbState,
		backend.graphState,
		backend.queryState,
		sourceIdentity,
	]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Field metadata must reload when the hub or viewer changes on a reused backend.
	useEffect(() => {
		let active = true;
		setColumns([]);
		setColumnError(null);
		if (!config.appId) return;
		if (config.sourceKind === "ontology") {
			if (overlay) setColumns(homeOntologyColumns(overlay, config.objectType));
			return;
		}
		if (config.sourceKind === "table" && !config.table) return;
		if (config.sourceKind === "query" && !savedQuery) return;
		const timer = setTimeout(() => {
			void (async () => {
				try {
					if (config.sourceKind === "table") {
						const schema = await backend.dbState.getSchema(
							config.appId,
							config.table,
							personal,
						);
						if (active) setColumns(homeDataColumns(schema));
					} else if (savedQuery) {
						const result = await backend.queryState.executeSql(
							config.appId,
							{
								sql: `SELECT * FROM (\n${homeSavedQuerySql(savedQuery.sql)}\n) AS "__home_schema" WHERE false`,
								params: resolveHomeQueryParams(
									{ queryParams: stableParams },
									auth?.user?.profile?.sub,
								),
								surface: savedQuery.surface,
								overlay_id: savedQuery.overlay_id,
								limit: 1,
							},
							personal,
						);
						if (active) setColumns(result.columns);
					}
				} catch (reason) {
					if (active)
						setColumnError(
							reason instanceof Error
								? reason.message
								: "Fields could not be loaded.",
						);
				}
			})();
		}, 250);
		return () => {
			active = false;
			clearTimeout(timer);
		};
	}, [
		config.appId,
		config.sourceKind,
		config.table,
		config.objectType,
		overlay,
		savedQuery,
		stableParams,
		personal,
		auth?.user?.profile?.sub,
		backend.dbState,
		backend.queryState,
		sourceIdentity,
	]);
	const fields: [string, string][] = columns.map((column) => [
		column.name,
		column.name,
	]);
	const numeric: [string, string][] = columns
		.filter((column) =>
			/int|float|double|decimal|numeric|number|real/i.test(column.type_name),
		)
		.map((column) => [column.name, column.name]);
	const resetSource = {
		table: "",
		ontologyId: "",
		objectType: "",
		queryId: "",
		groupBy: "",
		seriesBy: "",
		fields: [],
		filters: [],
		dateField: "",
		xField: "",
		yField: "",
		queryParams: {},
		dateRange: "all" as const,
		measures: [
			{
				id: "measure-count",
				aggregation: "count" as const,
				field: "",
				label: "Count",
			},
		],
	};
	const properties = savedQuery?.param_schema?.properties as
		| Record<string, { type?: string; title?: string }>
		| undefined;
	const parameterNames = savedQuery
		? extractHomeQueryParameters(savedQuery.sql)
		: [];

	return (
		<div className="min-w-0 space-y-5">
			<p className="text-sm text-muted-foreground">
				Choose a data source, then shape it with measures, groups, and filters.
			</p>
			<div className="space-y-3 rounded-xl border border-border/60 bg-muted/10 p-3">
				<p className="text-xs font-semibold">Data source</p>
				<Choice
					label="App"
					value={config.appId}
					options={apps}
					optional="Choose an app"
					onChange={(appId) => patch({ ...resetSource, appId })}
				/>
				<div className="grid grid-cols-2 gap-3">
					<Choice
						label="Source"
						value={config.sourceKind}
						options={[
							["table", "Native table"],
							["ontology", "Ontology objects"],
							["query", "Saved query or view"],
						]}
						onChange={(value) =>
							patch({
								...resetSource,
								sourceKind: value as HomeDataConfig["sourceKind"],
							})
						}
					/>
					<Choice
						label="Database"
						value={config.scope}
						options={[
							["project", "Project data"],
							["personal", "Viewer's personal data"],
						]}
						onChange={(value) =>
							patch({ ...resetSource, scope: value as HomeDataConfig["scope"] })
						}
					/>
				</div>
				{loading && (
					<p className="flex items-center gap-2 text-xs text-muted-foreground">
						<Loader2 className="size-3 animate-spin" />
						Loading sources…
					</p>
				)}
				{sourceError && (
					<p role="alert" className="text-sm text-destructive">
						{sourceError}
					</p>
				)}
				{config.sourceKind === "table" && (
					<Choice
						label="Table"
						value={config.table}
						options={tables.map((table) => [table, table])}
						optional="Choose a table"
						onChange={(table) => patch({ ...resetSource, table })}
					/>
				)}
				{config.sourceKind === "ontology" && (
					<>
						<Choice
							label="Ontology"
							value={config.ontologyId}
							options={ontologies.map((item) => [item.id, item.name])}
							optional="Choose an ontology"
							onChange={(ontologyId) => patch({ ...resetSource, ontologyId })}
						/>
						<Choice
							label="Object type"
							value={config.objectType}
							options={
								overlay?.nodes.map((item) => [item.label, item.label]) ?? []
							}
							optional="Choose an object type"
							onChange={(objectType) =>
								patch({
									objectType,
									groupBy: "",
									seriesBy: "",
									fields: [],
									filters: [],
								})
							}
						/>
					</>
				)}
				{config.sourceKind === "query" && (
					<>
						<Choice
							label="Saved query or view"
							hint="Measures use this query's result, including any filters or LIMIT in its saved SQL."
							value={config.queryId}
							options={queries.map((item) => [
								item.id,
								`${item.name}${item.kind === "view" ? " (view)" : ""}`,
							])}
							optional="Choose a saved query"
							onChange={(queryId) => patch({ ...resetSource, queryId })}
						/>
						{parameterNames.map((name) => (
							<div className="grid grid-cols-2 gap-2" key={name}>
								<Choice
									label={properties?.[name]?.title || name}
									value={
										config.queryParams[name] === "$viewer.id"
											? "viewer"
											: "value"
									}
									options={[
										["value", "Fixed value"],
										["viewer", "Current user's ID"],
									]}
									onChange={(value) =>
										patch({
											queryParams: {
												...config.queryParams,
												[name]: value === "viewer" ? "$viewer.id" : "",
											},
										})
									}
								/>
								{config.queryParams[name] !== "$viewer.id" &&
									properties?.[name]?.type === "boolean" && (
										<Choice
											label="Parameter value"
											value={
												config.queryParams[name] === undefined
													? ""
													: String(config.queryParams[name])
											}
											options={[
												["true", "True"],
												["false", "False"],
											]}
											optional="Choose"
											onChange={(value) =>
												patch({
													queryParams: {
														...config.queryParams,
														[name]: value === "true",
													},
												})
											}
										/>
									)}
								{config.queryParams[name] !== "$viewer.id" &&
									properties?.[name]?.type !== "boolean" && (
										<Field label="Parameter value">
											<Input
												value={String(config.queryParams[name] ?? "")}
												onChange={(event) => {
													const value = event.target.value;
													const type = properties?.[name]?.type;
													patch({
														queryParams: {
															...config.queryParams,
															[name]:
																type === "number" || type === "integer"
																	? Number(value)
																	: type === "boolean"
																		? value === "true"
																		: value,
														},
													});
												}}
											/>
										</Field>
									)}
							</div>
						))}
					</>
				)}
				{columnError && (
					<p role="alert" className="text-xs text-destructive">
						{columnError}
					</p>
				)}
			</div>
			<Choice
				label="Presentation"
				value={config.visualization}
				options={HOME_DATA_VISUALIZATIONS}
				onChange={(value) => {
					const visualization = value as HomeDataConfig["visualization"];
					const records = [
						"table",
						"list",
						"cards",
						"kanban",
						"record",
						"graph",
						"scatter",
						"timeline",
						"recordcalendar",
						"comparison",
					].includes(visualization);
					patch({
						visualization,
						mode: records ? "records" : "aggregate",
						...(visualization === "calendar"
							? {
									timeBucket: "day",
									sortBy: "group",
									sortDirection: "asc",
									limit: 366,
								}
							: {}),
						...(visualization === "line" || visualization === "area"
							? { sortBy: "group", sortDirection: "asc" }
							: {}),
					});
				}}
			/>
			{["table", "list", "cards"].includes(config.visualization) && (
				<Choice
					label="Show"
					value={config.mode}
					options={[
						["records", "Source records"],
						["aggregate", "Aggregated results"],
					]}
					onChange={(mode) => patch({ mode: mode as HomeDataConfig["mode"] })}
				/>
			)}
			{config.mode === "aggregate" && config.visualization !== "boxplot" && (
				<div className="space-y-3 rounded-lg border p-3">
					<div className="flex items-center justify-between">
						<span className="text-sm font-medium">Measures</span>
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={config.measures.length >= 6}
							onClick={() =>
								patch({
									measures: [
										...config.measures,
										{
											id: crypto.randomUUID(),
											aggregation: "count",
											field: "",
											label: "Count",
										},
									],
								})
							}
						>
							<Plus className="size-3" />
							Measure
						</Button>
					</div>
					{config.measures.map((measure, index) => (
						<div
							className="space-y-2 border-b pb-3 last:border-b-0 last:pb-0"
							key={measure.id}
						>
							<div className="grid grid-cols-2 gap-2">
								<Choice
									label="Aggregation"
									value={measure.aggregation}
									options={HOME_DATA_AGGREGATIONS.map((value) => [
										value,
										value === "avg"
											? "Average"
											: value === "distinct"
												? "Distinct count"
												: value === "count" && config.sourceKind === "ontology"
													? "Count objects"
													: value[0].toUpperCase() + value.slice(1),
									])}
									onChange={(aggregation) =>
										patch({
											measures: config.measures.map((item, i) =>
												i === index
													? updateHomeDataMeasure(item, {
															aggregation:
																aggregation as typeof measure.aggregation,
														})
													: item,
											),
										})
									}
								/>
								{measure.aggregation !== "count" && (
									<Choice
										label="Field"
										value={measure.field}
										options={
											measure.aggregation === "distinct" ||
											measure.aggregation === "min" ||
											measure.aggregation === "max"
												? fields
												: numeric.length
													? numeric
													: fields
										}
										optional="Choose a field"
										onChange={(field) =>
											patch({
												measures: config.measures.map((item, i) =>
													i === index
														? updateHomeDataMeasure(item, { field })
														: item,
												),
											})
										}
									/>
								)}
							</div>
							<div className="flex items-end gap-2">
								<div className="min-w-0 flex-1">
									<Field label="Label">
										<Input
											value={measure.label}
											placeholder={homeDataMeasureTitle({
												...measure,
												label: "",
											})}
											onChange={(event) =>
												patch({
													measures: config.measures.map((item, i) =>
														i === index
															? { ...item, label: event.target.value }
															: item,
													),
												})
											}
										/>
									</Field>
								</div>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									aria-label={`Remove measure ${index + 1}`}
									disabled={config.measures.length === 1}
									onClick={() =>
										patch({
											measures: config.measures.filter((_, i) => i !== index),
										})
									}
								>
									<Trash2 className="size-4" />
								</Button>
							</div>
						</div>
					))}
				</div>
			)}
			{config.visualization === "boxplot" && (
				<Choice
					label="Distribution field"
					value={config.yField}
					options={numeric.length ? numeric : fields}
					optional="Choose a numeric field"
					onChange={(yField) => patch({ yField })}
				/>
			)}
			{["funnel", "waterfall"].includes(config.visualization) && (
				<Field
					label="Category order (optional)"
					hint="One group value per line. Other groups follow the query's sort order."
				>
					<Textarea
						value={config.categoryOrder.join("\n")}
						onChange={(event) =>
							patch({ categoryOrder: event.target.value.split("\n") })
						}
					/>
				</Field>
			)}
			{config.visualization === "waterfall" && (
				<Field label="Starting value">
					<Input
						type="number"
						value={config.baseline}
						onChange={(event) =>
							patch({ baseline: Number(event.target.value) })
						}
					/>
				</Field>
			)}
			{["timeline", "recordcalendar"].includes(config.visualization) && (
				<Choice
					label="Record date"
					value={config.xField}
					options={fields}
					optional="Choose a date column"
					onChange={(xField) => patch({ xField })}
				/>
			)}
			{(config.mode === "aggregate" || config.visualization === "kanban") && (
				<>
					<Choice
						label={
							config.visualization === "histogram"
								? "Numeric field to bin"
								: config.visualization === "kanban"
									? "Status column"
									: "Group by"
						}
						value={config.groupBy}
						options={fields}
						optional="No grouping"
						onChange={(groupBy) => patch({ groupBy })}
					/>
					{config.mode === "aggregate" &&
						config.groupBy &&
						config.visualization !== "histogram" && (
							<Choice
								label="Date grouping"
								value={config.timeBucket}
								options={[
									["none", "Use original value"],
									["day", "Day"],
									["week", "Week"],
									["month", "Month"],
									["quarter", "Quarter"],
									["year", "Year"],
								]}
								hint="Date grouping expects a timestamp, date, or ISO date string. Dates are grouped in UTC."
								onChange={(timeBucket) =>
									patch({
										timeBucket: timeBucket as HomeDataConfig["timeBucket"],
									})
								}
							/>
						)}
					{config.mode === "aggregate" && (
						<Choice
							label="Split into series"
							value={config.seriesBy}
							options={fields}
							optional="No series"
							onChange={(seriesBy) => patch({ seriesBy })}
						/>
					)}
				</>
			)}
			{config.visualization === "histogram" && (
				<Field label="Bin width">
					<Input
						type="number"
						min="0.001"
						step="any"
						value={config.binWidth}
						onChange={(event) =>
							patch({ binWidth: Number(event.target.value) })
						}
					/>
				</Field>
			)}
			{["graph", "scatter"].includes(config.visualization) && (
				<div className="grid grid-cols-2 gap-3">
					<Choice
						label={
							config.visualization === "graph" ? "Source ID column" : "X field"
						}
						value={config.xField}
						options={fields}
						optional="Choose a field"
						onChange={(xField) => patch({ xField })}
					/>
					<Choice
						label={
							config.visualization === "graph" ? "Target ID column" : "Y field"
						}
						value={config.yField}
						options={fields}
						optional="Choose a field"
						onChange={(yField) => patch({ yField })}
					/>
				</div>
			)}
			{config.mode === "records" && (
				<fieldset className="space-y-2">
					<legend className="mb-2 text-sm font-medium">Visible fields</legend>
					<p className="text-xs text-muted-foreground">
						Leave all unchecked to show every field. The first selected field
						titles each record.
					</p>
					<div className="flex max-h-36 flex-wrap gap-2 overflow-auto">
						{fields.map(([name]) => (
							<label
								key={name}
								className="flex items-center gap-1.5 rounded border px-2 py-1 text-xs"
							>
								<input
									type="checkbox"
									checked={config.fields.includes(name)}
									onChange={(event) =>
										patch({
											fields: event.target.checked
												? [...config.fields, name]
												: config.fields.filter((field) => field !== name),
										})
									}
								/>
								{name}
							</label>
						))}
					</div>
				</fieldset>
			)}
			<div className="space-y-3 rounded-lg border p-3">
				<div className="flex items-center justify-between">
					<span className="text-sm font-medium">Filters</span>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={config.filters.length >= 12}
						onClick={() =>
							patch({
								filters: [
									...config.filters,
									{
										id: crypto.randomUUID(),
										field: "",
										operator: "eq",
										value: "",
										valueType: "text",
									},
								],
							})
						}
					>
						<Plus className="size-3" />
						Filter
					</Button>
				</div>
				{config.filters.map((filter, index) => {
					const update = (changes: Partial<typeof filter>) =>
						patch({
							filters: config.filters.map((item, i) =>
								i === index ? { ...item, ...changes } : item,
							),
						});
					return (
						<div key={filter.id} className="space-y-2 rounded bg-muted/40 p-2">
							<div className="grid grid-cols-2 gap-2">
								<Choice
									label="Field"
									value={filter.field}
									options={fields}
									optional="Choose a field"
									onChange={(field) => update({ field })}
								/>
								<Choice
									label="Condition"
									value={filter.operator}
									options={HOME_DATA_FILTER_OPERATORS.map((value) => [
										value,
										filterNames[value],
									])}
									onChange={(operator) =>
										update({ operator: operator as typeof filter.operator })
									}
								/>
							</div>
							{!["empty", "not_empty"].includes(filter.operator) && (
								<div className="grid grid-cols-2 gap-2">
									<Choice
										label="Compare with"
										value={filter.valueType}
										options={[
											["text", "Text"],
											["number", "Number"],
											["boolean", "True / false"],
											["viewer", "Current user's ID"],
										]}
										onChange={(valueType) =>
											update({
												valueType: valueType as typeof filter.valueType,
											})
										}
									/>
									{filter.valueType === "boolean" ? (
										<Choice
											label="Value"
											value={filter.value}
											options={[
												["true", "True"],
												["false", "False"],
											]}
											optional="Choose"
											onChange={(value) => update({ value })}
										/>
									) : (
										filter.valueType !== "viewer" && (
											<Field label="Value">
												<Input
													value={filter.value}
													onChange={(event) =>
														update({ value: event.target.value })
													}
												/>
											</Field>
										)
									)}
								</div>
							)}
							<Button
								type="button"
								variant="ghost"
								size="sm"
								onClick={() =>
									patch({
										filters: config.filters.filter((_, i) => i !== index),
									})
								}
							>
								<Trash2 className="size-3" />
								Remove filter
							</Button>
						</div>
					);
				})}
				<p className="text-xs text-muted-foreground">
					All conditions must match. “Current user's ID” resolves when each
					person opens their home.
				</p>
			</div>
			<div className="grid grid-cols-2 gap-3">
				<Choice
					label="Time range"
					value={config.dateRange}
					options={[
						["all", "All time"],
						["7d", "Last 7 days"],
						["30d", "Last 30 days"],
						["90d", "Last 90 days"],
						["year", "This year (UTC)"],
					]}
					onChange={(dateRange) =>
						patch({ dateRange: dateRange as HomeDataConfig["dateRange"] })
					}
				/>
				{config.dateRange !== "all" && (
					<Choice
						label="Date field"
						value={config.dateField}
						options={fields}
						optional="Choose a date field"
						onChange={(dateField) => patch({ dateField })}
					/>
				)}
				<Choice
					label="Sort by"
					value={config.sortBy}
					options={
						config.mode === "aggregate"
							? [
									["value", "First measure"],
									["group", "Group value"],
								]
							: [["", "Source order"], ...fields]
					}
					onChange={(sortBy) => patch({ sortBy })}
				/>
				<Choice
					label="Order"
					value={config.sortDirection}
					options={[
						["desc", "Descending"],
						["asc", "Ascending"],
					]}
					onChange={(sortDirection) =>
						patch({
							sortDirection: sortDirection as HomeDataConfig["sortDirection"],
						})
					}
				/>
				<Field label="Result limit">
					<Input
						type="number"
						min={1}
						max={500}
						value={config.limit}
						onChange={(event) => patch({ limit: Number(event.target.value) })}
					/>
				</Field>
				<Choice
					label="Refresh"
					value={String(config.refreshSeconds)}
					options={[
						["0", "On open / manually"],
						["30", "Every 30 seconds"],
						["60", "Every minute"],
						["300", "Every 5 minutes"],
						["900", "Every 15 minutes"],
						["3600", "Every hour"],
					]}
					onChange={(value) => patch({ refreshSeconds: Number(value) })}
				/>
				<Choice
					label="Number format"
					value={config.format}
					options={[
						["number", "Number"],
						["currency", "Currency"],
						["percent", "Percentage (0–1)"],
					]}
					onChange={(format) =>
						patch({ format: format as HomeDataConfig["format"] })
					}
				/>
				{config.format === "currency" && (
					<Field label="Currency code">
						<Input
							maxLength={3}
							value={config.currency}
							onChange={(event) =>
								patch({ currency: event.target.value.toUpperCase() })
							}
						/>
					</Field>
				)}
				<Field label="Decimal places">
					<Input
						type="number"
						min={0}
						max={6}
						value={config.decimals}
						onChange={(event) =>
							patch({ decimals: Number(event.target.value) })
						}
					/>
				</Field>
				{["stat", "metricstrip", "progress", "gauge", "bullet"].includes(
					config.visualization,
				) && (
					<Field label="Optional target">
						<Input
							type="number"
							value={config.target ?? ""}
							placeholder="No target"
							onChange={(event) =>
								patch({
									target: event.target.value
										? Number(event.target.value)
										: null,
								})
							}
						/>
					</Field>
				)}
			</div>
			<Button
				type="button"
				variant="outline"
				className="w-full"
				onClick={() => setPreview((value) => !value)}
			>
				{preview ? "Hide preview" : "Preview with my data"}
			</Button>
			{preview && (
				<div
					className="rounded-xl border border-border/60 bg-card p-4"
					style={{
						height:
							config.appId &&
							(config.table || config.objectType || config.queryId)
								? ["stat", "metricstrip", "progress", "bullet"].includes(
										config.visualization,
									)
									? 180
									: 280
								: undefined,
					}}
				>
					<HomeDataWidget widget={widget} />
				</div>
			)}
		</div>
	);
}
