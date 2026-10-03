"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleStop,
	FileKey,
	OctagonX,
	RefreshCw,
	ShieldCheck,
} from "lucide-react";
import { useId, useState } from "react";
import {
	type CertificateIssuer,
	type CertificateRequest,
	createCertificateIssuerRequest,
	deleteCertificateIssuer,
} from "../../../../../lib/device-management/certificate-issuance";
import { useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { Field, InputWithUnit } from "../../primitives/form-fields";
import { GateNotice } from "../../primitives/gate-notice";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StateView } from "../../primitives/state-view";
import { useRouteLink } from "../../routing/use-devices-route";
import {
	useDeviceAction,
	useInlineResults,
	useLocalSummary,
} from "../../workspace";
import type { CertificateFlows } from "./certificate-list";
import {
	ActionResults,
	GatedButton,
	LINK,
	LiveStamp,
	Names,
	SheetBlocker,
	certificateResultKey,
	failureSummary,
	gateView,
	lastErrorText,
	listOf,
	useAlive,
	useDay,
	withDeviceReason,
} from "./certificate-parts";
import { requestsResultKey } from "./signing-requests";
import {
	type CertificateRow,
	type DeviceCertificates,
	certificateLabel,
	certificateNames,
	issuerFailing,
} from "./use-device-certificates";

const MAX_LIFETIME_DAYS = 397;
const DEFAULT_LIFETIME_DAYS = 30;

export const renewalResultKey = (deviceId: string) =>
	certificateResultKey(deviceId, "renewal");

function useStopRenewal(certs: DeviceCertificates) {
	const { t } = useTranslation("devices");
	const day = useDay();
	const actions = useDeviceAction();
	return async (issuer: CertificateIssuer, row: CertificateRow | undefined) => {
		const label = row
			? certificateLabel(row)
			: issuer.certificate_id.slice(0, 8);
		const users = row?.uses ?? 0;
		const outcome = await actions.run({
			action: "renewal_delegation",
			deviceId: certs.deviceId,
			label: t(
				"deviceCertificates.renewal.stopLabel",
				"Stop automatic renewal of {{label}}",
				{ label },
			),
			consequence: {
				what: t(
					"deviceCertificates.renewal.stopWhat",
					"The device stops renewing {{label}} with your authority and destroys the renewal authority it holds.",
					{ label },
				),
				who: users
					? t(
							"deviceCertificates.renewal.stopWhoUsed",
							"Nobody yet: the services that use {{label}} keep working until it expires.",
							{ label },
						)
					: t(
							"deviceCertificates.renewal.stopWho",
							"Nobody: no service uses {{label}} today.",
							{ label },
						),
				when: row
					? t(
							"deviceCertificates.renewal.stopWhen",
							"{{label}} keeps working until {{date}}, then expires.",
							{ label, date: day(row.notAfter) },
						)
					: t("deviceCertificates.renewal.stopWhenNow", "Immediately."),
				undo: {
					reversible: true,
					text: t(
						"deviceCertificates.renewal.stopUndo",
						"Set up automatic renewal again.",
					),
				},
			},
			strength: "none",
			confirm: {
				title: t(
					"deviceCertificates.renewal.stopTitle",
					"Stop automatic renewal of {{label}}?",
					{ label },
				),
				sub: certs.name,
				tone: "danger",
				icon: CircleStop,
			},
			resultKey: renewalResultKey(certs.deviceId),
			call: (context) =>
				withDeviceReason(context, (call) =>
					deleteCertificateIssuer(call, issuer),
				),
		});
		if (outcome.status === "done") await certs.reload();
	};
}

function RenewalPolicy({
	certs,
	issuer,
	flows,
	onStop,
}: Readonly<{
	certs: DeviceCertificates;
	issuer: CertificateIssuer;
	flows: CertificateFlows;
	onStop(issuer: CertificateIssuer, row: CertificateRow | undefined): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const day = useDay();
	const link = useRouteLink();
	const authorities = useLocalSummary().authorities.length;
	const row = certs.rows.find((entry) => entry.id === issuer.certificate_id);
	const label = row ? certificateLabel(row) : issuer.certificate_id.slice(0, 8);
	const expired = issuer.not_after <= certs.now;
	const summary = failureSummary(t, issuer, certs.support.failureDetail);
	const error = lastErrorText(t, issuer, expired);
	const failing = issuerFailing(certs.now, issuer);
	const overdue = issuer.next_renewal_at <= certs.now;
	const gate = certs.gate("renewal_delegation", {
		authorityPresent: authorities > 0,
	});
	return (
		<div
			data-renewal-policy={issuer.certificate_id}
			className="flex flex-col gap-3"
		>
			{failing ? (
				<Banner
					tone="critical"
					title={t(
						"deviceCertificates.renewal.failingTitle",
						"Automatic renewal of {{label}} is failing.",
						{ label },
					)}
				>
					{expired
						? row
							? t(
									"deviceCertificates.renewal.failingExpired",
									"The authority that renews it expired {{at}}. The certificate expires {{when}} ({{date}}) unless you install a new one.",
									{
										at: time.at(issuer.not_after),
										when: time.ago(row.notAfter, "long"),
										date: day(row.notAfter),
									},
								)
							: t(
									"deviceCertificates.renewal.failingExpiredNoRow",
									"The authority that renews it expired {{at}}. Install a new one.",
									{ at: time.at(issuer.not_after) },
								)
						: [summary.attempts, error].filter(Boolean).join(". ") ||
							t(
								"deviceCertificates.renewal.failingGeneric",
								"The device's last attempt failed. It tries again by itself.",
							)}
				</Banner>
			) : null}
			<KeyValueList>
				<KvRow
					label={t("deviceCertificates.renewal.certificate", "Certificate")}
				>
					<b className="font-semibold">{label}</b>
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.names", "Approved names")}>
					<Names names={certificateNames(issuer)} />
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.authority", "Authority")}>
					{expired ? (
						<span
							data-authority-expired=""
							className="inline-flex items-center gap-1 text-critical"
						>
							<OctagonX aria-hidden className="size-3.5" />
							{t(
								"deviceCertificates.renewal.authorityExpired",
								"expired {{at}}",
								{
									at: time.at(issuer.not_after),
								},
							)}
						</span>
					) : (
						t(
							"deviceCertificates.renewal.authorityValid",
							"valid until {{date}}",
							{
								date: day(issuer.not_after),
							},
						)
					)}
				</KvRow>
				<KvRow
					label={t(
						"deviceCertificates.renewal.lifetime",
						"Certificate lifetime",
					)}
				>
					{t("deviceCertificates.renewal.lifetimeDays", "{{count, number}} d", {
						count: issuer.leaf_lifetime_days,
					})}
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.next", "Next renewal")}>
					{overdue
						? t("deviceCertificates.renewal.nextOverdue", "was due {{at}}", {
								at: time.at(issuer.next_renewal_at),
							})
						: time.at(issuer.next_renewal_at)}
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.last", "Last renewed")}>
					{issuer.last_renewed_at === null
						? t("deviceCertificates.renewal.never", "Not yet")
						: time.at(issuer.last_renewed_at)}
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.lastError", "Last error")}>
					{error ?? t("deviceCertificates.renewal.noError", "None")}
					{summary.attempts ? (
						<span
							data-failures=""
							className="block text-xs text-muted-foreground"
						>
							{summary.attempts}
						</span>
					) : null}
					{summary.note ? (
						<span
							data-failure-note=""
							className="block text-xs text-muted-foreground"
						>
							{summary.note}
						</span>
					) : null}
				</KvRow>
			</KeyValueList>
			<div className="flex flex-wrap items-start gap-2">
				<GatedButton
					result={gate}
					icon={FileKey}
					disabled={gate.ok && !row?.detail}
					onClick={() => {
						if (row) flows.setupRenewal(row);
					}}
				>
					{t(
						"deviceCertificates.renewal.install",
						"Install new renewal authority…",
					)}
				</GatedButton>
				<GatedButton
					result={certs.gate("renewal_delegation")}
					variant="danger-ghost"
					onClick={() => onStop(issuer, row)}
				>
					{t("deviceCertificates.renewal.stop", "Stop renewal…")}
				</GatedButton>
				<a
					className="inline-flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-ui font-medium hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
					{...link({ screen: "certificates", tab: "authorities" })}
				>
					<ShieldCheck aria-hidden className="size-4" />
					{t(
						"deviceCertificates.renewal.authorities",
						"Organisation authorities",
					)}
				</a>
			</div>
		</div>
	);
}

/** S34: certificates the device renews by itself with an authority the owner signed for it. */
export function RenewalPanel({
	certs,
	flows,
}: Readonly<{ certs: DeviceCertificates; flows: CertificateFlows }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { issuers } = certs;
	const stop = useStopRenewal(certs);
	const results = useInlineResults(renewalResultKey(certs.deviceId));
	const rows = issuers.data;
	// No certificate renews this way and nothing to report: a certificate's menu sets it up.
	if (rows && !rows.length && !results.length) return null;
	return (
		<Block
			id="device-certificate-renewal"
			icon={RefreshCw}
			title={t(
				"deviceCertificates.renewal.title",
				"Automatic renewal · your authority",
			)}
			stamp={<LiveStamp freshness={issuers.freshness} />}
		>
			{!rows ? (
				issuers.failed ? (
					<StateView
						kind="error"
						title={t(
							"deviceCertificates.renewal.error",
							"Couldn't read the renewal settings from {{device}}",
							{ device: certs.name },
						)}
						actions={
							<DvButton size="sm" onClick={() => void issuers.refetch()}>
								{t("deviceCertificates.error.retry", "Retry now")}
							</DvButton>
						}
					/>
				) : (
					<StateView
						kind="loading"
						rows={2}
						title={t(
							"deviceCertificates.renewal.loading",
							"Reading renewal settings…",
						)}
					/>
				)
			) : rows.length ? (
				rows.map((issuer, index) => (
					<div
						key={issuer.certificate_id}
						className={index ? "border-t border-hairline pt-3" : undefined}
					>
						<RenewalPolicy
							certs={certs}
							issuer={issuer}
							flows={flows}
							onStop={(target, row) => void stop(target, row)}
						/>
					</div>
				))
			) : (
				<p className="max-w-[80ch] text-ui text-muted-foreground">
					{t(
						"deviceCertificates.renewal.none",
						"No certificate renews with your authority yet. Open a certificate's menu and choose Set up automatic renewal: your organisation authority signs a renewal authority for exactly its names, and the device then renews it by itself, even while the hub is unreachable.",
					)}{" "}
					<a
						className={LINK}
						{...link({ screen: "certificates", tab: "authorities" })}
					>
						{t(
							"deviceCertificates.renewal.authorities",
							"Organisation authorities",
						)}
					</a>
				</p>
			)}
			<ActionResults scopeKey={renewalResultKey(certs.deviceId)} />
		</Block>
	);
}

/* Set up (or repair) automatic renewal: step 1 of 2, the device makes the request. */

export function RenewalSetupSheet({
	certs,
	row,
	onClose,
	onPrepared,
}: Readonly<{
	certs: DeviceCertificates;
	row: CertificateRow;
	onClose(): void;
	/** The device holds a renewal authority request: go on to signing it. */
	onPrepared(request: CertificateRequest): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const alive = useAlive();
	const actions = useDeviceAction();
	const authorities = useLocalSummary().authorities.length;
	const label = certificateLabel(row);
	const names = row.detail ? certificateNames(row.detail) : [];
	const [days, setDays] = useState(
		String(row.issuer?.leaf_lifetime_days ?? DEFAULT_LIFETIME_DAYS),
	);
	const [touched, setTouched] = useState(false);
	const [ran, setRan] = useState(false);
	const resultKey = requestsResultKey(certs.deviceId);
	const waiting = certs.requests.data?.find(
		(entry) => entry.certificate_id === row.id,
	);
	const prepared = waiting?.purpose === "issuer" ? waiting : undefined;
	const lifetime = Number(days);
	const daysError =
		!Number.isInteger(lifetime) || lifetime < 1 || lifetime > MAX_LIFETIME_DAYS;
	const view = gateView(
		t,
		time,
		certs.gate("renewal_delegation", { authorityPresent: authorities > 0 }),
	);
	const blocked = !!view || (!!waiting && !prepared) || !row.detail;

	const submit = async () => {
		if (prepared) {
			onPrepared(prepared);
			return;
		}
		setTouched(true);
		const certificate = row.detail;
		if (blocked || daysError || !certificate) return;
		setRan(true);
		const outcome = await actions.run({
			action: "renewal_delegation",
			deviceId: certs.deviceId,
			target: { extra: { authorityPresent: authorities > 0 } },
			label: t(
				"deviceCertificates.setup.label",
				"Prepare automatic renewal of {{label}}",
				{ label },
			),
			resultKey,
			activity: {
				kind: "signing_request",
				deviceName: certs.name,
				href: {
					screen: "device",
					deviceId: certs.deviceId,
					tab: "certificates",
				},
			},
			call: (context) =>
				withDeviceReason(context, (call) =>
					createCertificateIssuerRequest(call, {
						certificateId: row.id,
						expectedRevision: row.revision,
						dnsNames: certificate.dns_names,
						ipAddresses: certificate.ip_addresses,
						leafLifetimeDays: lifetime,
					}),
				),
		});
		if (outcome.status !== "done") return;
		await certs.reload();
		if (alive()) onPrepared(outcome.result);
	};

	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={FileKey}
			title={
				row.issuer
					? t(
							"deviceCertificates.setup.titleRepair",
							"Install a new renewal authority for {{label}}",
							{ label },
						)
					: t(
							"deviceCertificates.setup.title",
							"Set up automatic renewal of {{label}}",
							{ label },
						)
			}
			sub={certs.name}
			footNote={t(
				"deviceCertificates.setup.step",
				"Step 1 of 2 · the authority password is asked on the next step",
			)}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("deviceCertificates.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={actions.pending(resultKey)}
						aria-disabled={(blocked && !prepared) || undefined}
						onClick={() => void submit()}
					>
						{t("deviceCertificates.setup.continue", "Continue")}
					</DvButton>
				</>
			}
		>
			{view ? (
				<GateNotice kind={view.gate.kind} title={view.gate.reason} />
			) : prepared ? (
				<GateNotice
					kind="busy"
					title={t(
						"deviceCertificates.setup.prepared",
						"The device already holds a renewal authority request for {{label}}.",
						{ label },
					)}
					text={t(
						"deviceCertificates.setup.preparedText",
						"Continue to sign it with your organisation authority.",
					)}
				/>
			) : (
				<SheetBlocker view={null} {...(waiting ? { waitingFor: label } : {})} />
			)}
			<KeyValueList>
				<KvRow
					label={t("deviceCertificates.renewal.certificate", "Certificate")}
				>
					<b className="font-semibold">{label}</b>
				</KvRow>
				<KvRow label={t("deviceCertificates.renewal.names", "Approved names")}>
					<Names names={names} />
				</KvRow>
			</KeyValueList>
			{prepared ? null : (
				<Field
					id={`${id}-days`}
					label={t(
						"deviceCertificates.setup.lifetime",
						"Each renewed certificate is valid for",
					)}
					hint={t(
						"deviceCertificates.setup.lifetimeHint",
						"1 to {{max, number}} days. Shorter lifetimes limit what a copied certificate is worth.",
						{ max: MAX_LIFETIME_DAYS },
					)}
					error={
						touched && daysError
							? t(
									"deviceCertificates.sign.daysError",
									"Enter a whole number from 1 to {{max, number}}.",
									{ max: MAX_LIFETIME_DAYS },
								)
							: undefined
					}
				>
					<InputWithUnit
						className="max-w-40"
						numeric
						inputMode="numeric"
						value={days}
						unit={t("deviceCertificates.days", "days")}
						onChange={(event) => setDays(event.target.value)}
					/>
				</Field>
			)}
			<ConsequencePreview
				rows={{
					what:
						row.mode === "acme"
							? t(
									"deviceCertificates.setup.whatAcme",
									"The device makes a key for a renewal authority limited to exactly {{names}}. Once your organisation authority signs it, the device renews {{label}} with it and stops using Let's Encrypt.",
									{ names: listOf(time.locale, names), label },
								)
							: t(
									"deviceCertificates.setup.what",
									"The device makes a key for a renewal authority limited to exactly {{names}}. Once your organisation authority signs it, the device renews {{label}} by itself.",
									{ names: listOf(time.locale, names), label },
								),
					who: t(
						"deviceCertificates.setup.who",
						"Nobody notices. {{label}} keeps working while it renews.",
						{ label },
					),
					stays: t(
						"deviceCertificates.setup.stays",
						"The certificate ID, its names and the services that use it.",
					),
					when: t(
						"deviceCertificates.setup.when",
						"The request is made now. Renewal starts when the signed authority is installed.",
					),
					undo: {
						reversible: true,
						text: t(
							"deviceCertificates.setup.undo",
							"Stop renewal at any time. The certificate then expires on its date.",
						),
					},
				}}
			/>
			{ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}
