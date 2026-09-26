"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { userLookupQueryOptions } from "../../hooks/use-user-lookup";
import {
	formatAbsoluteDateTime,
	formatCalendarDate,
	formatRelativeTime,
} from "../../lib/date";
import { userDisplayName } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import { accountIdFromValue } from "../../state/backend-state/user-state";
import {
	type ColumnKind,
	isNullish,
} from "../settings/data-studio/query-workbench/column-types";
import {
	ResultCellValue,
	ResultDetailValue,
} from "../settings/data-studio/query-workbench/result-value";
import { type HomeDataConfig, formatHomeDataValue } from "./home-data-query";
import { HOME_DATA_EMPTY_LABEL, homeDataText } from "./home-data-text";
import {
	type HomeDataLabelFormat,
	homeDataDateLabel,
	homeDataLabelFormat,
	homeDataTemporalValue,
	homeDataValueLabel,
} from "./home-data-values";

/** A result holds at most 500 rows; no widget needs more names than this. */
const MAX_RESOLVED_USERS = 200;
const EMPTY_RESULT: Pick<ExecuteSqlResult, "columns" | "rows"> = {
	columns: [],
	rows: [],
};

/** Display names for account ids, resolved through the shared batched lookup. */
export function useHomeDataUserNames(
	ids: readonly string[],
): ReadonlyMap<string, string> {
	const backend = useBackend();
	const results = useQueries({
		queries: ids.map((id) => userLookupQueryOptions(backend.userState, id)),
	});
	const signature = JSON.stringify(
		results.map((result) =>
			result.data ? userDisplayName(result.data, "") : "",
		),
	);
	return useMemo(() => {
		const names = JSON.parse(signature) as string[];
		return new Map(
			ids.flatMap((id, index): [string, string][] =>
				names[index] ? [[id, names[index]]] : [],
			),
		);
	}, [ids, signature]);
}

export interface HomeDataLabels {
	/** How a result column reads; aggregate aliases resolve to their source column. */
	format: (name: string) => HomeDataLabelFormat;
	kind: (name: string) => ColumnKind;
	/** Plain-text reading of a value of a result column, for charts and headers. */
	label: (name: string, value: unknown) => string;
	group: (value: unknown) => string;
	series: (value: unknown) => string;
}

/**
 * Everything a widget needs to show result values the way the Data Studio table
 * does: dates as dates, account ids as people, stored paths as file names.
 */
export function useHomeDataLabels(
	result: Pick<ExecuteSqlResult, "columns" | "rows"> | null,
	config: HomeDataConfig,
): HomeDataLabels {
	const source = result ?? EMPTY_RESULT;
	const formats = useMemo(
		() =>
			new Map(
				source.columns.map((column): [string, HomeDataLabelFormat] => [
					column.name,
					homeDataLabelFormat(source, column.name, config),
				]),
			),
		[source, config],
	);
	const userIds = useMemo(() => {
		const ids = new Set<string>();
		for (const [name, format] of formats) {
			if (format.kind !== "user") continue;
			for (const row of source.rows) {
				if (ids.size >= MAX_RESOLVED_USERS) break;
				const id = accountIdFromValue(row[name]);
				if (id) ids.add(id);
			}
		}
		return [...ids];
	}, [source.rows, formats]);
	const userNames = useHomeDataUserNames(userIds);
	return useMemo(() => {
		const named = new Map<string, HomeDataLabelFormat>();
		const cache = new Map<string, Map<unknown, string>>();
		const format = (name: string) => {
			let known = named.get(name);
			if (!known) {
				known = {
					...(formats.get(name) ??
						homeDataLabelFormat(EMPTY_RESULT, name, config)),
					userNames,
				};
				named.set(name, known);
			}
			return known;
		};
		const label = (name: string, value: unknown) => {
			let values = cache.get(name);
			if (!values) {
				values = new Map();
				cache.set(name, values);
			}
			const key =
				value !== null && typeof value === "object"
					? JSON.stringify(value)
					: value;
			let text = values.get(key);
			if (text === undefined) {
				text = homeDataValueLabel(value, format(name));
				values.set(key, text);
			}
			return text;
		};
		return {
			format,
			kind: (name) => format(name).kind,
			label,
			group: (value) => label("__group", value),
			series: (value) => label("__series", value),
		};
	}, [formats, userNames, config]);
}

/** An instant in a row: bucket label, calendar day, or relative time with the exact one on hover. */
function HomeDataInstant({
	date,
	format,
	detail,
	className,
}: Readonly<{
	date: Date;
	format: HomeDataLabelFormat;
	detail: boolean;
	className?: string;
}>) {
	if (format.bucket !== "none")
		return (
			<time className={className} dateTime={date.toISOString()}>
				{homeDataDateLabel(date, format)}
			</time>
		);
	// Whole UTC days carry no time of day; reading them locally would move the
	// calendar day for every viewer west of Greenwich.
	const exact = format.dateOnly
		? formatCalendarDate(date, "full")
		: formatAbsoluteDateTime(date);
	if (detail)
		return (
			<time className={className} dateTime={date.toISOString()}>
				{exact}
				<span className="ml-1.5 text-xs text-muted-foreground">
					{formatRelativeTime(date)}
				</span>
			</time>
		);
	return (
		<time className={className} dateTime={date.toISOString()} title={exact}>
			{formatRelativeTime(date)}
		</time>
	);
}

/** Titles keep raw text for these kinds: an id or a year is not a quantity. */
const PLAIN_TITLE_KINDS = new Set<ColumnKind>(["text", "json", "number"]);

/** A record's headline value: raw text for plain kinds, a person, date or file otherwise. */
export function HomeDataTitle({
	value,
	format,
	config,
}: Readonly<{
	value: unknown;
	format: HomeDataLabelFormat;
	config: Pick<HomeDataConfig, "appId" | "format" | "currency" | "decimals">;
}>) {
	return PLAIN_TITLE_KINDS.has(format.kind) ? (
		<>{homeDataText(value)}</>
	) : (
		<HomeDataValue value={value} format={format} config={config} />
	);
}

/** Raw text only helps as a tooltip where the value is shown as that text. */
export function homeDataValueTitle(value: unknown, kind: ColumnKind) {
	return PLAIN_TITLE_KINDS.has(kind) ? homeDataText(value) : undefined;
}

/**
 * One record value in a widget. Numbers keep the widget's own number format;
 * people, files, booleans and shapes read as the Data Studio table reads them,
 * and dates honour their bucket and day precision. `detail` gives the roomier
 * reading a single record has space for.
 */
export function HomeDataValue({
	value,
	format,
	config,
	detail = false,
	className,
}: Readonly<{
	value: unknown;
	format: HomeDataLabelFormat;
	config: Pick<HomeDataConfig, "appId" | "format" | "currency" | "decimals">;
	detail?: boolean;
	className?: string;
}>) {
	if (isNullish(value))
		return (
			<span className={cn("text-muted-foreground", className)}>
				{HOME_DATA_EMPTY_LABEL}
			</span>
		);
	if (format.kind === "temporal") {
		const date = homeDataTemporalValue(value, format.typeName);
		if (date)
			return (
				<HomeDataInstant
					date={date}
					format={format}
					detail={detail}
					className={className}
				/>
			);
	}
	// A stored whole number is a year, an id or a code as often as an amount, so the
	// default number format only reaches measures, fractions, or a format the
	// widget explicitly chose.
	if (format.kind === "number")
		return (
			<span className={cn("tabular-nums", className)}>
				{format.measure ||
				config.format !== "number" ||
				!Number.isInteger(Number(value))
					? formatHomeDataValue(value, config)
					: homeDataText(value)}
			</span>
		);
	if (["text", "json", "temporal"].includes(format.kind))
		return <span className={className}>{homeDataText(value)}</span>;
	// A widget row has no room for an identity card; only shapes gain from detail.
	if (detail && format.kind === "geometry")
		return (
			<div className={cn("min-w-0", className)}>
				<ResultDetailValue
					value={value}
					kind={format.kind}
					name={format.sourceName}
					appId={config.appId}
					metadata={format.metadata}
				/>
			</div>
		);
	return (
		<span className={cn("inline-flex min-w-0 max-w-full", className)}>
			<ResultCellValue
				value={value}
				kind={format.kind}
				name={format.sourceName}
				appId={config.appId}
				metadata={format.metadata}
			/>
		</span>
	);
}
