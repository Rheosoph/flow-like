"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type { DevicesT } from "../../primitives/area-context";
import { DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import { CHART_FILL } from "../../primitives/time-series-chart";
import { cx } from "../../primitives/tone";
import { usePersonNames } from "../../workspace";
import { type StatsFormats, consumerLabel } from "./stats-copy";
import type { ConsumerRow } from "./stats-view";

/*
 * The per-consumer breakdown (plan §3.7 item 4): a table, because callers
 * are many and named; each row carries its tokens as an inline bar against
 * the busiest caller, in and out keyed like the Tokens chart.
 */

const COLS = ["22%", "auto", "12%", "12%", "12%", "10%"] as const;

const tokensOf = (row: ConsumerRow) => row.tokensIn + row.tokensOut;

const share = (value: number, max: number) =>
	`${((value / max) * 100).toFixed(2)}%`;

function TokenBar({ row, max }: Readonly<{ row: ConsumerRow; max: number }>) {
	return (
		<span
			aria-hidden
			className="flex h-2 w-full min-w-16 items-stretch gap-0.5"
		>
			{row.tokensIn > 0 ? (
				<i
					className={cx("block", CHART_FILL.context)}
					style={{ width: share(row.tokensIn, max) }}
				/>
			) : null}
			{row.tokensOut > 0 ? (
				<i
					className={cx("block rounded-r-[3px]", CHART_FILL.accent)}
					style={{ width: share(row.tokensOut, max) }}
				/>
			) : null}
		</span>
	);
}

interface Labels {
	caller: string;
	tokens: string;
	tokensIn: string;
	tokensOut: string;
	requests: string;
	failed: string;
}

function labelsOf(t: DevicesT) {
	return {
		caller: t("devices:models.stats.consumers.caller", "Caller"),
		tokens: t("devices:models.stats.consumers.tokens", "Tokens"),
		tokensIn: t("devices:models.stats.tokens.in", "In"),
		tokensOut: t("devices:models.stats.tokens.out", "Out"),
		requests: t("devices:models.stats.requests.title", "Requests"),
		failed: t("devices:models.stats.requests.failed", "Failed"),
	} satisfies Labels;
}

function ConsumerLine({
	row,
	max,
	label,
	labels,
	formats,
}: Readonly<{
	row: ConsumerRow;
	max: number;
	label: string;
	labels: Labels;
	formats: StatsFormats;
}>) {
	const service = row.consumer.kind === "service";
	return (
		<Tr data-consumer={row.consumer.key}>
			<Td label={labels.caller} kind={service ? "mono" : "name"}>
				{label}
			</Td>
			<Td label={labels.tokens}>
				<TokenBar row={row} max={max} />
			</Td>
			<Td label={labels.tokensIn} kind="num">
				{formats.count(row.tokensIn)}
			</Td>
			<Td label={labels.tokensOut} kind="num">
				{formats.count(row.tokensOut)}
			</Td>
			<Td label={labels.requests} kind="num">
				{formats.count(row.requests)}
			</Td>
			<Td label={labels.failed} kind="num">
				{formats.count(row.errors)}
			</Td>
		</Tr>
	);
}

/** Who called the model in the range, most tokens first. */
export function ConsumerTable({
	rows,
	formats,
}: Readonly<{ rows: readonly ConsumerRow[]; formats: StatsFormats }>) {
	const { t } = useTranslation("devices");
	const people = useMemo(() => {
		const ids: string[] = [];
		for (const row of rows)
			if (row.consumer.kind === "person" && row.consumer.userId)
				ids.push(row.consumer.userId);
		return ids;
	}, [rows]);
	const name = usePersonNames(people);
	const labels = labelsOf(t);
	let max = 1;
	for (const row of rows) max = Math.max(max, tokensOf(row));
	return (
		<DvTable
			label={t("devices:models.stats.consumers.label", "Requests by caller")}
			cols={COLS}
			stackAt={560}
			head={
				<tr>
					<Th>{labels.caller}</Th>
					<Th>{labels.tokens}</Th>
					<Th numeric>{labels.tokensIn}</Th>
					<Th numeric>{labels.tokensOut}</Th>
					<Th numeric>{labels.requests}</Th>
					<Th numeric>{labels.failed}</Th>
				</tr>
			}
		>
			{rows.map((row) => (
				<ConsumerLine
					key={row.consumer.key}
					row={row}
					max={max}
					label={consumerLabel(t, row.consumer, name)}
					labels={labels}
					formats={formats}
				/>
			))}
		</DvTable>
	);
}
