"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	Ellipsis,
	FileBadge,
	FileKey,
	FileUp,
	Globe,
	Hourglass,
	type LucideIcon,
	OctagonX,
	RefreshCw,
	Trash2,
	TriangleAlert,
	Wrench,
} from "lucide-react";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { deleteCertificate } from "../../../../../lib/device-management/certificates";
import type { GateResult } from "../../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../../ui/dropdown-menu";
import { enumLabel } from "../../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { CellSub, DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import { ExpiryRail, ExpiryRailScale } from "../../primitives/expiry-rail";
import { GateInline } from "../../primitives/gate-notice";
import { IdRef } from "../../primitives/id-ref";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StatusChip } from "../../primitives/status-chip";
import { cx } from "../../primitives/tone";
import {
	type RouteLinkProps,
	useRouteLink,
} from "../../routing/use-devices-route";
import { useDeviceAction, useLocalSummary } from "../../workspace";
import {
	ActionResults,
	GatedButton,
	LINK,
	MENU_CONTENT,
	MENU_ITEM,
	Names,
	certificateResultKey,
	failureSummary,
	gateView,
	lastErrorText,
	listOf,
	neverHere,
	useDay,
	withDeviceReason,
} from "./certificate-parts";
import {
	type CertificateRow,
	type DeviceCertificates,
	certificateLabel,
	certificateNames,
} from "./use-device-certificates";

/** What the tab opens for a certificate; every flow is a sheet of its own. */
export interface CertificateFlows {
	renew(row: CertificateRow): void;
	replace(row: CertificateRow): void;
	importNew(): void;
	createRequest(row?: CertificateRow): void;
	setupRenewal(row: CertificateRow): void;
	/** An existing certificate or Let's Encrypt setup by ID; none for a new certificate. */
	setupAcme(certificateId?: string): void;
}

const COLS = ["13%", "15%", "24%", "14%", "13%", "21%"] as const;
const DAY_S = 86_400;

export const rowElementId = (certificateId: string) =>
	`certificate-${certificateId}`;

/** The result line of actions that add or remove a row. */
export const listResultKey = (deviceId: string) =>
	certificateResultKey(deviceId, "list");

function ValidityChip({
	row,
	now,
}: Readonly<{ row: CertificateRow; now: number }>) {
	const { t } = useTranslation("devices");
	const day = useDay();
	if (row.validity === "expired")
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{t("deviceCertificates.validity.expiredOn", "Expired {{date}}", {
					date: day(row.notAfter),
				})}
			</StatusChip>
		);
	if (row.validity === "expiring") {
		const days = Math.floor((row.notAfter - now) / DAY_S);
		return (
			<StatusChip tone="warning" icon={TriangleAlert}>
				{days >= 1
					? enumLabel(t, "validity", "expiring", { count: days })
					: t("deviceCertificates.validity.today", "Expires within a day")}
			</StatusChip>
		);
	}
	if (row.validity === "not_yet_valid")
		return (
			<StatusChip tone="info" icon={Hourglass}>
				{enumLabel(t, "validity", "not_yet_valid")}
			</StatusChip>
		);
	return (
		<StatusChip tone="good" icon={CircleCheck}>
			{enumLabel(t, "validity", "valid")}
		</StatusChip>
	);
}

function Muted({ children }: Readonly<{ children: ReactNode }>) {
	return <span className="text-muted-foreground">{children}</span>;
}

function RenewalCell({
	certs,
	row,
}: Readonly<{ certs: DeviceCertificates; row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	if (!row.detail)
		return (
			<Muted>
				{t(
					"deviceCertificates.list.unknownUntilLive",
					"Unknown until a live read",
				)}
			</Muted>
		);
	if (!row.mode)
		return (
			<Muted>
				{certs.support.canDelegate
					? t("deviceCertificates.list.readingRenewal", "Reading…")
					: t(
							"deviceCertificates.list.renewalOwnerOnly",
							"Only the owner sees how it renews",
						)}
			</Muted>
		);
	const staging = row.acme?.environment === "lets_encrypt_staging";
	return (
		<>
			{enumLabel(t, "certMode", row.mode)}
			{staging ? (
				<CellSub>
					{enumLabel(t, "acmeEnvironment", "lets_encrypt_staging")}
				</CellSub>
			) : null}
			{row.failing ? (
				<CellSub
					data-renewal-failing=""
					className="flex items-center gap-1 text-critical"
				>
					<OctagonX aria-hidden className="size-3" />
					{t("deviceCertificates.list.failing", "failing")}
				</CellSub>
			) : null}
		</>
	);
}

function UsedByCell({
	certs,
	row,
}: Readonly<{ certs: DeviceCertificates; row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (!row.detail)
		return (
			<Muted>
				{t(
					"deviceCertificates.list.unknownUntilLive",
					"Unknown until a live read",
				)}
			</Muted>
		);
	const services = [
		...new Set(row.detail.bindings.map((binding) => binding.placement_id)),
	];
	const total = row.uses ?? services.length;
	if (!total)
		return <Muted>{t("deviceCertificates.list.notUsed", "Not used")}</Muted>;
	if (!services.length)
		return (
			<>
				{t("deviceCertificates.list.usedByCount", {
					count: total,
					defaultValue_one: "{{count, number}} service",
					defaultValue_other: "{{count, number}} services",
				})}
			</>
		);
	return (
		<>
			{services.map((serviceId, index) => (
				<span key={serviceId}>
					{index ? ", " : null}
					<a
						className={cx("font-mono text-xs", LINK)}
						{...link({
							screen: "service",
							deviceId: certs.deviceId,
							serviceId,
							tab: "endpoint",
						})}
					>
						{serviceId}
					</a>
				</span>
			))}
			{total > row.detail.bindings.length ? (
				<CellSub>
					{t(
						"deviceCertificates.list.usesShown",
						"Showing {{shown, number}} of {{count, number}} uses",
						{ shown: row.detail.bindings.length, count: total },
					)}
				</CellSub>
			) : null}
		</>
	);
}

interface MenuEntry {
	id: string;
	label: string;
	icon: LucideIcon;
	note?: ReactNode;
	disabled?: boolean;
	danger?: boolean;
	separated?: boolean;
	run?(): void;
	link?: RouteLinkProps;
}

interface Gated {
	hidden: boolean;
	disabled: boolean;
	note?: ReactNode;
}

function gated(
	t: DevicesT,
	time: ReturnType<typeof useAreaTime>,
	result: GateResult,
): Gated {
	if (neverHere(result)) return { hidden: true, disabled: true };
	const view = gateView(t, time, result);
	return view
		? { hidden: false, disabled: true, note: view.gate.reason }
		: { hidden: false, disabled: false };
}

function inUseReason(t: DevicesT, count: number): string {
	return t("devices:deviceCertificates.delete.inUse", {
		count,
		defaultValue_one:
			"Used by {{count, number}} service. Assign another certificate first.",
		defaultValue_other:
			"Used by {{count, number}} services. Assign another certificate first.",
	});
}

function useDeleteCertificate(certs: DeviceCertificates) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const { deviceId, name, reload } = certs;
	return useCallback(
		async (row: CertificateRow) => {
			const certificate = row.detail;
			if (!certificate) return;
			const label = certificateLabel(row);
			const names = certificateNames(certificate);
			const automatic = row.mode === "acme" || row.mode === "delegated";
			const outcome = await actions.run({
				action: "certificate_delete",
				deviceId,
				target: { extra: { bindingCount: row.uses ?? 0 } },
				label: t(
					"deviceCertificates.delete.label",
					"Delete certificate {{label}}",
					{ label },
				),
				consequence: {
					what: t(
						"deviceCertificates.delete.what",
						"The certificate and its private key are deleted from {{device}}. Its ID can never be reused.",
						{ device: name },
					),
					who: names.length
						? t(
								"deviceCertificates.delete.who",
								"Nobody today: no service uses it. Anything that still trusts it for {{names}} can't connect with it any more.",
								{ names: listOf(time.locale, names) },
							)
						: t(
								"deviceCertificates.delete.whoNone",
								"Nobody: no service uses it.",
							),
					when: automatic
						? t(
								"deviceCertificates.delete.whenAutomatic",
								"Immediately. Automatic renewal stops.",
							)
						: t("deviceCertificates.delete.when", "Immediately."),
					undo: {
						reversible: false,
						text: t(
							"deviceCertificates.delete.undo",
							"Import or create a new one.",
						),
					},
				},
				strength: "none",
				confirm: {
					title: t(
						"deviceCertificates.delete.title",
						"Delete certificate {{label}}?",
						{ label },
					),
					sub: name,
					tone: "danger",
					icon: Trash2,
				},
				resultKey: listResultKey(deviceId),
				call: (context) =>
					withDeviceReason(context, (call) =>
						deleteCertificate(call, certificate),
					),
			});
			if (outcome.status === "done") await reload();
		},
		[actions, deviceId, name, reload, t, time.locale],
	);
}

function RowMenu({
	certs,
	row,
	flows,
	onDelete,
}: Readonly<{
	certs: DeviceCertificates;
	row: CertificateRow;
	flows: CertificateFlows;
	onDelete(row: CertificateRow): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const authorities = useLocalSummary().authorities.length;
	const label = certificateLabel(row);
	const uses = row.uses ?? 0;
	const replace = gated(t, time, certs.gate("certificate_import"));
	const request = gated(t, time, certs.gate("csr_create"));
	const renewal = gated(
		t,
		time,
		certs.gate("renewal_delegation", { authorityPresent: authorities > 0 }),
	);
	const acme = gated(t, time, certs.gate("acme_configure"));
	const remove = gated(
		t,
		time,
		certs.gate("certificate_delete", { bindingCount: uses }),
	);
	const actions: (MenuEntry & { gate: Gated })[] = [
		{
			id: "replace",
			label: t("deviceCertificates.menu.replaceFile", "Replace with file…"),
			icon: FileUp,
			gate: replace,
			run: () => flows.replace(row),
		},
		{
			id: "request",
			label: t("deviceCertificates.menu.request", "Create signing request…"),
			icon: FileKey,
			gate: request,
			run: () => flows.createRequest(row),
		},
		{
			id: "renewal",
			label: t("deviceCertificates.menu.renewal", "Set up automatic renewal…"),
			icon: RefreshCw,
			gate: renewal,
			run: () => flows.setupRenewal(row),
		},
		{
			id: "acme",
			label: t("deviceCertificates.menu.acme", "Set up Let's Encrypt…"),
			icon: Globe,
			gate:
				row.mode === "acme" && !acme.hidden
					? {
							hidden: false,
							disabled: true,
							note: t(
								"deviceCertificates.menu.acmeAlready",
								"Already renews with Let's Encrypt",
							),
						}
					: acme,
			run: () => flows.setupAcme(row.id),
		},
	];
	const shown = actions.filter((entry) => !entry.gate.hidden);
	const entries: MenuEntry[] = shown.map(({ gate, ...entry }) => ({
		...entry,
		disabled: gate.disabled || !row.detail,
		...(gate.note ? { note: gate.note } : {}),
	}));
	if (!remove.hidden)
		entries.push({
			id: "delete",
			label: t("deviceCertificates.menu.delete", "Delete…"),
			icon: Trash2,
			danger: true,
			separated: true,
			disabled: remove.disabled || !row.detail,
			...(remove.disabled
				? { note: uses > 0 ? inUseReason(t, uses) : remove.note }
				: {}),
			run: () => onDelete(row),
		});
	if (!entries.length) return null;
	entries.splice(shown.length, 0, {
		id: "all",
		label: t("deviceCertificates.menu.all", "All certificates"),
		icon: FileBadge,
		separated: shown.length > 0,
		link: link({ screen: "certificates", tab: "expiry" }),
	});
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton
					variant="ghost"
					size="sm"
					iconOnly
					icon={Ellipsis}
					aria-label={t("deviceCertificates.menu.more", "More for {{label}}", {
						label,
					})}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className={MENU_CONTENT}>
				{entries.map((entry) => {
					const Icon = entry.icon;
					const body = (
						<>
							<Icon aria-hidden className="mt-0.5 size-4 shrink-0" />
							<span className="min-w-0">
								{entry.label}
								{entry.note ? (
									<span className="block text-xs text-ink-2">{entry.note}</span>
								) : null}
							</span>
						</>
					);
					const className = cx(
						MENU_ITEM,
						entry.danger &&
							"text-critical focus:text-critical [&_svg]:text-critical!",
					);
					return (
						<div key={entry.id}>
							{entry.separated ? (
								<DropdownMenuSeparator className="bg-hairline" />
							) : null}
							{entry.link ? (
								<DropdownMenuItem
									asChild
									data-menu-item={entry.id}
									className={className}
								>
									<a {...entry.link}>{body}</a>
								</DropdownMenuItem>
							) : (
								<DropdownMenuItem
									disabled={entry.disabled}
									data-menu-item={entry.id}
									className={className}
									onSelect={() => entry.run?.()}
								>
									{body}
								</DropdownMenuItem>
							)}
						</div>
					);
				})}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function RowActions({
	certs,
	row,
	flows,
	onDelete,
}: Readonly<{
	certs: DeviceCertificates;
	row: CertificateRow;
	flows: CertificateFlows;
	onDelete(row: CertificateRow): void;
}>) {
	const { t } = useTranslation("devices");
	const authorities = useLocalSummary().authorities.length;
	const fixing =
		row.failing && (row.mode === "acme" || row.mode === "delegated");
	const result = !fixing
		? certs.gate("certificate_import")
		: row.mode === "acme"
			? certs.gate("acme_configure")
			: certs.gate("renewal_delegation", {
					authorityPresent: authorities > 0,
				});
	const open = () => {
		if (!row.detail) return;
		if (!fixing) flows.renew(row);
		else if (row.mode === "acme") flows.setupAcme(row.id);
		else flows.setupRenewal(row);
	};
	return (
		<>
			<div className="flex flex-wrap items-start gap-1.5">
				<GatedButton
					result={result}
					size="sm"
					icon={fixing ? Wrench : RefreshCw}
					disabled={result.ok && !row.detail}
					onClick={open}
				>
					{fixing
						? t("deviceCertificates.actions.fix", "Fix renewal…")
						: t("deviceCertificates.actions.renew", "Renew…")}
				</GatedButton>
				{row.detail ? (
					<RowMenu certs={certs} row={row} flows={flows} onDelete={onDelete} />
				) : null}
			</div>
			<ActionResults
				scopeKey={certificateResultKey(certs.deviceId, row.id)}
				className="mt-1.5"
			/>
		</>
	);
}

function RowDetail({
	certs,
	row,
}: Readonly<{ certs: DeviceCertificates; row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const detail = row.detail;
	const failure = row.issuer ?? row.acme;
	const summary = failure
		? failureSummary(t, failure, certs.support.failureDetail)
		: {};
	const error = failure
		? lastErrorText(
				t,
				failure,
				!!row.issuer && row.issuer.not_after <= certs.now,
			)
		: null;
	const id = (
		<KvRow
			label={t("deviceCertificates.detail.id", "Certificate ID")}
			provenance={t(
				"deviceCertificates.detail.idNote",
				"reminders name this ID",
			)}
		>
			<IdRef
				id={row.id}
				copyLabel={t("deviceCertificates.copyId", "Copy certificate ID")}
			/>
		</KvRow>
	);
	const fingerprint = (
		<KvRow label={t("deviceCertificates.detail.fingerprint", "Fingerprint")}>
			<IdRef
				id={row.fingerprint}
				group4
				copyLabel={t("deviceCertificates.copyFingerprint", "Copy fingerprint")}
			/>
		</KvRow>
	);
	const expires = (
		<KvRow label={t("deviceCertificates.detail.expires", "Expires")}>
			{time.abs(row.notAfter)}
		</KvRow>
	);
	if (!detail)
		return (
			<KeyValueList>
				{id}
				{fingerprint}
				{expires}
				<KvRow label={t("deviceCertificates.detail.more", "Details")}>
					{t(
						"deviceCertificates.detail.needsLive",
						"Read live to see the issuer, names and which services use it.",
					)}
				</KvRow>
			</KeyValueList>
		);
	return (
		<KeyValueList>
			<KvRow label={t("deviceCertificates.detail.issuer", "Issuer")}>
				{detail.issuer || t("deviceCertificates.detail.empty", "Empty")}
			</KvRow>
			<KvRow label={t("deviceCertificates.detail.subject", "Subject")}>
				{detail.subject || t("deviceCertificates.detail.empty", "Empty")}
			</KvRow>
			<KvRow label={t("deviceCertificates.detail.validFrom", "Valid from")}>
				{time.abs(detail.not_before)}
			</KvRow>
			{expires}
			{fingerprint}
			{id}
			<KvRow label={t("deviceCertificates.detail.version", "Version")}>
				<span className="font-mono tabular-nums">{detail.revision}</span>
			</KvRow>
			{row.failing ? (
				<KvRow label={t("deviceCertificates.detail.renewal", "Renewal")}>
					<span className="text-critical">
						{[summary.attempts, error].filter(Boolean).join(". ") ||
							t(
								"deviceCertificates.detail.renewalFailing",
								"The last attempt failed.",
							)}
					</span>
				</KvRow>
			) : null}
		</KeyValueList>
	);
}

function CertificateRowView({
	certs,
	row,
	flows,
	expanded,
	focused,
	onToggle,
	onDelete,
}: Readonly<{
	certs: DeviceCertificates;
	row: CertificateRow;
	flows: CertificateFlows;
	expanded: boolean;
	focused: boolean;
	onToggle(): void;
	onDelete(row: CertificateRow): void;
}>) {
	const { t } = useTranslation("devices");
	const detailId = `${rowElementId(row.id)}-detail`;
	const labels = {
		certificate: t("deviceCertificates.col.certificate", "Certificate"),
		validity: t("deviceCertificates.col.validity", "Validity"),
		names: t("deviceCertificates.col.names", "Names"),
		renews: t("deviceCertificates.col.renews", "How it renews"),
		usedBy: t("deviceCertificates.col.usedBy", "Used by"),
		actions: t("deviceCertificates.col.actions", "Actions"),
	};
	return (
		<>
			<Tr
				id={rowElementId(row.id)}
				data-certificate={row.id}
				data-focused={focused || undefined}
				selected={focused}
			>
				<Td label={labels.certificate} kind="name">
					<button
						type="button"
						aria-expanded={expanded}
						aria-controls={detailId}
						onClick={onToggle}
						className="cursor-pointer text-left font-semibold underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
					>
						{row.detail ? (
							row.detail.label
						) : (
							<span className="font-mono" title={row.id}>
								{row.id.slice(0, 8)}
							</span>
						)}
					</button>
					{/* Without a name the ID is the title; the row's details carry the copy control. */}
					{row.detail ? (
						<CellSub>
							<IdRef
								id={row.id}
								copyLabel={t(
									"deviceCertificates.copyId",
									"Copy certificate ID",
								)}
							/>
						</CellSub>
					) : null}
				</Td>
				<Td label={labels.validity}>
					<ValidityChip row={row} now={certs.now} />
					<CellSub>
						<ExpiryRail notAfter={row.notAfter} />
					</CellSub>
				</Td>
				<Td label={labels.names}>
					{row.detail ? (
						<Names
							names={certificateNames(row.detail)}
							empty={t("deviceCertificates.list.noNames", "No names")}
						/>
					) : (
						<Muted>
							{t(
								"deviceCertificates.list.nameAfterLive",
								"Name shows after a live read",
							)}
						</Muted>
					)}
				</Td>
				<Td label={labels.renews}>
					<RenewalCell certs={certs} row={row} />
				</Td>
				<Td label={labels.usedBy}>
					<UsedByCell certs={certs} row={row} />
				</Td>
				<Td label={labels.actions} kind="act">
					<RowActions
						certs={certs}
						row={row}
						flows={flows}
						onDelete={onDelete}
					/>
				</Td>
			</Tr>
			{expanded ? (
				<tr id={detailId} data-certificate-detail={row.id}>
					<td
						colSpan={COLS.length}
						className="border-t border-hairline bg-surface-sunken px-4 py-3"
					>
						<RowDetail certs={certs} row={row} />
					</td>
				</tr>
			) : null}
		</>
	);
}

/**
 * SPEC §5.2 Certificates tab: one row per certificate with its validity on the
 * shared expiry scale. A row opens to its facts; `focusId` (the reminder
 * link's `certificate=`) opens and marks its row.
 */
export function CertificateList({
	certs,
	flows,
	focusId,
}: Readonly<{
	certs: DeviceCertificates;
	flows: CertificateFlows;
	focusId?: string;
}>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState<string | null>(focusId ?? null);
	const [followed, setFollowed] = useState(focusId);
	if (focusId !== followed) {
		setFollowed(focusId);
		if (focusId) setOpen(focusId);
	}
	const remove = useDeleteCertificate(certs);
	const present = !!focusId && certs.rows.some((row) => row.id === focusId);
	useEffect(() => {
		if (!present || !focusId) return;
		const element = document.getElementById(rowElementId(focusId));
		if (typeof element?.scrollIntoView === "function")
			element.scrollIntoView({ block: "center" });
	}, [present, focusId]);
	return (
		<DvTable
			cols={COLS}
			label={t("deviceCertificates.list.label", "Certificates on {{device}}", {
				device: certs.name,
			})}
			head={
				<tr>
					<Th>{t("deviceCertificates.col.certificate", "Certificate")}</Th>
					<Th>
						<span className="inline-flex flex-col gap-0.5">
							<ExpiryRailScale />
							<span>{t("deviceCertificates.col.validity", "Validity")}</span>
						</span>
					</Th>
					<Th>{t("deviceCertificates.col.names", "Names")}</Th>
					<Th>{t("deviceCertificates.col.renews", "How it renews")}</Th>
					<Th>{t("deviceCertificates.col.usedBy", "Used by")}</Th>
					<Th>{t("deviceCertificates.col.actions", "Actions")}</Th>
				</tr>
			}
		>
			{certs.rows.map((row) => (
				<CertificateRowView
					key={row.id}
					certs={certs}
					row={row}
					flows={flows}
					expanded={open === row.id}
					focused={focusId === row.id}
					onToggle={() =>
						setOpen((current) => (current === row.id ? null : row.id))
					}
					onDelete={(target) => void remove(target)}
				/>
			))}
		</DvTable>
	);
}

/* Renew: a guided choice by how the certificate renews (IA §6.2 N2). */

function Choice({
	icon: Icon,
	title,
	hint,
	result,
	onChoose,
}: Readonly<{
	icon: LucideIcon;
	title: ReactNode;
	hint: ReactNode;
	result: GateResult;
	onChoose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (neverHere(result)) return null;
	const view = gateView(t, time, result);
	return (
		<div className="flex flex-col gap-1">
			<button
				type="button"
				data-choice=""
				aria-disabled={view ? true : undefined}
				onClick={view ? undefined : onChoose}
				className={cx(
					"flex w-full items-start gap-2.5 rounded-lg border border-border bg-card px-3 py-2.5 text-left hover:border-border-strong hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					view && "cursor-not-allowed opacity-60 hover:bg-card",
				)}
			>
				<Icon aria-hidden className="mt-0.5 size-4 shrink-0 text-ink-2" />
				<span className="flex min-w-0 flex-col gap-0.5">
					<span className="text-ui font-semibold">{title}</span>
					<span className="text-xs text-muted-foreground">{hint}</span>
				</span>
			</button>
			{view ? (
				<GateInline kind={view.gate.kind}>{view.gate.reason}</GateInline>
			) : null}
		</div>
	);
}

function RenewIntro({
	certs,
	row,
}: Readonly<{ certs: DeviceCertificates; row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const label = certificateLabel(row);
	if (row.mode === "acme" && row.acme)
		return (
			<p className="text-sm/5">
				{t(
					"deviceCertificates.renew.acmeIntro",
					"Let's Encrypt renews {{label}} by itself. The device's next attempt is {{when}}.",
					{ label, when: time.at(row.acme.next_attempt_at) },
				)}
			</p>
		);
	if (row.mode === "delegated" && row.issuer)
		return (
			<p className="text-sm/5">
				{row.issuer.not_after <= certs.now
					? t(
							"deviceCertificates.renew.delegatedExpired",
							"Your authority renewed {{label}} on the device until its renewal authority expired {{when}}. Install a new one to renew it again.",
							{ label, when: time.at(row.issuer.not_after) },
						)
					: t(
							"deviceCertificates.renew.delegatedIntro",
							"Your authority renews {{label}} on the device by itself. The next renewal is {{when}}.",
							{ label, when: time.at(row.issuer.next_renewal_at) },
						)}
			</p>
		);
	return (
		<p className="text-sm/5">
			{t(
				"deviceCertificates.renew.manualIntro",
				"{{label}} renews by replacing it. Pick how you get the new certificate.",
				{ label },
			)}
		</p>
	);
}

export function RenewSheet({
	certs,
	row,
	flows,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	row: CertificateRow;
	flows: CertificateFlows;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const authorities = useLocalSummary().authorities.length;
	const label = certificateLabel(row);
	const mode = row.mode ? enumLabel(t, "certMode", row.mode) : null;
	const automatic = row.mode === "acme" || row.mode === "delegated";
	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={RefreshCw}
			title={t("deviceCertificates.renew.title", "Renew {{label}}", { label })}
			sub={mode ? `${certs.name} · ${mode}` : certs.name}
			foot={
				<DvButton onClick={onClose}>
					{t("deviceCertificates.cancel", "Cancel")}
				</DvButton>
			}
		>
			<RenewIntro certs={certs} row={row} />
			<div className="flex flex-col gap-2">
				{row.mode === "delegated" ? (
					<Choice
						icon={FileKey}
						title={t(
							"deviceCertificates.renew.authority",
							"Install a new renewal authority",
						)}
						hint={t(
							"deviceCertificates.renew.authorityHint",
							"Your organisation authority signs a fresh one for the same names. The device then renews by itself again.",
						)}
						result={certs.gate("renewal_delegation", {
							authorityPresent: authorities > 0,
						})}
						onChoose={() => flows.setupRenewal(row)}
					/>
				) : null}
				{row.mode === "acme" ? (
					<Choice
						icon={Globe}
						title={t(
							"deviceCertificates.renew.acme",
							"Request a new certificate now",
						)}
						hint={t(
							"deviceCertificates.renew.acmeHint",
							"Review the Let's Encrypt settings and save them again. The device then asks for a new certificate within a minute.",
						)}
						result={certs.gate("acme_configure")}
						onChoose={() => flows.setupAcme(row.id)}
					/>
				) : null}
				<Choice
					icon={FileKey}
					title={t(
						"deviceCertificates.renew.request",
						"Create a signing request",
					)}
					hint={
						automatic
							? t(
									"deviceCertificates.renew.requestHintAutomatic",
									"The device makes a new key. Installing the signed certificate stops automatic renewal.",
								)
							: t(
									"deviceCertificates.renew.requestHint",
									"The device makes a new key. Get the request signed by any authority, or sign it here with your organisation authority, then install the chain.",
								)
					}
					result={certs.gate("csr_create")}
					onChoose={() => flows.createRequest(row)}
				/>
				<Choice
					icon={FileUp}
					title={t("deviceCertificates.renew.file", "Replace with a file")}
					hint={
						automatic
							? t(
									"deviceCertificates.renew.fileHintAutomatic",
									"You already have a new chain and key as PEM files. Replacing it stops automatic renewal.",
								)
							: t(
									"deviceCertificates.renew.fileHint",
									"You already have a new chain and key as PEM files.",
								)
					}
					result={certs.gate("certificate_import")}
					onChoose={() => flows.replace(row)}
				/>
			</div>
			<p className="text-xs text-muted-foreground">
				<a
					className={LINK}
					{...link({ screen: "certificates", tab: "authorities" })}
				>
					{t(
						"deviceCertificates.renew.authorities",
						"Organisation authorities on this computer",
					)}
				</a>
			</p>
		</DvSheet>
	);
}
