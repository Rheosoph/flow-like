"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Bell,
	Check,
	Copy,
	FileBadge,
	FileKey,
	FileUp,
	Globe,
	type LucideIcon,
	Plus,
	RefreshCw,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import type { CertificateRequest } from "../../../../lib/device-management/certificate-issuance";
import { classify } from "../../../../lib/device-management/model/freshness";
import type { GateResult } from "../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { AcmePanel, AcmeSetupSheet } from "../certificates/device/acme-panel";
import { CertificateImportSheet } from "../certificates/device/certificate-import-sheet";
import {
	type CertificateFlows,
	CertificateList,
	RenewSheet,
	listResultKey,
} from "../certificates/device/certificate-list";
import {
	ActionResults,
	GateFixButton,
	LINK,
	LiveStamp,
	MENU_CONTENT,
	MENU_ITEM,
	OwnerOnlyNote,
	gateView,
	neverHere,
} from "../certificates/device/certificate-parts";
import {
	RenewalPanel,
	RenewalSetupSheet,
} from "../certificates/device/renewal-panel";
import { SignWithAuthoritySheet } from "../certificates/device/sign-with-authority-sheet";
import {
	CreateRequestSheet,
	InstallChainSheet,
	SigningRequests,
} from "../certificates/device/signing-requests";
import {
	type DeviceCertificates,
	MAX_CERTIFICATE_SLOTS,
	useDeviceCertificates,
} from "../certificates/device/use-device-certificates";
import { CertificateReminderHistory } from "../certificates/reminder-history";
import { gateCopy } from "../copy/gate-copy";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { Meter, type MeterTone } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { useCopy } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import type { DeviceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { useCertificateNotices } from "../workspace";

type Sheet =
	| { kind: "renew" | "replace" | "renewal"; id: string }
	| { kind: "import" }
	| { kind: "request" | "acme"; id?: string }
	| { kind: "install" | "sign"; request: CertificateRequest };

const NEARLY_FULL = 28;

/** "Hub · device confirmed 30 min ago": when the device last reported its certificates, not when the app asked. */
function ConfirmedStamp({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { hub } = certs;
	const inventory = hub.data;
	if (!inventory || hub.freshness.age === "error")
		return <FreshnessStamp {...stampOf(hub.freshness)} />;
	if (inventory.updated_at === null)
		return (
			<FreshnessStamp
				source="hub"
				age="notloaded"
				text={t("deviceCertificates.stamp.neverReported", "never reported")}
			/>
		);
	const confirmed = classify("cert_inventory", {
		now: time.nowS,
		at: inventory.updated_at,
		loaded: true,
		noFail: true,
	});
	return (
		<FreshnessStamp
			{...stampOf(confirmed)}
			text={t(
				"deviceCertificates.stamp.confirmed",
				"device confirmed {{ago}}",
				{
					ago: time.ago(Math.min(inventory.updated_at, time.nowS)),
				},
			)}
		/>
	);
}

function SlotsMeter({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const { slots } = certs;
	if (!slots) return null;
	const tone: MeterTone =
		slots.used >= MAX_CERTIFICATE_SLOTS
			? "critical"
			: slots.used >= NEARLY_FULL
				? "warning"
				: "neutral";
	const counts = { used: slots.used, max: MAX_CERTIFICATE_SLOTS };
	const label = t(
		"deviceCertificates.slots.label",
		"{{used, number}} of {{max, number}} certificate slots used",
		counts,
	);
	return (
		<div data-certificate-slots="" className="flex flex-col gap-1.5">
			<Meter
				className="w-60 max-w-full"
				label={label}
				segments={[{ value: (slots.used / MAX_CERTIFICATE_SLOTS) * 100, tone }]}
			/>
			<p className="text-xs text-muted-foreground">
				{slots.complete
					? t(
							"deviceCertificates.slots.complete",
							"{{used, number}} of {{max, number}} certificate slots used (certificates, pending requests and Let's Encrypt policies).",
							counts,
						)
					: t(
							"deviceCertificates.slots.partial",
							"{{used, number}} of {{max, number}} certificate slots used by what you can see. Pending requests and Let's Encrypt policies count too.",
							counts,
						)}
			</p>
		</div>
	);
}

/** Why the list shows IDs only, with the way out ("Unlock…", "Connect live"). */
function DetailNotice({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const device = certs.name;
	if (certs.detail.loading)
		return (
			<output className="text-xs text-muted-foreground">
				{t(
					"deviceCertificates.notice.reading",
					"Reading names and details from {{device}}…",
					{ device },
				)}
			</output>
		);
	if (certs.detail.failed)
		return (
			<GateNotice
				kind="live"
				title={t(
					"deviceCertificates.notice.failed",
					"Names and details couldn't be read from {{device}}.",
					{ device },
				)}
				text={t(
					"deviceCertificates.notice.hubOnly",
					"The hub knows only IDs, fingerprints and expiry dates.",
				)}
				actions={
					<DvButton size="xs" onClick={() => void certs.reload()}>
						{t("deviceCertificates.notice.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	const result = certs.gate("certificates_list");
	if (result.ok) return null;
	const view = gateView(t, time, result);
	const code = result.copy.code;
	const offline =
		code === "offline_needs_live" || code === "never_connected_needs_live";
	const title =
		result.kind === "locked"
			? t(
					"deviceCertificates.notice.locked",
					"Unlock {{device}} to see names and details.",
					{ device },
				)
			: offline
				? t(
						"deviceCertificates.notice.offline",
						"{{device}} is offline, so names and details can't be read.",
						{ device },
					)
				: result.kind === "unsupported"
					? t(
							"deviceCertificates.notice.agent",
							"Update the agent on {{device}} to manage certificates remotely.",
							{ device },
						)
					: gateCopy(t, result, { at: time.at, locale: time.locale }).title;
	return (
		<GateNotice
			kind={result.kind}
			title={title}
			text={t(
				"deviceCertificates.notice.hubOnly",
				"The hub knows only IDs, fingerprints and expiry dates.",
			)}
			actions={view?.fix ? <GateFixButton view={view} /> : undefined}
		/>
	);
}

interface AddEntry {
	id: string;
	label: string;
	icon: LucideIcon;
	result: GateResult;
	run(): void;
}

function AddMenu({
	certs,
	flows,
}: Readonly<{ certs: DeviceCertificates; flows: CertificateFlows }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const all: AddEntry[] = [
		{
			id: "import",
			label: t("deviceCertificates.add.import", "Import from files…"),
			icon: FileUp,
			result: certs.gate("certificate_import"),
			run: flows.importNew,
		},
		{
			id: "request",
			label: t("deviceCertificates.add.request", "Create on the device…"),
			icon: FileKey,
			result: certs.gate("csr_create"),
			run: () => flows.createRequest(),
		},
		{
			id: "acme",
			label: t("deviceCertificates.add.acme", "Set up Let's Encrypt…"),
			icon: Globe,
			result: certs.gate("acme_configure"),
			run: () => flows.setupAcme(),
		},
	];
	const entries = all.filter((entry) => !neverHere(entry.result));
	if (!entries.length) return null;
	const full = (certs.slots?.used ?? 0) >= MAX_CERTIFICATE_SLOTS;
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton size="sm" icon={Plus}>
					{t("deviceCertificates.add.button", "Add certificate")}
				</DvButton>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className={MENU_CONTENT}>
				{entries.map((entry) => {
					const Icon = entry.icon;
					const view = gateView(t, time, entry.result);
					const note: ReactNode = view
						? view.gate.reason
						: full
							? t(
									"deviceCertificates.add.full",
									"All {{max, number}} certificate slots are used. Delete a certificate or discard a request first.",
									{ max: MAX_CERTIFICATE_SLOTS },
								)
							: null;
					return (
						<DropdownMenuItem
							key={entry.id}
							disabled={!!note}
							data-menu-item={entry.id}
							className={MENU_ITEM}
							onSelect={entry.run}
						>
							<Icon aria-hidden className="mt-0.5 size-4 shrink-0" />
							<span className="min-w-0">
								{entry.label}
								{note ? (
									<span className="block text-xs text-ink-2">{note}</span>
								) : null}
							</span>
						</DropdownMenuItem>
					);
				})}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function RefreshButton({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const [busy, setBusy] = useState(false);
	const refresh = async () => {
		setBusy(true);
		try {
			await certs.reload();
		} finally {
			setBusy(false);
		}
	};
	return (
		<DvButton
			variant="ghost"
			size="sm"
			icon={RefreshCw}
			busy={busy}
			onClick={() => void refresh()}
		>
			{t("deviceCertificates.refresh", "Refresh")}
		</DvButton>
	);
}

function NoAccess({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const request = t(
		"deviceCertificates.noAccess.request",
		"Please add View status for the whole device {{device}} to my access.",
		{ device: certs.name },
	);
	return (
		<GateNotice
			kind="noaccess"
			title={t(
				"deviceCertificates.noAccess.title",
				"Needs whole-device View status or Manage certificates.",
			)}
			text={t(
				"deviceCertificates.noAccess.text",
				"You can see certificates used by your services only: a service's Web endpoint names the one it uses.",
			)}
			actions={
				certs.owner ? undefined : (
					<DvButton
						size="sm"
						icon={copied ? Check : Copy}
						onClick={() => void copy(request)}
					>
						{copied
							? t("deviceCertificates.noAccess.copied", "Request copied")
							: t(
									"deviceCertificates.noAccess.ask",
									"Copy a request for the owner",
								)}
					</DvButton>
				)
			}
		/>
	);
}

function NothingReported({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="never"
			title={t(
				"deviceCertificates.never.title",
				"The device hasn't reported certificates",
			)}
			text={
				certs.presence === "never"
					? t(
							"deviceCertificates.never.notCheckedIn",
							"It hasn't checked in yet. Devices report certificates on change and at least hourly.",
						)
					: t(
							"deviceCertificates.never.text",
							"It reports certificate IDs and expiry to the hub on change and at least hourly. None have arrived yet.",
						)
			}
		/>
	);
}

type ListState = "noaccess" | "loading" | "error" | "never" | "empty" | "rows";

function listState(certs: DeviceCertificates): ListState {
	const { hub, detail, rows } = certs;
	if (detail.inventory) return rows.length ? "rows" : "empty";
	const summary = certs.gate("view_cert_summary");
	const refused =
		hub.freshness.age === "noaccess" ||
		(!summary.ok && summary.kind === "noaccess") ||
		!!detail.rejected;
	if (refused) return "noaccess";
	if (!hub.data) return hub.error ? "error" : "loading";
	if (rows.length) return "rows";
	return hub.data.updated_at === null ? "never" : "empty";
}

function CertificatesBlock({
	certs,
	flows,
	focusId,
	state,
}: Readonly<{
	certs: DeviceCertificates;
	flows: CertificateFlows;
	focusId?: string;
	state: ListState;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const hasDetail = !!certs.detail.inventory;
	const listed = state === "rows" || state === "empty";
	const missing =
		!!focusId && listed && !certs.rows.some((row) => row.id === focusId);
	const body: Record<ListState, () => ReactNode> = {
		noaccess: () => <NoAccess certs={certs} />,
		loading: () => (
			<StateView
				kind="loading"
				rows={3}
				title={t(
					"deviceCertificates.loading",
					"Loading this device's certificates…",
				)}
			/>
		),
		error: () => (
			<StateView
				kind="error"
				title={t(
					"deviceCertificates.error.title",
					"Couldn't read this device's certificates from the hub",
				)}
				text={t(
					"deviceCertificates.error.text",
					"Nothing is known about them on this computer yet.",
				)}
				actions={
					<DvButton size="sm" onClick={() => void certs.hub.refetch()}>
						{t("deviceCertificates.error.retry", "Retry now")}
					</DvButton>
				}
			/>
		),
		never: () => (
			<>
				<NothingReported certs={certs} />
				{certs.presence === "never" ? null : <DetailNotice certs={certs} />}
			</>
		),
		empty: () => (
			<>
				<SlotsMeter certs={certs} />
				<StateView
					kind="empty"
					icon={FileBadge}
					title={t(
						"deviceCertificates.empty.title",
						"No certificates on this device",
					)}
					text={
						hasDetail
							? t(
									"deviceCertificates.empty.text",
									"Create one on the device, import a file, or set up Let's Encrypt for a public name.",
								)
							: t(
									"deviceCertificates.empty.textHub",
									"The device reported none. Its services serve plain HTTP or no web page at all.",
								)
					}
				/>
				{hasDetail ? null : <DetailNotice certs={certs} />}
			</>
		),
		rows: () => (
			<>
				{certs.slots || !hasDetail || missing ? (
					<div className="flex flex-col gap-3 px-4 py-3">
						<SlotsMeter certs={certs} />
						{hasDetail ? null : <DetailNotice certs={certs} />}
						{missing ? (
							<InlineResult tone="warning">
								{t(
									"deviceCertificates.focusMissing",
									"The certificate this link points to isn't on {{device}} any more. It may have been deleted.",
									{ device: certs.name },
								)}
							</InlineResult>
						) : null}
					</div>
				) : null}
				<CertificateList
					certs={certs}
					flows={flows}
					{...(focusId ? { focusId } : {})}
				/>
			</>
		),
	};
	const flush = state === "rows";
	return (
		<Block
			id="device-certificates"
			icon={FileBadge}
			title={t("deviceCertificates.title", "Certificates")}
			{...(listed ? { count: certs.rows.length } : {})}
			stamp={
				state === "noaccess" || state === "loading" ? undefined : (
					<>
						<ConfirmedStamp certs={certs} />
						{hasDetail ? (
							<LiveStamp freshness={certs.detail.freshness} details />
						) : null}
					</>
				)
			}
			tools={
				state === "noaccess" || state === "loading" ? undefined : (
					<>
						<AddMenu certs={certs} flows={flows} />
						<RefreshButton certs={certs} />
					</>
				)
			}
			flush={flush}
			foot={
				state === "noaccess" ? undefined : (
					<>
						<a
							className={LINK}
							{...link({ screen: "certificates", tab: "expiry" })}
						>
							{t(
								"deviceCertificates.allLink",
								"All certificates across your devices",
							)}
						</a>
						<span>
							{t(
								"deviceCertificates.remindersNote",
								"Reminders go to you and people with whole-device View status or Manage certificates: 7 days, 3 days and 1 day before expiry and on the day, by push and email.",
							)}
						</span>
					</>
				)
			}
		>
			{body[state]()}
			<ActionResults
				scopeKey={listResultKey(certs.deviceId)}
				className={flush ? "px-4 py-3" : undefined}
			/>
		</Block>
	);
}

/** IA §6.2 N2: for everyone but the owner the renewal sections are a read-only explanation. */
function RenewalReadOnly({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			icon={RefreshCw}
			title={t("deviceCertificates.renewalReadOnly.title", "Automatic renewal")}
		>
			<p className="max-w-[72ch] text-ui">
				{t(
					"deviceCertificates.renewalReadOnly.text",
					"The owner can let this device renew certificates by itself, with the organisation's authority or with Let's Encrypt. How each certificate renews is visible to the owner only.",
				)}
			</p>
			<OwnerOnlyNote ownerId={certs.ownerId} />
		</Block>
	);
}

/** BG26: who is reminded, what was sent, mute and test, from the hub. */
function RemindersBlock({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const notices = useCertificateNotices(certs.deviceId);
	return (
		<Block
			id="device-certificate-reminders"
			icon={Bell}
			title={t("deviceCertificates.reminders.title", "Reminders")}
			stamp={<FreshnessStamp {...stampOf(notices.freshness)} />}
			foot={
				<a
					className={LINK}
					{...link({ screen: "certificates", tab: "reminders" })}
				>
					{t(
						"deviceCertificates.reminders.allLink",
						"Reminders across your devices",
					)}
				</a>
			}
		>
			<p className="max-w-[80ch] text-ui">
				<Trans
					t={t}
					i18nKey="deviceCertificates.reminders.who"
					defaults="Reminders go to you and people with whole-device View status or Manage certificates. Stages: <1>7 d, 3 d, 1 d</1> and on expiry, by push and email."
					components={{ 1: <span className="whitespace-nowrap" /> }}
				/>
			</p>
			<CertificateReminderHistory deviceId={certs.deviceId} compact />
		</Block>
	);
}

/** Agents from before signing requests and automatic renewal: one sentence instead of controls that can't work. */
function OlderAgentNote({ certs }: Readonly<{ certs: DeviceCertificates }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	return (
		<p
			data-older-agent=""
			className="max-w-[80ch] text-xs text-muted-foreground"
		>
			{t(
				"deviceCertificates.olderAgent",
				"This device's agent can't create signing requests or renew certificates by itself yet.",
			)}{" "}
			<a
				className={LINK}
				{...link({
					screen: "device",
					deviceId: certs.deviceId,
					tab: "settings",
				})}
			>
				{t("deviceCertificates.olderAgentLink", "Update the agent")}
			</a>
		</p>
	);
}

function Sheets({
	certs,
	sheet,
	flows,
	open,
	close,
}: Readonly<{
	certs: DeviceCertificates;
	sheet: Sheet | null;
	flows: CertificateFlows;
	open(sheet: Sheet): void;
	close(): void;
}>) {
	if (!sheet) return null;
	const row =
		"id" in sheet && sheet.id
			? certs.rows.find((entry) => entry.id === sheet.id)
			: undefined;
	if (sheet.kind === "import")
		return <CertificateImportSheet certs={certs} onClose={close} />;
	if (sheet.kind === "request")
		return (
			<CreateRequestSheet
				certs={certs}
				{...(row ? { row } : {})}
				onClose={close}
			/>
		);
	if (sheet.kind === "acme")
		return (
			<AcmeSetupSheet
				certs={certs}
				{...(sheet.id ? { certificateId: sheet.id } : {})}
				onClose={close}
			/>
		);
	if (sheet.kind === "install" || sheet.kind === "sign") {
		const { request_id: requestId } = sheet.request;
		const request =
			certs.requests.data?.find((entry) => entry.request_id === requestId) ??
			sheet.request;
		return sheet.kind === "install" ? (
			<InstallChainSheet certs={certs} request={request} onClose={close} />
		) : (
			<SignWithAuthoritySheet certs={certs} request={request} onClose={close} />
		);
	}
	if (!row) return null;
	if (sheet.kind === "renew")
		return <RenewSheet certs={certs} row={row} flows={flows} onClose={close} />;
	if (sheet.kind === "replace")
		return <CertificateImportSheet certs={certs} row={row} onClose={close} />;
	return (
		<RenewalSetupSheet
			certs={certs}
			row={row}
			onClose={close}
			onPrepared={(request) => open({ kind: "sign", request })}
		/>
	);
}

function CertificatesTabView({
	certs,
	focusId,
}: Readonly<{ certs: DeviceCertificates; focusId?: string }>) {
	const [sheet, setSheet] = useState<Sheet | null>(null);
	const flows = useMemo<CertificateFlows>(
		() => ({
			renew: (row) => setSheet({ kind: "renew", id: row.id }),
			replace: (row) => setSheet({ kind: "replace", id: row.id }),
			importNew: () => setSheet({ kind: "import" }),
			createRequest: (row) =>
				setSheet({ kind: "request", ...(row ? { id: row.id } : {}) }),
			setupRenewal: (row) => setSheet({ kind: "renewal", id: row.id }),
			setupAcme: (id) => setSheet({ kind: "acme", ...(id ? { id } : {}) }),
		}),
		[],
	);
	const state = listState(certs);
	const { support } = certs;
	const detailed = !!certs.detail.inventory;
	return (
		<div
			data-device-certificates={certs.deviceId}
			className="flex min-w-0 flex-col gap-4"
		>
			<CertificatesBlock
				certs={certs}
				flows={flows}
				state={state}
				{...(focusId ? { focusId } : {})}
			/>
			{detailed && support.canManage && !support.issuance ? (
				<OlderAgentNote certs={certs} />
			) : null}
			{detailed && certs.requests.enabled ? (
				<SigningRequests
					certs={certs}
					onInstall={(request) => setSheet({ kind: "install", request })}
					onSign={(request) => setSheet({ kind: "sign", request })}
				/>
			) : null}
			{detailed && certs.issuers.enabled ? (
				<RenewalPanel certs={certs} flows={flows} />
			) : null}
			{detailed && certs.acme.enabled ? (
				<AcmePanel certs={certs} flows={flows} />
			) : null}
			{detailed && !certs.owner ? <RenewalReadOnly certs={certs} /> : null}
			{state === "noaccess" ? null : <RemindersBlock certs={certs} />}
			<Sheets
				certs={certs}
				sheet={sheet}
				flows={flows}
				open={setSheet}
				close={() => setSheet(null)}
			/>
		</div>
	);
}

/** N2 › Certificates (SPEC §5.2): the device-scoped view of the certificates screen. */
export function DeviceCertificatesTab({
	route,
	deviceId,
}: Readonly<DeviceTabProps>) {
	const { t } = useTranslation("devices");
	const certs = useDeviceCertificates(deviceId);
	const focusId = route.screen === "device" ? route.certificateId : undefined;
	if (!certs)
		return (
			<StateView
				kind="loading"
				rows={3}
				title={t(
					"deviceCertificates.loading",
					"Loading this device's certificates…",
				)}
			/>
		);
	return (
		<CertificatesTabView
			key={deviceId}
			certs={certs}
			{...(focusId ? { focusId } : {})}
		/>
	);
}
