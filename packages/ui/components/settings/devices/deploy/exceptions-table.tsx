"use client";

import { useTranslation } from "@flow-like/locales";
import { Check } from "lucide-react";
import type { ReactNode } from "react";
import type {
	DeployPlan,
	PlanException,
	PlanFacts,
} from "../../../../lib/device-management/model/deploy-plan";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { cx } from "../primitives/tone";
import { exceptionText, planNames } from "./deploy-copy";
import { TABLE_RESET } from "./deploy-parts";

/* APP §3.7 / §3.9 `exceptions`: what is the same on every device first, then one row per device that differs. */

export interface ExceptionRow {
	id: string;
	device: string;
	/** `warning` needs an acknowledgement, `info` differs by choice, `paused` is left out. */
	tone: PlanException["tone"];
	differs: ReactNode;
	why: ReactNode;
	/** The required acknowledgement, under the reason. */
	check?: ReactNode;
	/** Replaces "Change for this device". */
	action?: ReactNode;
	/** Opens the per-device sheet. */
	onChange?(): void;
}

const TONE_EDGE: Record<ExceptionRow["tone"], string> = {
	warning:
		"[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-warning-solid",
	info: "[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-info-solid",
	paused:
		"[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-paused-solid",
};

/** Plan exceptions as rows, with the sentences of `deploy-copy`. */
export function exceptionRows(
	t: DevicesT,
	plan: DeployPlan,
	exceptions: readonly PlanException[],
	extras: (
		exception: PlanException,
	) => Partial<
		Pick<ExceptionRow, "check" | "action" | "onChange">
	> = () => ({}),
	facts?: PlanFacts,
): ExceptionRow[] {
	const names = planNames(t, plan);
	return exceptions.map((exception, index) => {
		const text = exceptionText(t, exception, plan, facts);
		return {
			id: `${exception.deviceId}:${exception.code}:${index}`,
			device: names.device(exception.deviceId),
			tone: exception.tone,
			differs: text.differs,
			why: text.why,
			...extras(exception),
		};
	});
}

export function ExceptionsTable({
	label,
	shared,
	rows,
}: Readonly<{
	/** Accessible name of the table. */
	label: string;
	/** "Same on every device: …". */
	shared?: ReactNode;
	rows: readonly ExceptionRow[];
}>) {
	const { t } = useTranslation("devices");
	return (
		<div data-exceptions="" className="flex min-w-0 flex-col gap-2">
			<p className="flex items-start gap-1.5 text-ui">
				<Check aria-hidden className="mt-0.5 size-3.5 shrink-0 text-good" />
				<span>
					{shared ?? t("deploy.exceptions.same", "Same on every device.")}
				</span>
			</p>
			{rows.length ? (
				<DvTable
					label={label}
					cols={["24%", "28%", "34%", "14%"]}
					stackAt={560}
					className={TABLE_RESET}
					wrapperClassName="rounded-lg border border-border"
					head={
						<tr>
							<Th>{t("deploy.exceptions.device", "Device")}</Th>
							<Th>{t("deploy.exceptions.differs", "Differs")}</Th>
							<Th>{t("deploy.exceptions.why", "Why")}</Th>
							<Th>
								<span className="sr-only">
									{t("deploy.exceptions.action", "Action")}
								</span>
							</Th>
						</tr>
					}
				>
					{rows.map((row) => (
						<Tr
							key={row.id}
							data-tone={row.tone}
							className={cx("hover:bg-transparent", TONE_EDGE[row.tone])}
						>
							<Td label={t("deploy.exceptions.device", "Device")} kind="name">
								<span className="font-mono">{row.device}</span>
							</Td>
							<Td label={t("deploy.exceptions.differs", "Differs")}>
								{row.differs}
							</Td>
							<Td label={t("deploy.exceptions.why", "Why")}>
								{row.why}
								{row.check ? <div className="mt-1.5">{row.check}</div> : null}
							</Td>
							<Td label={t("deploy.exceptions.action", "Action")} kind="act">
								{row.action ??
									(row.onChange ? (
										<DvButton size="xs" onClick={row.onChange}>
											{t("deploy.exceptions.change", "Change for this device")}
										</DvButton>
									) : null)}
							</Td>
						</Tr>
					))}
				</DvTable>
			) : null}
		</div>
	);
}
