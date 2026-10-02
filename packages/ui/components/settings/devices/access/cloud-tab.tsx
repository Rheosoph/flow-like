"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Cloud, RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import {
	APPROVAL_COLUMNS,
	type ApprovalColumnLabels,
	ApprovalRow,
} from "../cloud/approval-row";
import { spendTotals } from "../cloud/cloud-model";
import { useMoney } from "../cloud/cloud-parts";
import { leaseNote } from "../cloud/leases-table";
import {
	type FleetApprovals,
	useDeviceRefs,
	useFleetApprovals,
} from "../cloud/use-cloud";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvTable, Th } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { Meter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import type { ScreenProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy } from "../workspace";

const BOLD = {
	1: <b className="font-semibold" />,
	2: <b className="font-semibold tabular-nums" />,
};
const PAGE = 20;
/** Older hubs: devices read per click (three requests each). */
const READ_STEP = 10;

function SumLine({ fleet }: Readonly<{ fleet: FleetApprovals }>) {
	const { t } = useTranslation("devices");
	const amount = useMoney();
	const totals = useMemo(() => spendTotals(fleet.rows), [fleet.rows]);
	const share = (value: number) =>
		totals.limit > 0 ? (value / totals.limit) * 100 : 0;
	const used = amount(totals.used);
	const limit = amount(totals.limit);
	const values = {
		limits: t("cloud.tab.limitCount", {
			count: totals.count,
			defaultValue_one: "{{count, number}} spending limit",
			defaultValue_other: "{{count, number}} spending limits",
		}),
		used,
		limit,
		reserved: amount(totals.reserved),
	};
	return (
		<div
			data-spend-sum=""
			className="flex flex-wrap items-center gap-x-3.5 gap-y-1.5 border-b border-hairline px-4 py-2.5 text-ui"
		>
			{totals.count ? (
				<>
					<span>
						{totals.reserved ? (
							<Trans
								t={t}
								i18nKey="cloud.tab.sumReserved"
								defaults="You pay for <1>{{limits}}</1> · <2>{{used}}</2> used of <2>{{limit}}</2> · {{reserved}} reserved."
								values={values}
								components={BOLD}
							/>
						) : (
							<Trans
								t={t}
								i18nKey="cloud.tab.sum"
								defaults="You pay for <1>{{limits}}</1> · <2>{{used}}</2> used of <2>{{limit}}</2>."
								values={values}
								components={BOLD}
							/>
						)}
					</span>
					<Meter
						className="w-40"
						label={t("cloud.tab.sumMeter", "{{used}} used of {{limit}}", {
							used,
							limit,
						})}
						segments={[
							{ value: share(totals.used), tone: "neutral" },
							{ value: share(totals.reserved), tone: "reserved" },
						]}
					/>
				</>
			) : (
				<span>
					{t(
						"cloud.tab.sumNone",
						"You don't pay for a spending limit right now.",
					)}
				</span>
			)}
			<span className="text-xs text-muted-foreground">
				{fleet.perDevice
					? t(
							"cloud.tab.sumNotePerDevice",
							"For the devices read so far. Not a billing record. Limits don't recur.",
						)
					: t("cloud.tab.sumNote", "Not a billing record. Limits don't recur.")}
			</span>
		</div>
	);
}

/** FG4 interim: an older hub has no fleet-wide list, so devices are read one by one. */
function PerDeviceNote({
	fleet,
	onRead,
}: Readonly<{ fleet: FleetApprovals; onRead(ids: string[]): void }>) {
	const { t } = useTranslation("devices");
	const next = fleet.unread.slice(0, READ_STEP);
	return (
		<div
			data-per-device=""
			className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-hairline bg-surface-sunken px-4 py-2 text-xs text-muted-foreground"
		>
			<span className="min-w-0 flex-[1_1_40ch]">
				{t(
					"cloud.tab.perDevice",
					"This hub lists approvals device by device, so only the devices read so far are shown.",
				)}{" "}
				{fleet.rows.length
					? t(
							"cloud.tab.approvedDates",
							"Ends are the approved dates: an approval ends earlier if its approver's access to the device ends first, which this hub doesn't report.",
						)
					: null}
			</span>
			{next.length ? (
				<DvButton
					size="xs"
					data-act="read-more"
					busy={fleet.loading}
					onClick={() => onRead(next)}
				>
					{t("cloud.tab.readMore", {
						count: next.length,
						defaultValue_one: "Check {{count, number}} more device",
						defaultValue_other: "Check {{count, number}} more devices",
					})}
				</DvButton>
			) : null}
		</div>
	);
}

/** SPEC §5.7: every cloud approval the viewer gave, owns the device of or pays for, with its spending limit. */
export function AccessCloudTab({ scope }: Readonly<ScreenProps>) {
	const { t } = useTranslation("devices");
	const [cap, setCap] = useState(PAGE);
	const [extra, setExtra] = useState<string[]>([]);
	const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
	const fleet = useFleetApprovals({
		cap,
		extra,
		...(scope.kind === "app" ? { appId: scope.appId } : {}),
	});
	const deviceOf = useDeviceRefs();
	const labels: ApprovalColumnLabels = {
		service: t("cloud.tab.colService", "Service"),
		models: t("cloud.tab.colModels", "Models & files"),
		max: t("cloud.tab.colMax", "Max"),
		instances: t("cloud.tab.colMaxTitle", "Instances allowed"),
		expires: t("cloud.tab.colExpires", "Expires"),
		status: t("cloud.tab.colStatus", "Status"),
		spending: t("cloud.tab.colSpending", "Spending"),
		paidBy: t("cloud.tab.colPaidBy", "Paid by"),
		actions: t("cloud.tab.colActions", "Actions"),
	};
	const read = (ids: string[]) =>
		setExtra((current) => [...new Set([...current, ...ids])]);
	const toggle = (grantId: string, deviceId: string) => {
		setOpen((current) => {
			const next = new Set(current);
			if (!next.delete(grantId)) next.add(grantId);
			return next;
		});
		read([deviceId]);
	};
	const stamp = (
		<FreshnessStamp
			{...stampOf(fleet.freshness)}
			{...(fleet.perDevice
				? { text: t("cloud.tab.stampPerDevice", "read per device") }
				: {})}
		/>
	);
	const title = t("cloud.tab.title", "Cloud approvals and spending limits");

	if (!fleet.rows.length) {
		const failed = !!fleet.error;
		return (
			<Block
				id="access-cloud"
				icon={Cloud}
				title={title}
				stamp={stamp}
				flush={fleet.perDevice}
				bodyClassName={fleet.perDevice ? "gap-0" : undefined}
			>
				{fleet.perDevice ? <PerDeviceNote fleet={fleet} onRead={read} /> : null}
				<div className={fleet.perDevice ? "p-4" : undefined}>
					{failed ? (
						<StateView
							kind="error"
							title={t(
								"cloud.tab.failed",
								"Couldn't read cloud approvals and spending limits",
							)}
							text={t(
								"cloud.tab.failedText",
								"{{why}} Nothing is known about what you approved or pay for until this can be read.",
								{ why: fleet.error ? hubErrorCopy(t, fleet.error.code) : "" },
							)}
							actions={
								<DvButton
									size="sm"
									icon={RefreshCw}
									onClick={() => void fleet.refetch()}
								>
									{t("cloud.retry", "Try again")}
								</DvButton>
							}
						/>
					) : fleet.loading ? (
						<StateView
							kind="loading"
							title={t("cloud.tab.loading", "Reading cloud approvals…")}
						/>
					) : fleet.perDevice && fleet.unread.length ? (
						<StateView
							kind="notloaded"
							title={t(
								"cloud.tab.notRead",
								"No approvals on the devices read so far",
							)}
							text={t(
								"cloud.tab.notReadText",
								"Other devices may have cloud approvals or spending limits. Check them to list theirs.",
							)}
						/>
					) : (
						<StateView
							kind="empty"
							icon={Cloud}
							title={t(
								"cloud.tab.empty",
								"No cloud approvals or spending limits",
							)}
							text={t(
								"cloud.tab.emptyText",
								"When you approve cloud access for a service, or pay for someone else's, it appears here.",
							)}
						/>
					)}
				</div>
			</Block>
		);
	}

	const shown = fleet.rows.slice(0, cap);
	const rest = fleet.rows.length - shown.length;
	return (
		<Block
			id="access-cloud"
			icon={Cloud}
			title={title}
			count={fleet.rows.length}
			stamp={stamp}
			flush
			foot={
				<span>
					{leaseNote(t)}{" "}
					{t(
						"cloud.tab.foot",
						"Revoking stops new credentials; ones already issued stay valid for up to 10 minutes.",
					)}
				</span>
			}
		>
			{fleet.perDevice ? <PerDeviceNote fleet={fleet} onRead={read} /> : null}
			<SumLine fleet={fleet} />
			<DvTable
				label={title}
				cols={APPROVAL_COLUMNS}
				stackAt={900}
				head={
					<tr>
						<Th>{labels.service}</Th>
						<Th>{labels.models}</Th>
						<Th title={labels.instances}>{labels.max}</Th>
						<Th>{labels.expires}</Th>
						<Th>{labels.status}</Th>
						<Th>{labels.spending}</Th>
						<Th>{labels.paidBy}</Th>
						<Th>{labels.actions}</Th>
					</tr>
				}
			>
				{shown.map((approval) => (
					<ApprovalRow
						key={approval.grantId}
						approval={approval}
						device={deviceOf(approval.deviceId)}
						detail={fleet.detail(approval.deviceId)}
						labels={labels}
						expanded={open.has(approval.grantId)}
						onToggle={() => toggle(approval.grantId, approval.deviceId)}
						approvedDates={fleet.perDevice}
					/>
				))}
			</DvTable>
			{rest > 0 ? (
				<div className="border-t border-hairline px-4 py-2">
					<DvButton
						size="sm"
						variant="ghost"
						data-act="show-more"
						onClick={() => setCap((current) => current + PAGE)}
					>
						{t("cloud.tab.showMore", {
							count: Math.min(rest, PAGE),
							defaultValue_one: "Show {{count, number}} more",
							defaultValue_other: "Show {{count, number}} more",
						})}
					</DvButton>
				</div>
			) : null}
		</Block>
	);
}
