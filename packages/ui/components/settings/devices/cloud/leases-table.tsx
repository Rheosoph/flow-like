"use client";

import { useTranslation } from "@flow-like/locales";
import type { InstanceLease } from "../../../../lib/device-resources";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { IdRef } from "../primitives/id-ref";
import { TABLE_RESET } from "./cloud-parts";

const HALF_DAY_S = 12 * 3600;

/** The fixed explanation of a lease (SPEC §6.1). */
export const leaseNote = (t: DevicesT) =>
	t(
		"devices:cloud.leases.note",
		"A lease lets an instance get cloud credentials for 10 minutes at a time. It isn't proof the instance is healthy.",
	);

/** A lease lasts minutes, so its times are shown to the second; a stale one keeps its date. */
function useLeaseTime(): (atS: number) => string {
	const time = useAreaTime();
	return (atS) =>
		Math.abs(atS - time.nowS) < HALF_DAY_S ? time.clock(atS) : time.at(atS);
}

const COMPACT_CELL =
	"border-t border-hairline px-1.5 py-[5px] text-left align-top";
const COMPACT_HEAD =
	"px-1.5 py-[5px] text-left text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground";

/** The device block's short list: instance, purpose and when the lease ends. */
function CompactLeases({
	leases,
}: Readonly<{ leases: readonly InstanceLease[] }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const at = useLeaseTime();
	return (
		<table
			aria-label={t("cloud.leases.label", "Cloud leases")}
			className={`w-full border-collapse text-xs ${TABLE_RESET}`}
		>
			<thead>
				<tr>
					<th scope="col" className={COMPACT_HEAD}>
						{t("cloud.leases.instance", "Instance")}
					</th>
					<th scope="col" className={COMPACT_HEAD}>
						{t("cloud.leases.purpose", "Purpose")}
					</th>
					<th scope="col" className={COMPACT_HEAD}>
						{t("cloud.leases.ends", "Lease ends")}
					</th>
				</tr>
			</thead>
			<tbody>
				{leases.map((lease) => (
					<tr key={lease.instance_id} data-lease={lease.instance_id}>
						<td className={COMPACT_CELL}>
							<IdRef
								id={lease.instance_id}
								copyLabel={t("cloud.leases.copyInstance", "Copy instance ID")}
							/>
						</td>
						<td className={COMPACT_CELL}>
							{enumLabel(t, "instancePurpose", lease.purpose)}
						</td>
						<td className={`${COMPACT_CELL} tabular-nums`}>
							{at(lease.lease_expires_at)}
							{lease.lease_expires_at <= time.nowS ? (
								<CellSub className="text-warning">
									{t("cloud.leases.expired", "may have expired")}
								</CellSub>
							) : null}
						</td>
					</tr>
				))}
			</tbody>
		</table>
	);
}

/** Which instances hold a cloud lease right now; `compact` is the device block's short list. */
export function LeasesTable({
	leases,
	compact = false,
}: Readonly<{ leases: readonly InstanceLease[]; compact?: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const at = useLeaseTime();
	if (compact) return <CompactLeases leases={leases} />;
	const labels = {
		instance: t("cloud.leases.instance", "Instance"),
		purpose: t("cloud.leases.purpose", "Purpose"),
		registered: t("cloud.leases.registered", "Registered"),
		ends: t("cloud.leases.ends", "Lease ends"),
	};
	return (
		<DvTable
			label={t("cloud.leases.label", "Cloud leases")}
			cols={["26%", "22%", "26%", "26%"]}
			stackAt={560}
			className={TABLE_RESET}
			head={
				<tr>
					<Th>{labels.instance}</Th>
					<Th>{labels.purpose}</Th>
					<Th>{labels.registered}</Th>
					<Th>{labels.ends}</Th>
				</tr>
			}
		>
			{leases.map((lease) => (
				<Tr key={lease.instance_id} data-lease={lease.instance_id}>
					<Td label={labels.instance}>
						<IdRef
							id={lease.instance_id}
							copyLabel={t("cloud.leases.copyInstance", "Copy instance ID")}
						/>
					</Td>
					<Td label={labels.purpose}>
						{enumLabel(t, "instancePurpose", lease.purpose)}
					</Td>
					<Td label={labels.registered} className="tabular-nums">
						{at(lease.registered_at)}
					</Td>
					<Td label={labels.ends} className="tabular-nums">
						{at(lease.lease_expires_at)}
						{lease.lease_expires_at <= time.nowS ? (
							<CellSub className="text-warning">
								{t("cloud.leases.expired", "may have expired")}
							</CellSub>
						) : (
							<CellSub>{time.ago(lease.lease_expires_at)}</CellSub>
						)}
					</Td>
				</Tr>
			))}
		</DvTable>
	);
}
