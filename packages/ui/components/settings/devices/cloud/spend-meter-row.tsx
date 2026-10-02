"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import { useAreaTime } from "../primitives/area-context";
import { CellSub } from "../primitives/dv-table";
import { IdRef } from "../primitives/id-ref";
import { Meter } from "../primitives/meter";
import type { CloudLimit } from "./cloud-model";
import { useMoney } from "./cloud-parts";
import { useBillingUsage } from "./use-cloud";

/** The hub lists at most this many instances per spending limit (E18). */
const LISTED_INSTANCES = 100;

/** When the limit stops (or stopped) accepting charges. */
function useLimitEnd(): (limit: CloudLimit) => string {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (limit) =>
		limit.state === "active"
			? t("cloud.spend.until", "until {{date}}", {
					date: time.at(limit.expiresAt),
				})
			: limit.state === "revoked"
				? t("cloud.spend.revoked", "Revoked. No new charges.")
				: t("cloud.spend.ended", "Ended {{date}}. No new charges.", {
						date: time.at(limit.expiresAt),
					});
}

/** Used + reserved against the limit, with when it ends (SPEC §4.37 spending meter). */
export function SpendMeterRow({
	limit,
	end = true,
	tail,
	meterClassName,
	className,
	children,
}: Readonly<{
	limit: CloudLimit;
	/** The "until …" line; off where the block states the end itself. */
	end?: boolean;
	/** Appended to the amounts ("paid by you · ends Oct 29"). */
	tail?: string;
	meterClassName?: string;
	className?: string;
	/** Further lines under the meter (warnings). */
	children?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const amount = useMoney();
	const endOf = useLimitEnd();
	const share = (value: number) =>
		limit.limit > 0 ? (value / limit.limit) * 100 : 0;
	const values = {
		used: amount(limit.used),
		reserved: amount(limit.reserved),
		limit: amount(limit.limit),
	};
	const amounts = limit.reserved
		? t(
				"cloud.spend.amountsReserved",
				"{{used}} used · {{reserved}} reserved · {{limit}} limit",
				values,
			)
		: t("cloud.spend.amounts", "{{used}} used · {{limit}} limit", values);
	return (
		<div data-spend={limit.id} className={className}>
			<Meter
				label={amounts}
				className={meterClassName}
				segments={[
					{ value: share(limit.used), tone: "neutral" },
					{ value: share(limit.reserved), tone: "reserved" },
				]}
			/>
			<span className="mt-1 block text-xs tabular-nums text-muted-foreground">
				{tail ? `${amounts} · ${tail}` : amounts}
			</span>
			{end ? <CellSub>{endOf(limit)}</CellSub> : null}
			{children}
		</div>
	);
}

/** BG35: what each instance spent on one limit; older hubs only know the totals. */
export function SpendByInstance({
	deviceId,
	limit,
}: Readonly<{ deviceId: string; limit: CloudLimit }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const amount = useMoney();
	const usage = useBillingUsage(deviceId, limit.id);
	if (usage.missing)
		return (
			<span data-spend-instances="interim">
				{t("cloud.spend.totalsOnly", "Totals only.")}
				<CellSub>
					{t(
						"cloud.spend.totalsOnlyWhy",
						"This hub doesn't report spend per instance.",
					)}
				</CellSub>
			</span>
		);
	if (!usage.data)
		return (
			<span className="text-muted-foreground">
				{usage.failed
					? t("cloud.spend.usageFailed", "Couldn't be read. Totals are above.")
					: t("cloud.spend.usageLoading", "Reading…")}
			</span>
		);
	const { instances, totals } = usage.data;
	if (!instances.length)
		return (
			<span className="text-muted-foreground">
				{t(
					"cloud.spend.noInstances",
					"No instance has spent on this limit yet.",
				)}
			</span>
		);
	return (
		<div data-spend-instances="" className="flex min-w-0 flex-col gap-1.5">
			<ul className="m-0 flex min-w-0 list-none flex-col p-0">
				{instances.map((row) => (
					<li
						key={row.instance_id}
						data-instance={row.instance_id}
						className="flex min-w-0 flex-wrap items-baseline gap-x-2 border-t border-hairline py-1.5 first:border-t-0 first:pt-0"
					>
						<IdRef
							id={row.instance_id}
							copyLabel={t("cloud.leases.copyInstance", "Copy instance ID")}
						/>
						<span className="ml-auto font-mono tabular-nums">
							{amount(row.used_micros)}
						</span>
						<span className="basis-full text-xs text-muted-foreground">
							{t("cloud.spend.instanceLine", {
								count: row.operations,
								time: time.at(row.last_at),
								defaultValue_one: "{{count, number}} request · last {{time}}",
								defaultValue_other:
									"{{count, number}} requests · last {{time}}",
							})}
						</span>
					</li>
				))}
			</ul>
			{instances.length > 1 ? (
				<span className="text-xs text-muted-foreground">
					{t("cloud.spend.totalRequests", {
						count: totals.operations,
						defaultValue_one: "{{count, number}} request in all.",
						defaultValue_other: "{{count, number}} requests in all.",
					})}
					{instances.length >= LISTED_INSTANCES
						? ` ${t(
								"cloud.spend.listedCap",
								"The 100 instances that spent most recently are listed.",
							)}`
						: null}
				</span>
			) : null}
		</div>
	);
}
