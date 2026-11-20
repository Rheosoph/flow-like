"use client";

import { Plus, Trash2 } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { useBackend } from "../../state/backend-state";
import type { SqlParameterInfo } from "../../state/backend-state/query-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";
import {
	ParamControl,
	type ParamOptionsLoader,
} from "./data-widget-param-controls";
import { Choice, Field } from "./data-widget-settings-fields";
import {
	HOME_VIEWER_ID,
	describeBinding,
	effectiveParamControl,
	withParamControl,
} from "./home-data-params";
import {
	HOME_DATA_SQL_MAX_CHARS,
	HOME_DATA_SQL_MAX_SOURCES,
	HOME_PARAM_CONTROLS,
	type HomeDataConfig,
	type HomeDataSqlSource,
	type HomeParamControl,
	extractHomeQueryParameters,
	homeDataSqlAlias,
	homeDataSqlConfigError,
} from "./home-data-query";
import {
	useHomeParamOptionsLoader,
	useHomeSqlDescription,
} from "./use-home-sql-description";

const CACHE_CHOICES: readonly (readonly [number, string])[] = [
	[0, "Don’t reuse results"],
	[60, "1 minute"],
	[300, "5 minutes"],
	[900, "15 minutes"],
	[3600, "1 hour"],
	[21_600, "6 hours"],
	[86_400, "1 day"],
];

export function homeDataCacheChoices(
	current: number,
): readonly (readonly [string, string])[] {
	const choices = CACHE_CHOICES.some(([seconds]) => seconds === current)
		? CACHE_CHOICES
		: [...CACHE_CHOICES, [current, `${current} seconds`] as const].sort(
				(a, b) => a[0] - b[0],
			);
	return choices.map(([seconds, label]) => [String(seconds), label]);
}

export function newHomeDataSqlSource(): HomeDataSqlSource {
	return {
		id: `source-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
		alias: "",
		appId: "",
		scope: "project",
	};
}

const SQL_PLACEHOLDER = `SELECT c.region, SUM(o.total) AS revenue
FROM crm.customers c
JOIN shop.orders o ON o.customer_id = c.id
GROUP BY c.region`;

type ParameterKind = "text" | "number" | "boolean" | "viewer";

function parameterKind(value: unknown): ParameterKind {
	if (value === "$viewer.id") return "viewer";
	if (typeof value === "number") return "number";
	if (typeof value === "boolean") return "boolean";
	return "text";
}

const PARAMETER_DEFAULTS: Record<ParameterKind, unknown> = {
	text: "",
	number: 0,
	boolean: false,
	viewer: "$viewer.id",
};

function useSourceTables(sources: readonly HomeDataSqlSource[]) {
	const backend = useBackend();
	const signature = JSON.stringify(
		[...new Set(sources.map((source) => `${source.scope}:${source.appId}`))]
			.filter((key) => !key.endsWith(":"))
			.sort(),
	);
	const [tables, setTables] = useState<Record<string, string[] | null>>({});
	useEffect(() => {
		let active = true;
		const keys = JSON.parse(signature) as string[];
		void Promise.all(
			keys.map(async (key) => {
				const [scope, ...rest] = key.split(":");
				const appId = rest.join(":");
				try {
					const names =
						scope === "personal"
							? await backend.dbState.listTablesUser(appId)
							: await backend.dbState.listTables(appId);
					return [key, names] as const;
				} catch {
					return [key, null] as const;
				}
			}),
		).then((entries) => {
			if (active) setTables(Object.fromEntries(entries));
		});
		return () => {
			active = false;
		};
	}, [signature, backend.dbState]);
	return tables;
}

function SourceTables({
	alias,
	tables,
}: { alias: string; tables: string[] | null | undefined }) {
	if (tables === undefined) return null;
	if (tables === null)
		return (
			<p className="text-xs text-destructive">
				Tables could not be loaded. Check your access to this app.
			</p>
		);
	if (!tables.length)
		return <p className="text-xs text-muted-foreground">No tables yet.</p>;
	const shown = tables.slice(0, 8);
	return (
		<p className="break-all font-mono text-[11px] text-muted-foreground">
			{shown.map((table) => `${alias || "schema"}.${table}`).join(", ")}
			{tables.length > shown.length && ` +${tables.length - shown.length} more`}
		</p>
	);
}

function SqlParameter({
	name,
	value,
	onChange,
}: { name: string; value: unknown; onChange: (value: unknown) => void }) {
	const kind = parameterKind(value);
	return (
		<div className="grid grid-cols-2 gap-2">
			<Choice
				label={`$${name}`}
				value={kind}
				options={[
					["text", "Text"],
					["number", "Number"],
					["boolean", "True or false"],
					["viewer", "Current user’s ID"],
				]}
				onChange={(next) => onChange(PARAMETER_DEFAULTS[next as ParameterKind])}
			/>
			{kind === "boolean" ? (
				<Choice
					label="Value"
					value={String(value)}
					options={[
						["true", "True"],
						["false", "False"],
					]}
					onChange={(next) => onChange(next === "true")}
				/>
			) : kind === "viewer" ? null : (
				<Field label="Value">
					<Input
						type={kind === "number" ? "number" : "text"}
						value={value === undefined ? "" : String(value)}
						placeholder={value === undefined ? "Required" : undefined}
						onChange={(event) =>
							onChange(
								kind === "number"
									? Number(event.target.value || 0)
									: event.target.value,
							)
						}
					/>
				</Field>
			)}
		</div>
	);
}

function SwitchRow({
	className,
	checked,
	onCheckedChange,
	children,
}: {
	className: string;
	checked: boolean;
	onCheckedChange: (checked: boolean) => void;
	children: string;
}) {
	const id = useId();
	return (
		<div className={className}>
			<Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
			<label htmlFor={id}>{children}</label>
		</div>
	);
}

const CONTROL_LABELS: Record<HomeParamControl, string> = {
	text: "Text",
	number: "Number",
	toggle: "True or false",
	date: "Date",
	datetime: "Date and time",
	select: "Choose one",
	combobox: "Search and choose one",
	"multi-select": "Choose several",
};

function DescribedParameter({
	info,
	config,
	patch,
	loadOptions,
}: {
	info: SqlParameterInfo;
	config: HomeDataConfig;
	patch: (updates: Partial<HomeDataConfig>) => void;
	loadOptions: ParamOptionsLoader;
}) {
	const setting = config.paramControls[info.name];
	const control = effectiveParamControl(info, setting);
	const bound = describeBinding(info);
	const value = config.queryParams[info.name];
	const setValue = (next: unknown) => {
		const { [info.name]: _dropped, ...rest } = config.queryParams;
		patch({
			queryParams: next === undefined ? rest : { ...rest, [info.name]: next },
		});
	};
	const setControl = (change: Partial<NonNullable<typeof setting>>) =>
		patch({
			paramControls: withParamControl(config.paramControls, info.name, change),
		});
	return (
		<div className="space-y-2 rounded-lg border border-border/60 bg-background/60 p-2">
			<div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
				<span className="font-mono text-xs font-medium">${info.name}</span>
				<span className="text-[11px] text-muted-foreground">
					{info.data_type ?? "type unknown"}
					{bound && ` · ${bound}`}
				</span>
			</div>
			<div className="grid grid-cols-2 gap-2">
				<Field label="Default value">
					<ParamControl
						info={info}
						control={control}
						value={value}
						onChange={setValue}
						loadOptions={loadOptions}
						label={`Default for ${info.name}`}
						eager
					/>
				</Field>
				<Choice
					label="Control"
					value={setting?.control ?? ""}
					optional="Automatic"
					options={HOME_PARAM_CONTROLS.map((id) => [id, CONTROL_LABELS[id]])}
					onChange={(next) =>
						setControl({ control: (next || undefined) as HomeParamControl })
					}
				/>
			</div>
			{control === "text" && (
				<SwitchRow
					className="flex items-center gap-2 text-xs"
					checked={value === HOME_VIEWER_ID}
					onCheckedChange={(on) => setValue(on ? HOME_VIEWER_ID : undefined)}
				>
					Use the current user’s ID
				</SwitchRow>
			)}
			<div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
				<Field label="Label">
					<Input
						value={setting?.label ?? ""}
						placeholder={info.name}
						maxLength={80}
						onChange={(event) => setControl({ label: event.target.value })}
					/>
				</Field>
				<SwitchRow
					className="flex h-9 items-center gap-2 text-xs"
					checked={setting?.interactive === true}
					onCheckedChange={(interactive) => setControl({ interactive })}
				>
					Viewers can change this
				</SwitchRow>
			</div>
		</div>
	);
}

function SqlParameters({
	config,
	patch,
}: {
	config: HomeDataConfig;
	patch: (updates: Partial<HomeDataConfig>) => void;
}) {
	const { description, error } = useHomeSqlDescription(config, {
		debounceMs: 700,
	});
	const loadOptions = useHomeParamOptionsLoader(config);
	const plain = extractHomeQueryParameters(config.sql);
	if (description)
		return (
			<>
				{description.parameters.map((info) => (
					<DescribedParameter
						key={info.name}
						info={info}
						config={config}
						patch={patch}
						loadOptions={loadOptions}
					/>
				))}
			</>
		);
	return (
		<>
			{error && plain.length > 0 && (
				<p className="line-clamp-2 text-xs text-muted-foreground" title={error}>
					Parameter types are unavailable: {error}
				</p>
			)}
			{plain.map((name) => (
				<SqlParameter
					key={name}
					name={name}
					value={config.queryParams[name]}
					onChange={(value) =>
						patch({ queryParams: { ...config.queryParams, [name]: value } })
					}
				/>
			))}
		</>
	);
}

/** Sources, SQL and parameters of a widget that queries the tables of several apps. */
export function HomeDataSqlSettings({
	config,
	apps,
	patch,
}: {
	config: HomeDataConfig;
	apps: readonly (readonly [string, string])[];
	patch: (updates: Partial<HomeDataConfig>) => void;
}) {
	const tables = useSourceTables(config.sources);
	const appName = (appId: string) =>
		apps.find(([id]) => id === appId)?.[1] ?? appId;
	const updateSource = (index: number, changes: Partial<HomeDataSqlSource>) =>
		patch({
			sources: config.sources.map((source, position) =>
				position === index ? { ...source, ...changes } : source,
			),
		});
	const chooseApp = (index: number, appId: string) => {
		const source = config.sources[index];
		const others = config.sources
			.filter((_, position) => position !== index)
			.map((item) => item.alias);
		const derived =
			!source.alias ||
			(source.appId &&
				source.alias === homeDataSqlAlias(appName(source.appId), others));
		updateSource(index, {
			appId,
			...(derived && appId
				? { alias: homeDataSqlAlias(appName(appId), others) }
				: {}),
		});
	};
	const problem = homeDataSqlConfigError(config);
	return (
		<>
			<div className="space-y-2">
				<p className="text-xs font-medium">Apps</p>
				{config.sources.map((source, index) => (
					<div
						key={source.id ?? index}
						className="space-y-2 rounded-lg border border-border/60 bg-background/60 p-2"
					>
						<div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
							<Choice
								label="App"
								value={source.appId}
								options={apps}
								optional="Choose an app"
								onChange={(appId) => chooseApp(index, appId)}
							/>
							<Button
								type="button"
								size="icon"
								variant="ghost"
								className="size-9"
								aria-label={`Remove ${source.alias || "this app"}`}
								onClick={() =>
									patch({
										sources: config.sources.filter(
											(_, position) => position !== index,
										),
									})
								}
							>
								<Trash2 className="size-4" />
							</Button>
						</div>
						<div className="grid grid-cols-2 gap-2">
							<Field label="Schema name">
								<Input
									value={source.alias}
									spellCheck={false}
									autoCapitalize="off"
									className="font-mono"
									placeholder="crm"
									onChange={(event) =>
										updateSource(index, {
											alias: event.target.value.toLowerCase(),
										})
									}
								/>
							</Field>
							<Choice
								label="Data"
								value={source.scope}
								options={[
									["project", "Project data"],
									["personal", "Viewer’s personal data"],
								]}
								onChange={(scope) =>
									updateSource(index, {
										scope: scope as HomeDataSqlSource["scope"],
									})
								}
							/>
						</div>
						{source.appId && (
							<SourceTables
								alias={source.alias}
								tables={tables[`${source.scope}:${source.appId}`]}
							/>
						)}
					</div>
				))}
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="h-8 text-xs"
					disabled={config.sources.length >= HOME_DATA_SQL_MAX_SOURCES}
					onClick={() =>
						patch({ sources: [...config.sources, newHomeDataSqlSource()] })
					}
				>
					<Plus className="mr-1.5 size-3.5" />
					Add app
				</Button>
			</div>
			<Field
				label="SQL"
				hint="One read-only SELECT or WITH statement. Address tables as schema.table and use $name for parameters."
			>
				<Textarea
					value={config.sql}
					rows={8}
					spellCheck={false}
					maxLength={HOME_DATA_SQL_MAX_CHARS}
					className="font-mono text-xs"
					placeholder={SQL_PLACEHOLDER}
					onChange={(event) => patch({ sql: event.target.value })}
				/>
			</Field>
			<SqlParameters config={config} patch={patch} />
			{problem && (
				<p className="text-xs text-muted-foreground" aria-live="polite">
					{problem}
				</p>
			)}
		</>
	);
}
