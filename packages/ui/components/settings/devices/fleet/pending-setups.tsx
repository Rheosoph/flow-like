"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleDashed, CircleX, Hourglass } from "lucide-react";
import { useMemo } from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type { PendingSetup } from "../../../../lib/device-management/model/types";
import { gateCopy } from "../copy/gate-copy";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StatusChip } from "../primitives/status-chip";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	useAttentionInput,
	useDeviceAction,
	useDeviceUsage,
	useDeviceWorkspace,
	useEnrollments,
	useGate,
	useHubSupport,
	useInlineResults,
	usePendingSetups,
} from "../workspace";
import { dayLabel } from "./fleet-device-row";
import { FLEET_TABLE_CLASS } from "./fleet-devices-table";

const COLS = ["20%", "16%", "11%", "14%", "16%", "23%"] as const;

/** The hub removes a lapsed setup from its list this long after it expired. */
const PRUNED_AFTER_S = 7 * 86_400;

export const PENDING_SETUPS_ID = "devices-pending-setups";
const RESULT_KEY = "fleet:pending-setups";

interface RowProps {
	setup: PendingSetup;
	onCancel(setup: PendingSetup): void;
	cancelling: boolean;
}

function SetupRow({ setup, onCancel, cancelling }: Readonly<RowProps>) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const gate = useGate("cancel_setup");
	const locale = i18n?.language ?? "en";
	const expired = setup.state === "expired" || setup.expiresAt <= time.nowS;
	const cancelled = setup.state === "cancelled";
	const waiting = !expired && !cancelled;
	const labels = {
		name: t("fleet.pending.column.name", "Device name"),
		state: t("fleet.pending.column.state", "State"),
		created: t("fleet.pending.column.created", "Created"),
		expires: t("fleet.pending.column.expires", "Expires"),
		slot: t("fleet.pending.column.slot", "Device slot"),
		actions: t("fleet.pending.column.actions", "Actions"),
	};
	return (
		<Tr data-setup={setup.enrollmentId} dim={!waiting}>
			<Td label={labels.name} kind="name">
				<span className="flex min-w-0 items-center gap-1.5">
					<PresenceGlyph kind="pending" />
					<span className="truncate font-mono font-semibold" title={setup.name}>
						{setup.name || t("fleet.pending.unnamed", "Unnamed device")}
					</span>
				</span>
				{setup.deviceId ? (
					<CellSub>
						<IdRef
							id={setup.deviceId}
							copyLabel={t("fleet.pending.copyId", "Copy device ID")}
						/>
					</CellSub>
				) : null}
			</Td>
			<Td label={labels.state}>
				{waiting ? (
					<StatusChip tone="info" icon={Hourglass}>
						{t("fleet.pending.waiting", "Waiting for the device")}
					</StatusChip>
				) : cancelled ? (
					<StatusChip tone="outline" icon={CircleX}>
						{t("fleet.pending.cancelled", "Cancelled")}
					</StatusChip>
				) : (
					<StatusChip tone="unknown" icon={CircleDashed}>
						{t("fleet.pending.expired", "Expired")}
					</StatusChip>
				)}
			</Td>
			<Td label={labels.created}>
				{setup.createdAt === undefined ? (
					"–"
				) : (
					<span title={time.abs(setup.createdAt)}>
						{time.at(setup.createdAt)}
					</span>
				)}
			</Td>
			<Td label={labels.expires}>
				{cancelled ? (
					"–"
				) : waiting ? (
					<>
						<span title={time.abs(setup.expiresAt)}>
							{time.at(setup.expiresAt)}
						</span>
						<CellSub className="tabular-nums">
							{t("fleet.pending.left", "{{time}} left", {
								time: time.countdown(setup.expiresAt),
							})}
						</CellSub>
					</>
				) : (
					<span title={time.abs(setup.expiresAt)}>
						{t("fleet.pending.expiredAt", "Expired {{time}}", {
							time: time.at(setup.expiresAt),
						})}
					</span>
				)}
			</Td>
			<Td label={labels.slot}>
				{waiting ? (
					<>
						{t("fleet.pending.counts", "Counts toward your limit")}
						<CellSub>
							{t("fleet.pending.until", "until {{time}}", {
								time: time.at(setup.expiresAt),
							})}
						</CellSub>
					</>
				) : cancelled ? (
					t("fleet.pending.noLongerCounts", "No longer counts")
				) : (
					<>
						{t("fleet.pending.noLongerWorks", "No longer works")}
						{setup.local ? null : (
							<CellSub>
								{t(
									"fleet.pending.removedOn",
									"removed from the list on {{date}}",
									{
										date: dayLabel(
											setup.expiresAt + PRUNED_AFTER_S,
											time.now,
											locale,
										),
									},
								)}
							</CellSub>
						)}
					</>
				)}
			</Td>
			<Td label={labels.actions} kind="act">
				<div className="flex flex-wrap items-start gap-2">
					{waiting ? (
						<>
							<DvButton size="sm" asChild>
								<a
									{...link({
										screen: "setup",
										enrollmentId: setup.enrollmentId,
										step: 6,
									})}
								>
									{t("fleet.pending.instructions", "Start instructions")}
								</a>
							</DvButton>
							<GatedAction
								gate={
									gate.ok
										? null
										: { kind: gate.kind, reason: gateCopy(t, gate).inline }
								}
							>
								<DvButton
									size="sm"
									variant="danger-ghost"
									busy={cancelling}
									onClick={() => onCancel(setup)}
								>
									{t("fleet.pending.cancel", "Cancel setup…")}
								</DvButton>
							</GatedAction>
						</>
					) : cancelled ? null : (
						<DvButton size="sm" asChild>
							<a {...link({ screen: "setup" })}>
								{t("fleet.pending.again", "Set up again")}
							</a>
						</DvButton>
					)}
				</div>
			</Td>
		</Tr>
	);
}

/** SPEC §5.1 Pending setups: setup packages no device has used yet, with Cancel setup… (BG2). */
export function PendingSetups() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const setups = usePendingSetups();
	const input = useAttentionInput();
	const enrollments = useEnrollments();
	const usage = useDeviceUsage();
	const { support } = useHubSupport();
	const actions = useDeviceAction();
	const results = useInlineResults(RESULT_KEY);
	const { scopeKey } = useDeviceWorkspace().hub;

	const open = useMemo(() => {
		const registered = new Set(input.devices.map((row) => row.device_id));
		return setups.filter(
			(setup) => !(setup.deviceId && registered.has(setup.deviceId)),
		);
	}, [setups, input.devices]);
	if (!open.length && !results.length) return null;

	const limits =
		usage.data?.limits ?? support.limits ?? support.configuredLimits;
	const cancel = (setup: PendingSetup) => {
		const name = setup.name || t("fleet.pending.unnamed", "Unnamed device");
		void actions.run({
			action: "cancel_setup",
			label: t("fleet.pending.cancelLabel", "Cancel setup for {{device}}", {
				device: name,
			}),
			strength: "none",
			confirm: {
				icon: CircleX,
				tone: "danger",
				title: t(
					"fleet.pending.cancelTitle",
					"Cancel the setup for {{device}}?",
					{ device: name },
				),
				sub:
					setup.createdAt === undefined
						? t(
								"fleet.pending.cancelSub",
								"Setup package · expires {{expires}}",
								{
									expires: time.at(setup.expiresAt),
								},
							)
						: t(
								"fleet.pending.cancelSubCreated",
								"Setup package created {{created}} · expires {{expires}}",
								{
									created: time.at(setup.createdAt),
									expires: time.at(setup.expiresAt),
								},
							),
			},
			consequence: {
				what: t(
					"fleet.pending.conseq.what",
					"The setup package for {{device}} stops working.",
					{ device: name },
				),
				who:
					limits?.max_pending_enrollments === undefined
						? t(
								"fleet.pending.conseq.who",
								"Frees one of your unused-package slots. It still counts toward today's packages.",
							)
						: t(
								"fleet.pending.conseq.whoLimits",
								"Frees 1 of your {{max, number}} unused-package slots. It still counts toward today's packages.",
								{ max: limits.max_pending_enrollments },
							),
				stays: t(
					"fleet.pending.conseq.stays",
					"Nothing was installed yet. The keys made for it stay on this computer until you delete them.",
				),
				when: t("fleet.pending.conseq.when", "Immediately."),
				undo: {
					reversible: false,
					text: t("fleet.pending.conseq.undo", "Create a new package."),
				},
			},
			resultKey: RESULT_KEY,
			call: async ({ workspace }) => {
				const { api, profile } = workspace.hub;
				await api.del(
					profile,
					`devices/enrollments/${encodeURIComponent(setup.enrollmentId)}`,
				);
				for (const item of workspace.activity.list())
					if (
						item.resume?.type === "setup" &&
						item.resume.enrollmentId === setup.enrollmentId
					)
						workspace.activity.finish(item.id, "failed", { code: "cancelled" });
			},
			invalidate: [
				deviceKeys.enrollments(scopeKey, "open"),
				deviceKeys.usage(scopeKey),
			],
		});
	};

	return (
		<Block
			id={PENDING_SETUPS_ID}
			icon={Hourglass}
			title={t("fleet.pending.title", "Pending setups")}
			count={open.length}
			stamp={
				enrollments.missingOnHub ? (
					<FreshnessStamp
						source="local"
						age="current"
						text={t("fleet.pending.local", "created on this computer")}
					/>
				) : (
					<FreshnessStamp {...stampOf(enrollments.freshness)} />
				)
			}
			flush
			foot={
				enrollments.missingOnHub
					? t(
							"fleet.pending.footLocal",
							"A setup package can't be downloaded again. If you lose one, cancel it here and create a new one. This hub doesn't list setups, so only the ones created on this computer show here.",
						)
					: t(
							"fleet.pending.foot",
							"A setup package can't be downloaded again. If you lose one, cancel it here and create a new one. Unused packages are removed 7 days after they expire.",
						)
			}
		>
			{open.length ? (
				<DvTable
					cols={COLS}
					className={FLEET_TABLE_CLASS}
					label={t(
						"fleet.pending.caption",
						"Setup packages no device has used yet",
					)}
					head={
						<tr>
							<Th>{t("fleet.pending.column.name", "Device name")}</Th>
							<Th>{t("fleet.pending.column.state", "State")}</Th>
							<Th>{t("fleet.pending.column.created", "Created")}</Th>
							<Th>{t("fleet.pending.column.expires", "Expires")}</Th>
							<Th>{t("fleet.pending.column.slot", "Device slot")}</Th>
							<Th>{t("fleet.pending.column.actions", "Actions")}</Th>
						</tr>
					}
				>
					{open.map((setup) => (
						<SetupRow
							key={setup.enrollmentId}
							setup={setup}
							onCancel={cancel}
							cancelling={actions.pending(RESULT_KEY)}
						/>
					))}
				</DvTable>
			) : null}
			{results.length ? (
				<div className="flex flex-col gap-2 px-4 py-3">
					{results.map((result) => (
						<InlineResult
							key={result.id}
							tone={result.tone}
							onDismiss={result.dismiss}
						>
							{result.text}
						</InlineResult>
					))}
				</div>
			) : null}
		</Block>
	);
}
