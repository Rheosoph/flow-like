"use client";

import { useTranslation } from "@flow-like/locales";
import { Activity, History, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import {
	FreshnessStamp,
	type FreshnessStampProps,
} from "../primitives/freshness-stamp";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import {
	type Json,
	OBSERVE_COLS,
	amount,
	asRecord,
	bytesText,
	dayLabel,
	whole,
} from "./observe-data";

export interface UsageBlocksProps {
	/** The latest metrics sample of the device, the service or the app. */
	sample: Json;
	/** Where the sample came from and how old it is. */
	stamp: FreshnessStampProps;
	/** "support-bot" or "All services on edge-berlin-01": whose runs these are. */
	subject: string;
}

/* Half-width blocks keep label and value side by side (the list stacks below 520 px by default). */
const FACTS =
	"@max-[520px]/kv:grid-cols-[minmax(150px,max-content)_minmax(0,1fr)] @max-[520px]/kv:gap-y-0";
const FACT = "@max-[520px]/kv:mt-1.5";

interface Fact {
	label: string;
	value: ReactNode;
	tone?: "warning";
}

function Facts({ rows }: Readonly<{ rows: readonly Fact[] }>) {
	return (
		<KeyValueList className={FACTS}>
			{rows.map((row) => (
				<KvRow
					key={row.label}
					label={row.label}
					className={cx(FACT, row.tone === "warning" && "text-warning")}
				>
					{row.value}
				</KvRow>
			))}
		</KeyValueList>
	);
}

function Counter({ value }: Readonly<{ value: unknown }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const number = whole(value);
	return number === undefined ? (
		<span className="text-muted-foreground">
			{t("observe.usage.unavailable", "Unavailable")}
		</span>
	) : (
		<span className="font-mono tabular-nums">
			{new Intl.NumberFormat(time.locale).format(number)}
		</span>
	);
}

function Size({ value }: Readonly<{ value: unknown }>) {
	const { t } = useTranslation("devices");
	const number = whole(value);
	return number === undefined ? (
		<span className="text-muted-foreground">
			{t("observe.usage.unavailable", "Unavailable")}
		</span>
	) : (
		<span className="font-mono tabular-nums">{bytesText(number)}</span>
	);
}

function Hint({ children }: Readonly<{ children: ReactNode }>) {
	return <p className="text-xs text-muted-foreground">{children}</p>;
}

function counterFacts(t: DevicesT, counters: Json, current: boolean): Fact[] {
	const failed = whole(counters.invocations_failed);
	return [
		{
			label: t("observe.usage.started", "Runs started"),
			value: <Counter value={counters.invocations_started} />,
		},
		{
			label: t("observe.usage.succeeded", "Succeeded"),
			value: <Counter value={counters.invocations_succeeded} />,
		},
		{
			label: t("observe.usage.failed", "Failed"),
			value: <Counter value={counters.invocations_failed} />,
			...(current && failed ? { tone: "warning" as const } : {}),
		},
		{
			label: t("observe.usage.cancelled", "Cancelled"),
			value: <Counter value={counters.invocations_cancelled} />,
		},
		...(current
			? [
					{
						label: t("observe.usage.inFlight", "In flight"),
						value: <Counter value={counters.in_flight} />,
					},
				]
			: []),
		{
			label: t("observe.usage.messages", "Runtime messages"),
			value: <Counter value={counters.runtime_messages} />,
		},
		{
			label: t("observe.usage.requests", "Requests received"),
			value: <Size value={counters.request_payload_bytes} />,
		},
		{
			label: t("observe.usage.responses", "Responses sent"),
			value: <Size value={counters.response_payload_bytes} />,
		},
		{
			label: t("observe.usage.turnedAway", "Turned away (too busy)"),
			value: <Counter value={counters.concurrency_rejections} />,
		},
	];
}

function CurrentUsage({
	usage,
	stamp,
	subject,
}: Readonly<{ usage: Json; stamp: FreshnessStampProps; subject: string }>) {
	const { t } = useTranslation("devices");
	const reported = whole(usage.reported_replicas);
	const expected = whole(usage.expected_replicas);
	return (
		<Block
			id="observe-usage-current"
			icon={Activity}
			title={t("observe.usage.currentTitle", "Usage since instances started")}
			stamp={
				<FreshnessStamp
					{...stamp}
					{...(stamp.age === "live" &&
					reported !== undefined &&
					expected !== undefined
						? {
								text: t(
									"observe.usage.coverage",
									"{{reported, number}} of {{expected, number}} running instances",
									{ reported, expected },
								),
							}
						: {})}
				/>
			}
		>
			<Hint>
				{t("observe.usage.currentHint", "{{subject}} · Not a billing record.", {
					subject,
				})}
			</Hint>
			<Facts rows={counterFacts(t, usage, true)} />
		</Block>
	);
}

function coverageLine(t: DevicesT, coverage: Json): string | undefined {
	const reported = whole(coverage.reported_runs);
	const registered = whole(coverage.registered_runs);
	if (reported === undefined || registered === undefined) return undefined;
	const parts = [
		t(
			"observe.usage.runsReported",
			"{{reported, number}} of {{registered, number}} runs reported",
			{ reported, registered },
		),
	];
	const add = (value: unknown, text: (count: number) => string) => {
		const count = whole(value);
		if (count) parts.push(text(count));
	};
	add(coverage.finalized_runs, (count) =>
		t("observe.usage.runsFinished", "{{count, number}} finished", { count }),
	);
	add(coverage.fresh_active_runs, (count) =>
		t("observe.usage.runsRunning", "{{count, number}} running", { count }),
	);
	add(coverage.incomplete_runs, (count) =>
		t(
			"observe.usage.runsIncomplete",
			"{{count, number}} ended without a final report",
			{ count },
		),
	);
	add(coverage.unreported_runs, (count) =>
		t(
			"observe.usage.runsUnreported",
			"{{count, number}} ended before their first report",
			{ count },
		),
	);
	add(coverage.stale_active_runs, (count) =>
		t("observe.usage.runsStale", "{{count, number}} stopped reporting", {
			count,
		}),
	);
	add(coverage.awaiting_first_report_runs, (count) =>
		t(
			"observe.usage.runsAwaiting",
			"{{count, number}} waiting for their first report",
			{ count },
		),
	);
	return parts.join(" · ");
}

function RetainedUsage({
	retained,
	stamp,
	subject,
}: Readonly<{ retained: Json; stamp: FreshnessStampProps; subject: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const counters = asRecord(retained.counters) ?? {};
	const coverage = asRecord(retained.coverage);
	const since = amount(retained.since);
	const through = amount(retained.through);
	const coverageText = coverage ? coverageLine(t, coverage) : undefined;
	return (
		<Block
			id="observe-usage-retained"
			icon={History}
			title={
				since === undefined
					? t("observe.usage.retainedTitle", "Retained usage")
					: t("observe.usage.retainedSince", "Usage since {{date}}", {
							date: dayLabel(time, since),
						})
			}
			stamp={
				<FreshnessStamp
					{...stamp}
					{...(stamp.age === "live" && through !== undefined
						? {
								text: t("observe.usage.through", "through {{time}}", {
									time: time.clock(through),
								}),
							}
						: {})}
				/>
			}
		>
			<Hint>
				{t(
					"observe.usage.retainedHint",
					"{{subject}} · retained across restarts · Not a billing record.",
					{ subject },
				)}
			</Hint>
			<Facts
				rows={[
					...counterFacts(t, counters, false),
					...(coverageText
						? [
								{
									label: t("observe.usage.coverageLabel", "Coverage"),
									value: coverageText,
								},
							]
						: []),
				]}
			/>
			{retained.tail_loss_possible === true ? (
				<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
					<TriangleAlert
						aria-hidden
						className="mt-0.5 size-3 shrink-0 text-warning"
					/>
					{t(
						"observe.usage.tailLoss",
						"Some activity may be missing: not every run reported before it ended.",
					)}
				</p>
			) : (
				<Hint>
					{t(
						"observe.usage.noTailLoss",
						"No activity is missing from this window.",
					)}
				</Hint>
			)}
		</Block>
	);
}

/** Usage since the instances started and usage kept across restarts, side by side (SPEC §5.2, §5.3). */
export function UsageBlocks({
	sample,
	stamp,
	subject,
}: Readonly<UsageBlocksProps>) {
	const { t } = useTranslation("devices");
	const usage = asRecord(sample.usage);
	const retained = asRecord(sample.usage_retained);
	if (!usage && !retained) return null;
	return (
		<div className={OBSERVE_COLS}>
			{usage ? (
				<CurrentUsage usage={usage} stamp={stamp} subject={subject} />
			) : null}
			{retained ? (
				<RetainedUsage retained={retained} stamp={stamp} subject={subject} />
			) : (
				<Block
					id="observe-usage-retained"
					icon={History}
					title={t("observe.usage.retainedTitle", "Retained usage")}
				>
					<StateView
						kind="notloaded"
						title={t("observe.usage.notRetained", "Not retained")}
						text={t(
							"observe.usage.notRetainedText",
							"No usage was reported across restarts yet.",
						)}
					/>
				</Block>
			)}
		</div>
	);
}
