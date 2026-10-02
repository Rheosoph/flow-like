"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	CalendarClock,
	CircleCheck,
	Copy,
	FileBadge,
	Funnel,
	Hourglass,
	Laptop,
	OctagonX,
	Radio,
	RefreshCw,
	TriangleAlert,
	Wrench,
	X,
} from "lucide-react";
import { Fragment, type ReactNode, useMemo, useState } from "react";
import type { CertificateErrorCategory } from "../../../../lib/device-management/certificate-acme";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DevicesRoute,
	Freshness,
} from "../../../../lib/device-management/model/types";
import type { KeyState } from "../../../../lib/device-management/workspace/types";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, GroupRow, Td, Th, Tr } from "../primitives/dv-table";
import { ExpiryRail, ExpiryRailScale } from "../primitives/expiry-rail";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { FilterChip } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { TONE_TEXT, cx } from "../primitives/tone";
import { copyText } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf, useAppNames } from "../shell/attention-popover";
import { hubErrorCopy } from "../workspace/area-context";
import { useOverlayStore } from "../workspace/overlay-store";
import { useAttentionInput } from "../workspace/use-attention";
import { useMyAccess } from "../workspace/use-hub";
import { useLiveSession } from "../workspace/use-live";
import {
	type CertificateRow,
	DAY_S,
	type FleetCounts,
	LIST_CAP,
	MONTH_DAYS,
	PAGE_SIZE,
	ROW_FILTERS,
	type RowFilter,
	SOON_DAYS,
	type SilentDevice,
	type WindowFilter,
	filterFleet,
	isExpired,
	isRowFilter,
	needsYou,
	selfRenewing,
	usedCount,
} from "./certificates-model";
import {
	DayOf,
	DeviceCell,
	DeviceSelect,
	LINK,
	LINK_BUTTON,
	OBJECT_LINK,
	TABLE_RESET,
	dayText,
	shortId,
	spanText,
	untilText,
} from "./parts";
import {
	type CertificateFleetRead,
	PER_DEVICE_READ_CAP,
	usePerson,
} from "./use-certificates";

/** SPEC §5.8: Device · Certificate · Expires · Status · How it renews · Used by · Action. */
const COLS = ["15%", "17%", "19%", "12%", "15%", "9%", "13%"] as const;
/** Devices report on change and at least hourly; older than two reports is last known. */
const REPORT_CURRENT_S = 2 * 3600;
const USED_CAP = 3;

interface Labels {
	device: string;
	certificate: string;
	expires: string;
	status: string;
	renews: string;
	used: string;
	action: string;
}

function columnLabels(t: DevicesT): Labels {
	return {
		device: t("devices:certificates.expiry.column.device", "Device"),
		certificate: t(
			"devices:certificates.expiry.column.certificate",
			"Certificate",
		),
		expires: t("devices:certificates.expiry.column.expires", "Expires"),
		status: t("devices:certificates.expiry.column.status", "Status"),
		renews: t("devices:certificates.expiry.column.renews", "How it renews"),
		used: t("devices:certificates.expiry.column.used", "Used by"),
		action: t("devices:certificates.expiry.column.action", "Action"),
	};
}

export function filterLabel(t: DevicesT, filter: WindowFilter): string {
	const labels: Record<WindowFilter, string> = {
		expired: t("devices:certificates.filter.expired", "Expired"),
		week: t("devices:certificates.filter.week", "≤ 7 d"),
		month: t("devices:certificates.filter.month", "≤ 30 d"),
		errors: t("devices:certificates.filter.errors", "Renewal errors"),
		silent: t("devices:certificates.filter.silent", "Not reported"),
		noaccess: t("devices:certificates.filter.noAccess", "No access"),
	};
	return labels[filter];
}

function certificateRoute(row: CertificateRow): DevicesRoute {
	return {
		screen: "device",
		deviceId: row.device.device_id,
		tab: "certificates",
		certificateId: row.certificateId,
	};
}

function ReportStamp({ reportedAt }: Readonly<{ reportedAt: number | null }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (reportedAt === null)
		return (
			<FreshnessStamp
				source="hub"
				age="notloaded"
				text={t("certificates.expiry.neverReported", "never reported")}
				compact
				noFail
			/>
		);
	return (
		<FreshnessStamp
			source="hub"
			age={time.nowS - reportedAt <= REPORT_CURRENT_S ? "current" : "lastknown"}
			observedAt={reportedAt}
			text={t("certificates.expiry.confirmed", "confirmed {{ago}}", {
				ago: time.ago(Math.min(reportedAt, time.nowS)),
			})}
			compact
			noFail
		/>
	);
}

/* Cells. */

function CertificateCell({
	row,
	open,
	onToggle,
}: Readonly<{ row: CertificateRow; open: boolean; onToggle(): void }>) {
	const { t } = useTranslation("devices");
	const { detail } = row;
	const toggle =
		"block max-w-full cursor-pointer truncate text-left font-semibold hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
	if (detail) {
		const names = [...detail.dns_names, ...detail.ip_addresses].join(", ");
		return (
			<>
				<button
					type="button"
					aria-expanded={open}
					aria-label={t(
						"certificates.expiry.showDetails",
						"{{name}}: show details",
						{ name: detail.label },
					)}
					onClick={onToggle}
					className={toggle}
				>
					{detail.label}
				</button>
				<span
					title={names}
					className="mt-0.5 block truncate font-mono text-xs text-muted-foreground"
				>
					{names || t("certificates.expiry.noNames", "no names")}
				</span>
			</>
		);
	}
	return (
		<>
			<button
				type="button"
				aria-expanded={open}
				title={row.certificateId}
				aria-label={t(
					"certificates.expiry.showDetailsId",
					"Certificate {{id}}: show details",
					{ id: shortId(row.certificateId) },
				)}
				onClick={onToggle}
				className={cx(toggle, "font-mono text-[12.5px]/[18px]")}
			>
				{shortId(row.certificateId)}
			</button>
			<CellSub>
				<Trans
					t={t}
					i18nKey="certificates.expiry.nameAfterLive"
					defaults="fingerprint <1>{{fingerprint}}</1> · name shows after a live read"
					values={{
						fingerprint: (
							row.fingerprint
								.slice(0, 8)
								.toUpperCase()
								.match(/.{1,4}/g) ?? []
						).join(" "),
					}}
					components={{ 1: <span className="font-mono" /> }}
				/>
			</CellSub>
		</>
	);
}

function ExpiresCell({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const ago = untilText(time, row.notAfter);
	return (
		<>
			<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
				<ExpiryRail notAfter={row.notAfter} />
				<DayOf
					at={row.notAfter}
					className="font-mono text-[12.5px]/[18px] font-medium whitespace-nowrap tabular-nums"
				/>
			</span>
			<CellSub>
				{isExpired(row)
					? t("certificates.expiry.expiredAgo", "expired {{ago}}", { ago })
					: ago}
			</CellSub>
		</>
	);
}

function soonReason(t: DevicesT, row: CertificateRow): string {
	if (row.failing)
		return t("devices:certificates.status.cantRenew", "can't renew itself");
	if (selfRenewing(row))
		return t(
			"devices:certificates.status.renewsItself",
			"renews automatically",
		);
	return row.mode
		? t("devices:certificates.status.renewBefore", "renew it before then")
		: t(
				"devices:certificates.status.renewalUnknown",
				"renewal unknown until a live read",
			);
}

function StatusCell({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const left = spanText(t, row.days * DAY_S);
	if (isExpired(row)) {
		const used = usedCount(row);
		if (used)
			return (
				<>
					<StatusChip tone="critical" icon={OctagonX}>
						{t("certificates.status.expired", "Expired")}
					</StatusChip>
					<CellSub>
						<Trans
							t={t}
							i18nKey="certificates.status.expiredInUse"
							count={used}
							tOptions={{
								defaultValue_one:
									"<1>Critical</1> · {{count, number}} service uses it",
								defaultValue_other:
									"<1>Critical</1> · {{count, number}} services use it",
							}}
							components={{
								1: <b className={cx("font-semibold", TONE_TEXT.critical)} />,
							}}
						/>
					</CellSub>
				</>
			);
		return (
			<>
				<StatusChip tone="warning" icon={TriangleAlert}>
					{t("certificates.status.expired", "Expired")}
				</StatusChip>
				<CellSub>
					{used === 0
						? t(
								"certificates.status.expiredUnused",
								"Warning · nothing uses it",
							)
						: t(
								"certificates.status.expiredUnknown",
								"Warning · which services use it is unknown",
							)}
				</CellSub>
			</>
		);
	}
	if (row.days <= SOON_DAYS)
		return (
			<>
				<StatusChip tone="warning" icon={Hourglass}>
					{t("certificates.status.expiresIn", "Expires in {{span}}", {
						span: left,
					})}
				</StatusChip>
				<CellSub>{soonReason(t, row)}</CellSub>
			</>
		);
	if (row.days <= MONTH_DAYS)
		return (
			<>
				<StatusChip tone="info" icon={CalendarClock}>
					{t("certificates.status.expiresIn", "Expires in {{span}}", {
						span: left,
					})}
				</StatusChip>
				<CellSub>
					{t("certificates.status.withinMonth", "within 30 days")}
				</CellSub>
			</>
		);
	return (
		<>
			<StatusChip tone="good" icon={CircleCheck}>
				{t("certificates.status.valid", "Valid")}
			</StatusChip>
			<CellSub>
				{t("certificates.status.validFor", "for {{span}}", { span: left })}
			</CellSub>
		</>
	);
}

/** BG27: why the last renewal attempt failed, as the device filed it. */
export function failureReason(
	t: DevicesT,
	category: CertificateErrorCategory | undefined,
): string {
	if (!category)
		return t(
			"devices:certificates.failure.unknown",
			"the device hasn't said why yet",
		);
	const reasons: Record<CertificateErrorCategory, string> = {
		dns: t(
			"devices:certificates.failure.dns",
			"the authority can't resolve the name",
		),
		port_bind: t(
			"devices:certificates.failure.portBind",
			"port 80 isn't free on the device",
		),
		rate_limited: t(
			"devices:certificates.failure.rateLimited",
			"Let's Encrypt asked it to wait",
		),
		ca_rejected: t(
			"devices:certificates.failure.caRejected",
			"the authority refused the request",
		),
		authority_expired: t(
			"devices:certificates.failure.authorityExpired",
			"its renewal authority has expired",
		),
		network: t(
			"devices:certificates.failure.network",
			"the device and the authority can't reach each other",
		),
		internal: t(
			"devices:certificates.failure.internal",
			"the device hit an internal error",
		),
	};
	return reasons[category];
}

export function modeLabel(
	t: DevicesT,
	mode: NonNullable<CertificateRow["mode"]>,
): string {
	const labels = {
		acme: t("devices:certificates.mode.acme", "Automatic (Let's Encrypt)"),
		delegated: t(
			"devices:certificates.mode.delegated",
			"Automatic (your authority)",
		),
		manual: t("devices:certificates.mode.manual", "Manual"),
	} satisfies Record<NonNullable<CertificateRow["mode"]>, string>;
	return labels[mode];
}

function failingText(t: DevicesT, time: AreaTime, row: CertificateRow) {
	if (row.issuer && row.issuer.not_after <= time.nowS)
		return t(
			"devices:certificates.renews.authorityExpired",
			"Failing · its authority expired {{when}}",
			{ when: time.at(row.issuer.not_after) },
		);
	return t("devices:certificates.renews.failing", "Failing · {{reason}}", {
		reason: failureReason(
			t,
			row.acme?.error_category ?? row.issuer?.error_category,
		),
	});
}

function healthyText(t: DevicesT, time: AreaTime, row: CertificateRow) {
	if (row.mode === "delegated" && row.issuer)
		return t("devices:certificates.renews.next", "next {{when}}", {
			when: time.at(row.issuer.next_renewal_at),
		});
	if (row.mode === "acme" && row.acme)
		return row.acme.environment === "lets_encrypt_staging"
			? t(
					"devices:certificates.renews.acmeTest",
					"test certificate, not trusted by browsers · next try {{date}}",
					{ date: dayText(time, row.acme.next_attempt_at) },
				)
			: t(
					"devices:certificates.renews.acmeProduction",
					"production · next try {{date}}",
					{ date: dayText(time, row.acme.next_attempt_at) },
				);
	return t(
		"devices:certificates.renews.manual",
		"replace it before it expires",
	);
}

function RenewsCell({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!row.mode)
		return (
			<>
				<span className="text-muted-foreground">
					{t("certificates.renews.unknown", "Unknown")}
				</span>
				<CellSub>
					{row.detail
						? t(
								"certificates.renews.unknownSettings",
								"shows once its Certificates tab was read",
							)
						: t("certificates.renews.unknownLive", "shows after a live read")}
				</CellSub>
			</>
		);
	return (
		<>
			{modeLabel(t, row.mode)}
			{row.failing ? (
				<CellSub
					className={cx("flex items-start gap-1", TONE_TEXT.critical)}
					data-renewal="failing"
				>
					<OctagonX aria-hidden className="mt-0.5 size-3 shrink-0" />
					<span>{failingText(t, time, row)}</span>
				</CellSub>
			) : (
				<CellSub>{healthyText(t, time, row)}</CellSub>
			)}
		</>
	);
}

function UsedCell({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	if (!row.detail)
		return (
			<>
				<span className="text-muted-foreground">
					{t("certificates.used.unknown", "Unknown")}
				</span>
				<CellSub>
					{t("certificates.used.unknownLive", "until a live read")}
				</CellSub>
			</>
		);
	const { bindings } = row.detail;
	const total = usedCount(row) ?? bindings.length;
	if (!total)
		return (
			<span className="text-muted-foreground">
				{t("certificates.used.none", "Not used")}
			</span>
		);
	return (
		<span className="flex min-w-0 flex-col gap-0.5">
			{bindings.slice(0, USED_CAP).map((binding) => (
				<a
					key={`${binding.placement_id}/${binding.service}`}
					{...link({
						screen: "service",
						deviceId: row.device.device_id,
						serviceId: binding.placement_id,
						tab: "endpoint",
					})}
					title={[binding.placement_id, appName(binding.project_id)]
						.filter(Boolean)
						.join(" · ")}
					className={cx(OBJECT_LINK, "truncate text-[12.5px]/[18px]")}
				>
					{binding.placement_id}
				</a>
			))}
			{total > USED_CAP ? (
				<span className="text-xs text-muted-foreground">
					{t("certificates.used.more", "+{{count, number}} more", {
						count: total - USED_CAP,
					})}
				</span>
			) : null}
		</span>
	);
}

/** Why opening the device can't lead to a renewal right now (the device page handles unlocking). */
function liveGate(
	t: DevicesT,
	row: CertificateRow,
	keys: KeyState | undefined,
	verb: "renew" | "fix",
): Gate | null {
	const device = row.deviceName;
	if (row.presence.kind === "offline")
		return {
			kind: "live",
			reason:
				verb === "fix"
					? t(
							"devices:certificates.gate.offlineFix",
							"{{device}} is offline. Fixing renewal needs a live connection.",
							{ device },
						)
					: t(
							"devices:certificates.gate.offlineRenew",
							"{{device}} is offline. Renewing needs a live connection.",
							{ device },
						),
		};
	if (row.presence.kind === "never")
		return {
			kind: "live",
			reason: t(
				"devices:certificates.gate.neverSeen",
				"{{device}} hasn't checked in yet, so there's no live connection.",
				{ device },
			),
		};
	if (!keys || keys === "none")
		return {
			kind: "nokeys",
			reason: t(
				"devices:certificates.gate.noKeys",
				"This computer has no keys for {{device}}.",
				{ device },
			),
		};
	return null;
}

function ActionCell({
	row,
	keys,
	canSignHere,
	onRestoreAuthority,
}: Readonly<{
	row: CertificateRow;
	keys: KeyState | undefined;
	canSignHere: boolean;
	onRestoreAuthority(): void;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const openDiagnose = useOverlayStore((store) => store.openDiagnose);
	const target = link(certificateRoute(row));
	if (!row.failing && !needsYou(row))
		return (
			<DvButton size="sm" asChild>
				<a {...target}>{t("certificates.action.open", "Open")}</a>
			</DvButton>
		);
	const fix = row.failing;
	const label = fix
		? t("certificates.action.fixRenewal", "Fix renewal")
		: t("certificates.action.renew", "Renew");
	const icon = fix ? Wrench : RefreshCw;
	const needsAuthority = fix && row.mode === "delegated" && !canSignHere;
	const gate: Gate | null = needsAuthority
		? {
				kind: "nokeys",
				reason: t(
					"certificates.gate.noAuthority",
					"Needs an organisation authority that can sign on this computer.",
				),
			}
		: liveGate(t, row, keys, fix ? "fix" : "renew");
	if (!gate)
		return (
			<DvButton size="sm" icon={icon} asChild>
				<a {...target}>{label}</a>
			</DvButton>
		);
	return (
		<span className="flex flex-col items-start gap-1.5">
			<GatedAction gate={gate}>
				<DvButton size="sm" icon={icon}>
					{label}
				</DvButton>
			</GatedAction>
			{needsAuthority ? (
				<button
					type="button"
					onClick={onRestoreAuthority}
					className={LINK_BUTTON}
				>
					{t(
						"certificates.action.restoreAuthority",
						"Restore one from backup…",
					)}
				</button>
			) : gate.kind === "live" ? (
				<button
					type="button"
					onClick={() => openDiagnose(row.device.device_id)}
					className={LINK_BUTTON}
				>
					{t("certificates.action.diagnose", "Diagnose connection…")}
				</button>
			) : null}
		</span>
	);
}

/* The expanded facts of one certificate. */

function RenewalFacts({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { issuer, acme } = row;
	return (
		<>
			{issuer ? (
				<>
					<KvRow
						label={t(
							"certificates.detail.renewalAuthority",
							"Renewal authority",
						)}
					>
						{issuer.not_after <= time.nowS ? (
							<span className={TONE_TEXT.critical}>
								{t("certificates.detail.authorityExpired", "Expired {{when}}", {
									when: time.at(issuer.not_after),
								})}
							</span>
						) : (
							t("certificates.detail.authorityValid", "valid until {{when}}", {
								when: time.at(issuer.not_after),
							})
						)}
					</KvRow>
					<KvRow
						label={t("certificates.detail.eachRenewal", "Each renewal lasts")}
					>
						{t("certificates.detail.lifetimeDays", {
							count: issuer.leaf_lifetime_days,
							defaultValue_one: "{{count, number}} day",
							defaultValue_other: "{{count, number}} days",
						})}
					</KvRow>
					<KvRow label={t("certificates.detail.lastRenewed", "Last renewed")}>
						{issuer.last_renewed_at === null
							? t("certificates.detail.never", "Not yet")
							: time.at(issuer.last_renewed_at)}
					</KvRow>
				</>
			) : null}
			{acme ? (
				<>
					<KvRow label={t("certificates.detail.letsEncrypt", "Let's Encrypt")}>
						{acme.environment === "lets_encrypt_production"
							? t(
									"certificates.detail.acmeProduction",
									"Production (trusted by browsers)",
								)
							: t(
									"certificates.detail.acmeTest",
									"Test (not trusted by browsers)",
								)}
					</KvRow>
					<KvRow label={t("certificates.detail.nextAttempt", "Next attempt")}>
						{time.at(acme.next_attempt_at)}
					</KvRow>
					<KvRow label={t("certificates.detail.lastIssued", "Last issued")}>
						{acme.last_renewed_at === null
							? t("certificates.detail.never", "Not yet")
							: time.at(acme.last_renewed_at)}
					</KvRow>
				</>
			) : null}
			{row.failing ? (
				<KvRow
					label={t("certificates.detail.lastFailure", "Last failure")}
					provenance={t(
						"certificates.detail.reportedByDevice",
						"reported by the device",
					)}
				>
					{failingText(t, time, row)}
				</KvRow>
			) : null}
		</>
	);
}

function CertificateDetails({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { detail } = row;
	const reported =
		row.reportedAt === null ? "" : time.at(Math.min(row.reportedAt, time.nowS));
	return (
		<div className="flex flex-col gap-2.5 py-1">
			<KeyValueList className="max-w-235">
				<KvRow
					label={t("certificates.detail.id", "Certificate ID")}
					provenance={t("certificates.detail.idNote", "reminders name this ID")}
				>
					<IdRef
						id={row.certificateId}
						copyLabel={t("certificates.detail.copyId", "Copy certificate ID")}
					/>
				</KvRow>
				<KvRow
					label={t("certificates.detail.fingerprint", "Fingerprint")}
					provenance={t("certificates.detail.sha256", "SHA-256")}
				>
					<IdRef
						id={row.fingerprint}
						group4
						copyLabel={t(
							"certificates.detail.copyFingerprint",
							"Copy fingerprint",
						)}
					/>
				</KvRow>
				<KvRow
					label={t("certificates.detail.expires", "Expires")}
					provenance={t(
						"certificates.detail.expiresNote",
						"earliest date in its chain",
					)}
				>
					{time.abs(row.notAfter)}
				</KvRow>
				<KvRow
					label={t("certificates.detail.version", "Version")}
					provenance={t(
						"certificates.detail.versionNote",
						"goes up each time it's replaced or renewed",
					)}
				>
					<span className="font-mono tabular-nums">{row.revision}</span>
				</KvRow>
				{detail ? (
					<>
						<KvRow label={t("certificates.detail.names", "Names")}>
							<span className="font-mono text-xs">
								{[...detail.dns_names, ...detail.ip_addresses].join(", ") ||
									t("certificates.detail.noNames", "none")}
							</span>
						</KvRow>
						<KvRow label={t("certificates.detail.issuer", "Issuer")}>
							{detail.issuer || "–"}
						</KvRow>
						<KvRow label={t("certificates.detail.validFrom", "Valid from")}>
							{time.abs(detail.not_before)}
						</KvRow>
						<RenewalFacts row={row} />
					</>
				) : (
					<KvRow label={t("certificates.detail.more", "Details")}>
						{t(
							"certificates.detail.connect",
							"Connect live to {{device}} to read the issuer, names and which services use it.",
							{ device: row.deviceName },
						)}
					</KvRow>
				)}
			</KeyValueList>
			<p className="max-w-[110ch] text-xs text-muted-foreground">
				{t(
					"certificates.detail.sourceHub",
					"ID, fingerprint and expiry come from the hub: {{device}} reported them {{when}}.",
					{ device: row.deviceName, when: reported },
				)}{" "}
				{detail
					? t(
							"certificates.detail.sourceLive",
							"Names, renewal and services come from this computer's live read of the device.",
						)
					: null}
			</p>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton size="sm" icon={ArrowRight} asChild>
					<a {...link(certificateRoute(row))}>
						{t("certificates.detail.openOn", "Open on {{device}}", {
							device: row.deviceName,
						})}
					</a>
				</DvButton>
				<DvButton
					size="sm"
					variant="ghost"
					icon={Copy}
					onClick={() => void copyText(row.certificateId)}
				>
					{t("certificates.detail.copyId", "Copy certificate ID")}
				</DvButton>
			</div>
		</div>
	);
}

/* Devices that list no certificate. */

function NoAccessReason({ entry }: Readonly<{ entry: SilentDevice }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	const shared = entry.relationship === "shared";
	const owner = usePerson(shared ? entry.device.owner_id : undefined);
	const access = useMyAccess(shared ? entry.device.device_id : undefined);
	const sharer = owner.name ?? t("certificates.silent.theOwner", "The owner");
	const appId = access.data?.grants.flatMap((grant) =>
		grant.scope.kind === "device" ? [] : [grant.scope.project_id],
	)[0];
	const tail = t(
		"certificates.silent.wholeDevice",
		"Certificates belong to the whole device.",
	);
	if (entry.relationship === "cloud_approval")
		return (
			<>
				{t(
					"certificates.silent.cloudOnly",
					"You see it because you approved cloud access for one of its services.",
				)}{" "}
				{tail}
			</>
		);
	if (!appId)
		return (
			<>
				{t(
					"certificates.silent.sharedPart",
					"{{owner}} shared it with you for part of the device only.",
					{ owner: sharer },
				)}{" "}
				{tail}
			</>
		);
	return (
		<>
			<Trans
				t={t}
				i18nKey="certificates.silent.sharedApp"
				defaults="{{owner}} shared it with you for App <1>{{app}}</1> only."
				values={{
					owner: sharer,
					app: appName(appId) ?? t("certificates.silent.anApp", "one app"),
				}}
				components={{
					1: (
						<a
							{...link(
								{ screen: "app-devices", by: "device" },
								{ scope: { kind: "app", appId } },
							)}
							className={LINK}
						/>
					),
				}}
			/>{" "}
			{tail}
		</>
	);
}

interface SilentCopy {
	title: string;
	text: ReactNode;
}

function silentCopy(
	t: DevicesT,
	time: AreaTime,
	entry: SilentDevice,
): SilentCopy {
	const { device, presence } = entry;
	switch (entry.state) {
		case "noaccess":
			return {
				title: t(
					"devices:certificates.silent.noAccessTitle",
					"No access · needs whole-device View status",
				),
				text: <NoAccessReason entry={entry} />,
			};
		case "never":
			return presence.kind === "never"
				? {
						title: t(
							"devices:certificates.silent.neverCheckedIn",
							"Hasn't reported (never checked in)",
						),
						text: t(
							"devices:certificates.silent.neverCheckedInText",
							"Registered {{ago}}. It reports certificates after its first check-in.",
							{ ago: time.ago(device.registered_at, "long") },
						),
					}
				: {
						title: t(
							"devices:certificates.silent.notReported",
							"The device hasn't reported certificates",
						),
						text:
							presence.since === undefined
								? t(
										"devices:certificates.silent.notReportedPlain",
										"No certificate report has reached the hub. Devices report on change and at least hourly.",
									)
								: t(
										"devices:certificates.silent.notReportedText",
										"It checks in (last {{ago}}), but no certificate report has reached the hub. Devices report on change and at least hourly.",
										{ ago: time.ago(presence.since, "long") },
									),
					};
		case "none":
			return {
				title: t("devices:certificates.silent.none", "No certificates"),
				text: t(
					"devices:certificates.silent.noneText",
					"The device reports none installed. Its services serve plain HTTP or none at all.",
				),
			};
		case "reading":
			return {
				title: t(
					"devices:certificates.silent.reading",
					"Reading its certificate report…",
				),
				text: t(
					"devices:certificates.silent.readingText",
					"This hub answers per device, so each report is read on its own.",
				),
			};
		default:
			return {
				title: t(
					"devices:certificates.silent.unread",
					"Certificate report not loaded",
				),
				text: t(
					"devices:certificates.silent.unreadText",
					"Open the device to read its certificates.",
				),
			};
	}
}

function SilentRow({
	entry,
	labels,
}: Readonly<{ entry: SilentDevice; labels: Labels }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const [copied, setCopied] = useState(false);
	const { device, deviceName: name } = entry;
	const copy = silentCopy(t, time, entry);
	const noAccess = entry.state === "noaccess";
	const ask = async () => {
		const ok = await copyText(
			t(
				"certificates.silent.askText",
				"Please add View status for the whole device {{device}} to my access, so I can see its certificates.",
				{ device: name },
			),
		);
		if (ok) setCopied(true);
	};
	return (
		<>
			<Tr data-silent={entry.state}>
				<Td label={labels.device} kind="name">
					<DeviceCell
						deviceId={device.device_id}
						name={name}
						presence={entry.presence}
						tab={noAccess ? "overview" : "certificates"}
					>
						{noAccess ? (
							<FreshnessStamp source="hub" age="noaccess" compact noFail />
						) : (
							<ReportStamp reportedAt={entry.reportedAt} />
						)}
					</DeviceCell>
				</Td>
				<Td label={labels.certificate} colSpan={5}>
					<span className="font-medium text-ink-2">{copy.title}</span>
					<CellSub>{copy.text}</CellSub>
				</Td>
				<Td label={labels.action} kind="act">
					{noAccess && entry.relationship === "shared" ? (
						<DvButton size="sm" icon={Copy} onClick={() => void ask()}>
							{t("certificates.silent.ask", "Ask the owner")}
						</DvButton>
					) : (
						<DvButton size="sm" asChild>
							<a
								{...link({
									screen: "device",
									deviceId: device.device_id,
									tab:
										noAccess || entry.presence.kind === "never"
											? "overview"
											: "certificates",
								})}
							>
								{t("certificates.action.open", "Open")}
							</a>
						</DvButton>
					)}
				</Td>
			</Tr>
			{copied ? (
				<tr data-confirm-row="">
					<td colSpan={COLS.length} className="px-4 pb-2">
						<InlineResult tone="good" onDismiss={() => setCopied(false)}>
							{t(
								"certificates.silent.asked",
								"Copied a request for the owner of {{device}}. Send it the way you usually reach them.",
								{ device: name },
							)}
						</InlineResult>
					</td>
				</tr>
			) : null}
		</>
	);
}

/* Toolbar and banner. */

/** Opens the device's keys and a live session, after which its certificate names load by themselves. */
function ReadNamesButton({
	deviceId,
	name,
	keys,
}: Readonly<{ deviceId: string; name: string; keys: KeyState | undefined }>) {
	const { t } = useTranslation("devices");
	const openUnlock = useOverlayStore((store) => store.openUnlock);
	const session = useLiveSession(deviceId);
	const busy = session.state.kind === "connecting";
	return (
		<DvButton
			size="sm"
			icon={Radio}
			busy={busy}
			onClick={() => {
				if (keys === "unlocked") void session.connect().catch(() => undefined);
				else openUnlock(deviceId, { connectLive: true });
			}}
		>
			{t("certificates.names.read", "Read names from {{device}}", {
				device: name,
			})}
		</DvButton>
	);
}

function hubStampText(
	t: DevicesT,
	time: AreaTime,
	freshness: Freshness,
): string | undefined {
	const stated = freshness.age === "current" || freshness.age === "delayed";
	return stated && freshness.at !== undefined
		? t(
				"devices:certificates.expiry.reportsChecked",
				"reports checked {{ago}}",
				{
					ago: time.ago(Math.min(freshness.at, time.nowS)),
				},
			)
		: undefined;
}

export interface ExpiryTabProps {
	read: CertificateFleetRead;
	counts: FleetCounts;
	filter: WindowFilter | null;
	onFilter(filter: WindowFilter | null): void;
	deviceId: string | null;
	onDevice(deviceId: string | null): void;
	/** An authority on this computer can sign today. */
	canSignHere: boolean;
	onRestoreAuthority(): void;
}

/** SPEC §5.8 Expiry across devices: every reported certificate, most urgent first, without unlocking. */
export function ExpiryTab({
	read,
	counts,
	filter,
	onFilter,
	deviceId,
	onDevice,
	canSignHere,
	onRestoreAuthority,
}: Readonly<ExpiryTabProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const input = useAttentionInput();
	const { fleet } = read;
	const [open, setOpen] = useState<string | null>(null);
	const [pages, setPages] = useState(1);
	const [allSilent, setAllSilent] = useState(false);
	const labels = useMemo(() => columnLabels(t), [t]);
	const keyStates = useMemo(
		() => new Map(input.keys.map((keys) => [keys.deviceId, keys.state])),
		[input.keys],
	);
	const shown = useMemo(
		() => filterFleet(fleet, filter, deviceId),
		[fleet, filter, deviceId],
	);
	const cap = PAGE_SIZE * pages;
	const silentCap = allSilent ? shown.silent.length : LIST_CAP;
	const named = fleet.rows.some((row) => row.detail);
	const firstReadable = fleet.rows.find(
		(row) =>
			!row.detail &&
			(row.presence.kind === "online" || row.presence.kind === "late") &&
			(keyStates.get(row.device.device_id) ?? "none") !== "none",
	);
	const setFilter = (next: WindowFilter | null) => {
		setPages(1);
		onFilter(next);
	};

	const chips = (
		<fieldset
			aria-label={t("certificates.expiry.filters", "Filter certificates")}
			className="m-0 flex min-w-0 flex-wrap items-center gap-1.5 border-0 p-0"
		>
			<FilterChip
				pressed={filter === null}
				onPressedChange={() => setFilter(null)}
			>
				{t("certificates.filter.all", "All")}
			</FilterChip>
			{ROW_FILTERS.map((id: RowFilter) => (
				<FilterChip
					key={id}
					pressed={filter === id}
					count={counts[id].length}
					onPressedChange={(pressed) => setFilter(pressed ? id : null)}
				>
					{filterLabel(t, id)}
				</FilterChip>
			))}
			{filter && !isRowFilter(filter) ? (
				<FilterChip
					pressed
					icon={X}
					count={shown.silent.length}
					onPressedChange={() => setFilter(null)}
				>
					{filterLabel(t, filter)}
				</FilterChip>
			) : null}
		</fieldset>
	);
	const countLine = shown.devicesOnly
		? filter === "noaccess"
			? t("certificates.expiry.countNoAccess", {
					count: shown.silent.length,
					defaultValue_one: "{{count, number}} device outside your access",
					defaultValue_other: "{{count, number}} devices outside your access",
				})
			: t("certificates.expiry.countSilent", {
					count: shown.silent.length,
					defaultValue_one: "{{count, number}} device that hasn't reported",
					defaultValue_other: "{{count, number}} devices that haven't reported",
				})
		: filter || deviceId
			? t(
					"certificates.expiry.countFiltered",
					"Showing {{shown, number}} of {{count, number}} · most urgent first",
					{ shown: shown.rows.length, count: fleet.rows.length },
				)
			: t(
					"certificates.expiry.countAll",
					"{{certificates}} on {{devices}} · most urgent first",
					{
						certificates: t("certificates.count.certificates", {
							count: fleet.rows.length,
							defaultValue_one: "{{count, number}} certificate",
							defaultValue_other: "{{count, number}} certificates",
						}),
						devices: t("certificates.count.devices", {
							count: fleet.devicesWithCertificates,
							defaultValue_one: "{{count, number}} device",
							defaultValue_other: "{{count, number}} devices",
						}),
					},
				);

	const stamp = (
		<span className="flex flex-wrap items-center gap-x-3 gap-y-1">
			<FreshnessStamp
				{...stampOf(read.freshness)}
				text={hubStampText(t, time, read.freshness)}
			/>
			{named ? (
				<FreshnessStamp
					source="local"
					age="current"
					text={t("certificates.expiry.namesRead", "names from live reads")}
				/>
			) : (
				<FreshnessStamp
					source="local"
					age="notloaded"
					text={t("certificates.expiry.noNamesRead", "no names read yet")}
				/>
			)}
		</span>
	);

	const revoked = fleet.revoked.slice(0, 4).map(deviceName).join(", ");
	const foot = (
		<span>
			{t(
				"certificates.expiry.foot",
				"Expiry comes from each device's report to the hub and needs no password. Names, how a certificate renews and which services use it come from this computer's live reads of each device.",
			)}
			{fleet.revoked.length ? (
				<>
					{" "}
					{fleet.revoked.length > 4
						? t(
								"certificates.expiry.revokedMore",
								"Revoked devices aren't read: {{names}} and {{count, number}} more.",
								{ names: revoked, count: fleet.revoked.length - 4 },
							)
						: t(
								"certificates.expiry.revoked",
								"Revoked devices aren't read: {{names}}.",
								{ names: revoked },
							)}
				</>
			) : null}
			{read.perDevice ? (
				<>
					{" "}
					{t(
						"certificates.expiry.perDevice",
						"This hub can't list every device's certificates at once, so each report is read on its own (the first {{count, number}} devices).",
						{ count: PER_DEVICE_READ_CAP },
					)}
				</>
			) : null}
		</span>
	);

	let body: ReactNode;
	if (!read.loaded)
		body = read.error ? (
			<div className="p-4">
				<StateView
					kind="error"
					title={t(
						"certificates.expiry.failed",
						"Couldn't read the certificate reports",
					)}
					text={hubErrorCopy(t, read.error.code)}
					actions={
						<DvButton
							size="sm"
							icon={RefreshCw}
							onClick={() => void read.refresh()}
						>
							{t("certificates.action.retry", "Try again")}
						</DvButton>
					}
				/>
			</div>
		) : (
			<div className="p-4">
				<StateView kind="loading" rows={4} />
			</div>
		);
	else
		body = (
			<>
				<DvTable
					cols={COLS}
					className={TABLE_RESET}
					label={t(
						"certificates.expiry.caption",
						"Certificates on your devices, most urgent first",
					)}
					head={
						<tr>
							<Th>{labels.device}</Th>
							<Th>{labels.certificate}</Th>
							<Th>
								<span className="inline-flex items-end gap-2.5">
									<ExpiryRailScale />
									<span>{labels.expires}</span>
								</span>
							</Th>
							<Th>{labels.status}</Th>
							<Th>{labels.renews}</Th>
							<Th>{labels.used}</Th>
							<Th>{labels.action}</Th>
						</tr>
					}
				>
					{shown.rows.slice(0, cap).map((row) => {
						const isOpen = open === row.key;
						return (
							<Fragment key={row.key}>
								<Tr selected={isOpen} data-certificate={row.certificateId}>
									<Td label={labels.device} kind="name">
										<DeviceCell
											deviceId={row.device.device_id}
											name={row.deviceName}
											presence={row.presence}
										>
											<ReportStamp reportedAt={row.reportedAt} />
										</DeviceCell>
									</Td>
									<Td label={labels.certificate} kind="name">
										<CertificateCell
											row={row}
											open={isOpen}
											onToggle={() => setOpen(isOpen ? null : row.key)}
										/>
									</Td>
									<Td label={labels.expires} kind="name">
										<ExpiresCell row={row} />
									</Td>
									<Td label={labels.status}>
										<StatusCell row={row} />
									</Td>
									<Td label={labels.renews}>
										<RenewsCell row={row} />
									</Td>
									<Td label={labels.used}>
										<UsedCell row={row} />
									</Td>
									<Td label={labels.action} kind="act">
										<ActionCell
											row={row}
											keys={keyStates.get(row.device.device_id)}
											canSignHere={canSignHere}
											onRestoreAuthority={onRestoreAuthority}
										/>
									</Td>
								</Tr>
								{isOpen ? (
									<tr data-expand="">
										<td
											colSpan={COLS.length}
											className="border-t border-hairline bg-surface-sunken px-4 py-3"
										>
											<CertificateDetails row={row} />
										</td>
									</tr>
								) : null}
							</Fragment>
						);
					})}
					{shown.silent.length ? (
						<>
							<GroupRow colSpan={COLS.length}>
								<b className="font-semibold text-foreground">
									{t(
										"certificates.expiry.silentGroup",
										"No certificates listed",
									)}
								</b>
								<span className="text-muted-foreground">
									{t("certificates.count.devices", {
										count: shown.silent.length,
										defaultValue_one: "{{count, number}} device",
										defaultValue_other: "{{count, number}} devices",
									})}
								</span>
								<span className="text-muted-foreground">
									{t(
										"certificates.expiry.silentGroupNote",
										"Not reported yet, outside your access, or none installed.",
									)}
								</span>
							</GroupRow>
							{shown.silent.slice(0, silentCap).map((entry) => (
								<SilentRow
									key={entry.device.device_id}
									entry={entry}
									labels={labels}
								/>
							))}
							{shown.silent.length > silentCap ? (
								<GroupRow colSpan={COLS.length}>
									<button
										type="button"
										onClick={() => setAllSilent(true)}
										className={LINK_BUTTON}
									>
										{t(
											"certificates.expiry.silentMore",
											"Show {{count, number}} more",
											{ count: shown.silent.length - silentCap },
										)}
									</button>
								</GroupRow>
							) : null}
						</>
					) : null}
					{!shown.rows.length && !shown.silent.length ? (
						<tr>
							<td colSpan={COLS.length} className="p-4">
								{fleet.devices.length ? (
									<StateView
										kind="empty"
										icon={Funnel}
										title={t(
											"certificates.expiry.noMatch",
											"No certificate matches this filter",
										)}
										text={t("certificates.expiry.noMatchText", {
											count: fleet.rows.length,
											defaultValue_one:
												"{{count, number}} certificate reported in total.",
											defaultValue_other:
												"{{count, number}} certificates reported in total.",
										})}
										actions={
											<DvButton
												size="sm"
												onClick={() => {
													setFilter(null);
													onDevice(null);
												}}
											>
												{t("certificates.expiry.clear", "Clear filters")}
											</DvButton>
										}
									/>
								) : (
									<StateView
										kind="empty"
										icon={FileBadge}
										title={t("certificates.expiry.noDevices", "No devices yet")}
										text={t(
											"certificates.expiry.noDevicesText",
											"Certificates show here once a device is set up and reports them.",
										)}
									/>
								)}
							</td>
						</tr>
					) : null}
				</DvTable>
				{shown.rows.length > cap ? (
					<div className="flex flex-wrap items-center justify-between gap-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
						<span>
							{t(
								"certificates.expiry.showing",
								"Showing {{shown, number}} of {{count, number}}",
								{ shown: cap, count: shown.rows.length },
							)}
						</span>
						<DvButton size="sm" onClick={() => setPages(pages + 1)}>
							{t("certificates.expiry.more", "Show {{count, number}} more", {
								count: Math.min(PAGE_SIZE, shown.rows.length - cap),
							})}
						</DvButton>
					</div>
				) : null}
			</>
		);

	return (
		<>
			{read.loaded && fleet.rows.length > 0 && !named ? (
				<Banner
					tone="info"
					icon={Laptop}
					title={t(
						"certificates.names.title",
						"No device was read live yet, so certificates show IDs only.",
					)}
					actions={
						firstReadable ? (
							<ReadNamesButton
								deviceId={firstReadable.device.device_id}
								name={firstReadable.deviceName}
								keys={keyStates.get(firstReadable.device.device_id)}
							/>
						) : undefined
					}
				>
					{t(
						"certificates.names.text",
						"The hub never learns certificate names. This computer reads them over a live connection once a device is unlocked. IDs, fingerprints and expiry dates below are complete.",
					)}
				</Banner>
			) : null}
			<Block
				id="certificates-expiry"
				icon={FileBadge}
				title={t("certificates.expiry.title", "Reported certificates")}
				count={read.loaded ? fleet.rows.length : undefined}
				stamp={stamp}
				toolbar={
					<>
						{chips}
						<DeviceSelect
							label={t("certificates.expiry.deviceFilter", "Device")}
							allLabel={t("certificates.expiry.allDevices", "All devices")}
							devices={fleet.devices.map((device) => ({
								id: device.device_id,
								name: deviceName(device),
							}))}
							value={deviceId}
							onChange={(next) => {
								setPages(1);
								onDevice(next);
							}}
						/>
						<span className="ml-auto text-xs whitespace-nowrap text-muted-foreground @max-[900px]/devices:ml-0 @max-[900px]/devices:basis-full @max-[900px]/devices:whitespace-normal">
							{read.loaded ? countLine : null}
						</span>
					</>
				}
				foot={foot}
				flush
			>
				{body}
			</Block>
		</>
	);
}
