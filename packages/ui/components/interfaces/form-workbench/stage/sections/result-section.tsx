"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, memo, useMemo, useState } from "react";
import { TONE_ICON } from "../../../../settings/devices/primitives/icons";
import {
	TONE_BG,
	TONE_LINE,
	TONE_TEXT,
	cx,
} from "../../../../settings/devices/primitives/tone";
import { TextEditor } from "../../../../ui/text-editor";
import type {
	ResultGroup,
	ResultModel,
	ResultRow,
	ResultTable,
} from "../../contracts";
import { type ResultFormat, toResultModel } from "../../run/result-view";
import { sectionLabel } from "../copy";
import { Section } from "../section-label";

const MONO = "font-mono tabular-nums";

function ToneChip({
	tone,
	children,
}: Readonly<{ tone: NonNullable<ResultRow["tone"]>; children: ReactNode }>) {
	const Icon = TONE_ICON[tone];
	return (
		<span
			className={cx(
				"inline-flex h-5.5 items-center gap-1.25 whitespace-nowrap rounded-lg border px-2 font-medium text-[12.5px]",
				TONE_BG[tone],
				TONE_LINE[tone],
				TONE_TEXT[tone],
			)}
		>
			<Icon aria-hidden className="size-3.25 shrink-0" />
			{children}
		</span>
	);
}

function ResultRowView({
	row,
	first,
}: Readonly<{ row: ResultRow; first: boolean }>) {
	return (
		<div
			className={cx(
				"flex min-h-8.5 items-center justify-between gap-4 px-3 py-1.25",
				!first && "border-hairline border-t",
			)}
		>
			{row.label ? (
				<dt className="shrink-0 text-[13px]/[18px] text-muted-foreground">
					{row.label}
				</dt>
			) : null}
			<dd
				className={cx(
					"m-0 min-w-0 text-right text-[13px]/[18px] wrap-anywhere",
					row.mono && MONO,
					!row.label && "text-left",
				)}
			>
				{row.tone ? <ToneChip tone={row.tone}>{row.text}</ToneChip> : row.text}
			</dd>
		</div>
	);
}

function ResultCard({ group }: Readonly<{ group: ResultGroup }>) {
	return (
		<div className="mb-4 break-inside-avoid overflow-hidden rounded-lg border border-border bg-card">
			{group.title ? (
				<h4 className="m-0 border-hairline border-b bg-surface-sunken px-3 py-1.75 font-semibold text-ink-2 text-xs/4 tracking-normal">
					{group.title}
				</h4>
			) : null}
			<dl className="m-0">
				{group.rows.map((row, index) => (
					<ResultRowView
						key={`${row.label}:${index}`}
						row={row}
						first={index === 0}
					/>
				))}
			</dl>
		</div>
	);
}

function ResultTableView({
	table,
	touch,
}: Readonly<{ table: ResultTable; touch: boolean }>) {
	const side = touch ? "px-2" : "px-3";
	return (
		<div className="mb-2 flex flex-col gap-1.5">
			{table.title ? (
				<h4 className="m-0 font-semibold text-[13px]/[18px] tracking-normal">
					{table.title}
				</h4>
			) : null}
			<div className="overflow-x-auto rounded-lg border border-border bg-card">
				<table className="m-0 w-full border-collapse text-[13px]/[18px]">
					<thead>
						<tr>
							{table.head.map((cell, index) => (
								<th
									// biome-ignore lint/suspicious/noArrayIndexKey: columns are positional, two may share a heading
									key={`${cell}:${index}`}
									scope="col"
									className={cx(
										"whitespace-nowrap border-0 bg-surface-sunken py-2 font-medium text-muted-foreground",
										side,
										table.numeric[index] ? "text-right" : "text-left",
									)}
								>
									{cell}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{table.rows.map((row, rowIndex) => (
							<tr key={`${rowIndex}:${row[0]}`}>
								{row.map((cell, index) => (
									<td
										key={`${index}:${cell}`}
										className={cx(
											"border-0 border-hairline border-t py-2",
											side,
											table.numeric[index]
												? cx("whitespace-nowrap text-right", MONO)
												: cell.length < 18 && "whitespace-nowrap",
										)}
									>
										{cell}
									</td>
								))}
							</tr>
						))}
					</tbody>
				</table>
			</div>
		</div>
	);
}

function StructuredView({
	groups,
	tables,
	touch,
}: Readonly<{
	groups: readonly ResultGroup[];
	tables: readonly ResultTable[];
	touch: boolean;
}>) {
	const single = groups.length === 1 && tables.length === 0;
	return (
		<>
			{groups.length > 0 ? (
				<div className={cx("gap-x-4", !single && "columns-[2_300px]")}>
					{groups.map((group, index) => (
						<ResultCard key={`${group.title}:${index}`} group={group} />
					))}
				</div>
			) : null}
			{tables.map((table, index) => (
				<ResultTableView
					key={`${table.title}:${index}`}
					table={table}
					touch={touch}
				/>
			))}
		</>
	);
}

function ResultBody({
	model,
	touch,
}: Readonly<{ model: ResultModel; touch: boolean }>) {
	const { view } = model;
	if (view.kind === "text")
		return (
			<div data-fl-chat-prose>
				<TextEditor
					initialContent={view.text}
					isMarkdown={view.markdown}
					editable={false}
				/>
			</div>
		);
	if (view.kind === "value")
		return (
			<div className="overflow-hidden rounded-lg border border-border bg-card">
				<ResultRowView
					first
					row={{ label: "", text: view.text, mono: view.mono, tone: view.tone }}
				/>
			</div>
		);
	return (
		<StructuredView groups={view.groups} tables={view.tables} touch={touch} />
	);
}

/** One small card (a lone value or a single group): the section stays narrow, as the canvas draws SmallDone. */
export function isSingleCard(view: ResultModel["view"]) {
	if (view.kind === "value") return true;
	return (
		view.kind === "structured" &&
		view.groups.length === 1 &&
		view.tables.length === 0
	);
}

const HEAD_BUTTON =
	"inline-flex h-6 items-center rounded-lg border px-2 font-medium text-xs outline-ring focus-visible:outline-2 focus-visible:outline-offset-1";

/**
 * What the flow returned, read as labelled rows and tables rather than JSON; the raw text sits behind a
 * toggle ("Raw"). The run bar's "Copy result" copies it.
 */
export const ResultSection = memo(function ResultSection({
	value,
	format,
	touch,
}: Readonly<{
	value: unknown;
	format: ResultFormat;
	touch: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	const model = useMemo(() => toResultModel(value, format), [value, format]);
	const [raw, setRaw] = useState(false);
	return (
		<Section
			id="result"
			label={sectionLabel(t, "result")}
			className={cx(isSingleCard(model.view) && "max-w-104")}
			aside={
				<>
					<span className="flex-1" />
					<button
						type="button"
						aria-pressed={raw}
						onClick={() => setRaw((on) => !on)}
						className={cx(
							HEAD_BUTTON,
							touch && "h-11",
							raw
								? "border-foreground bg-foreground text-background"
								: "border-border bg-card text-ink-2 hover:bg-row-hover",
						)}
					>
						{t("workbench.stage.result.raw", "Raw")}
					</button>
				</>
			}
		>
			{raw ? (
				<pre className="m-0 max-h-105 overflow-auto rounded-lg border border-border bg-surface-sunken px-3.5 py-3 font-mono text-[12.5px]/[19px] [tab-size:2]">
					{model.raw}
				</pre>
			) : (
				<ResultBody model={model} touch={touch} />
			)}
		</Section>
	);
});
