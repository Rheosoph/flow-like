"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Download,
	FileCheck,
	FileKey,
	Hourglass,
	OctagonX,
	ShieldCheck,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import { useId, useState } from "react";
import {
	type CertificateRequest,
	createCertificateRequest,
	deleteCertificateRequest,
	installCertificateIssuer,
	installCertificateRequest,
} from "../../../../../lib/device-management/certificate-issuance";
import type { ActionId } from "../../../../../lib/device-management/model/types";
import { enumExplain, enumLabel } from "../../copy/enum-labels";
import { useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { CellSub, DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import {
	CheckField,
	DropZone,
	DvTextarea,
	Field,
} from "../../primitives/form-fields";
import { GateNotice, GatedAction } from "../../primitives/gate-notice";
import { IdRef } from "../../primitives/id-ref";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StateView } from "../../primitives/state-view";
import { StatusChip } from "../../primitives/status-chip";
import { useDeviceAction, useInlineResults } from "../../workspace";
import {
	ActionResults,
	CHIP_WRAP,
	GatedButton,
	LabelField,
	LiveStamp,
	Names,
	SheetBlocker,
	TABLE_RESET,
	TickFirst,
	certificateResultKey,
	chainProblem,
	chainProblemText,
	downloadText,
	gateView,
	labelProblem,
	listOf,
	neverHere,
	useAlive,
	useDay,
	withDeviceReason,
} from "./certificate-parts";
import {
	type CertificateRow,
	type DeviceCertificates,
	MAX_CERTIFICATE_SLOTS,
	certificateLabel,
	certificateNames,
} from "./use-device-certificates";

const COLS = ["17%", "17%", "18%", "18%", "30%"] as const;
const MAX_NAMES = 32;
const IP_ADDRESS = /^(?:\d{1,3}(?:\.\d{1,3}){3}|[0-9a-f:]*:[0-9a-f:.]*)$/iu;

export const requestsResultKey = (deviceId: string) =>
	certificateResultKey(deviceId, "requests");

type RequestState = "pending" | "stale" | "expired";
type RequestContext = Pick<DeviceCertificates, "rows" | "detail" | "now">;

/** A request goes out of date when its certificate changed after the device made the key. */
function requestState(request: CertificateRequest, certs: RequestContext) {
	const state = (value: RequestState) => value;
	if (request.expires_at <= certs.now) return state("expired");
	if (!certs.detail.inventory) return state("pending");
	const current = certs.rows.find((row) => row.id === request.certificate_id);
	return state(
		(current?.revision ?? 0) === request.expected_revision
			? "pending"
			: "stale",
	);
}

/** One entry per line or comma; IP addresses are told apart from DNS names. */
export function splitNames(value: string) {
	const entries = [
		...new Set(
			value
				.split(/[\s,]+/u)
				.map((entry) => entry.trim())
				.filter(Boolean),
		),
	];
	return {
		dns: entries.filter((entry) => !IP_ADDRESS.test(entry)),
		ips: entries.filter((entry) => IP_ADDRESS.test(entry)),
	};
}

function installAction(request: CertificateRequest): ActionId {
	return request.purpose === "issuer" ? "renewal_delegation" : "csr_install";
}

function discardAction(request: CertificateRequest): ActionId {
	return request.purpose === "issuer" ? "renewal_delegation" : "csr_create";
}

const activityOf = (certs: DeviceCertificates) => ({
	kind: "signing_request" as const,
	deviceName: certs.name,
	href: {
		screen: "device" as const,
		deviceId: certs.deviceId,
		tab: "certificates" as const,
	},
});

function RequestStateChip({ state }: Readonly<{ state: RequestState }>) {
	const { t } = useTranslation("devices");
	if (state === "expired")
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{enumLabel(t, "csrState", "expired")}
			</StatusChip>
		);
	if (state === "stale")
		return (
			<>
				<StatusChip tone="warning" icon={TriangleAlert}>
					{enumLabel(t, "csrState", "stale")}
				</StatusChip>
				<CellSub>{enumExplain(t, "csrState", "stale")}</CellSub>
			</>
		);
	return (
		<StatusChip tone="info" icon={Hourglass} className={CHIP_WRAP}>
			{enumLabel(t, "csrState", "pending")}
		</StatusChip>
	);
}

function RequestRow({
	certs,
	request,
	onInstall,
	onSign,
	onDiscard,
}: Readonly<{
	certs: DeviceCertificates;
	request: CertificateRequest;
	onInstall(request: CertificateRequest): void;
	onSign(request: CertificateRequest): void;
	onDiscard(request: CertificateRequest): void;
}>) {
	const { t } = useTranslation("devices");
	const day = useDay();
	const state = requestState(request, certs);
	const target = certs.rows.find((row) => row.id === request.certificate_id);
	const install = certs.gate(installAction(request));
	const usable = state === "pending";
	const unusable = usable
		? undefined
		: state === "expired"
			? t(
					"deviceCertificates.requests.expiredReason",
					"The request expired. Discard it and create a new one.",
				)
			: t(
					"deviceCertificates.requests.staleReason",
					"The certificate changed after this request was made. Discard it and create a new one.",
				);
	const installLabel =
		request.purpose === "issuer"
			? t(
					"deviceCertificates.requests.installIssuer",
					"Install signed authority…",
				)
			: t("deviceCertificates.requests.install", "Install signed certificate…");
	return (
		<Tr data-request={request.request_id}>
			<Td
				label={t("deviceCertificates.requests.col.label", "Label")}
				kind="name"
			>
				<b className="font-semibold">{request.label}</b>
				<CellSub>
					<IdRef
						id={request.request_id}
						copyLabel={t(
							"deviceCertificates.requests.copyId",
							"Copy request ID",
						)}
					/>
				</CellSub>
			</Td>
			<Td label={t("deviceCertificates.requests.col.purpose", "Purpose")}>
				{enumLabel(t, "csrPurpose", request.purpose)}
				<CellSub>
					{request.expected_revision === 0
						? t("deviceCertificates.requests.new", "new certificate")
						: t(
								"deviceCertificates.requests.replaces",
								"replaces {{label}} (version {{revision, number}})",
								{
									label: target ? certificateLabel(target) : request.label,
									revision: request.expected_revision,
								},
							)}
					{" · "}
					{t(
						"deviceCertificates.requests.dates",
						"created {{created}} · expires {{expires}}",
						{
							created: day(request.created_at),
							expires: day(request.expires_at),
						},
					)}
				</CellSub>
			</Td>
			<Td label={t("deviceCertificates.requests.col.names", "Names")}>
				<Names names={certificateNames(request)} />
			</Td>
			<Td label={t("deviceCertificates.requests.col.state", "State")}>
				<RequestStateChip state={state} />
			</Td>
			<Td
				label={t("deviceCertificates.requests.col.actions", "Actions")}
				kind="act"
			>
				<div className="flex flex-wrap items-start gap-1.5">
					<DvButton
						size="sm"
						icon={Download}
						onClick={() =>
							downloadText(
								`${request.request_id}.csr`,
								request.csr_pem,
								"application/pkcs10",
							)
						}
					>
						{t(
							"deviceCertificates.requests.download",
							"Download signing request",
						)}
					</DvButton>
					{usable ? (
						<GatedButton
							result={install}
							size="sm"
							icon={FileCheck}
							onClick={() => onInstall(request)}
						>
							{installLabel}
						</GatedButton>
					) : neverHere(install) ? null : (
						<GatedAction gate={{ kind: "busy", reason: unusable }}>
							<DvButton size="sm" icon={FileCheck}>
								{installLabel}
							</DvButton>
						</GatedAction>
					)}
					{usable ? (
						<GatedButton
							result={install}
							variant="ghost"
							size="sm"
							icon={ShieldCheck}
							onClick={() => onSign(request)}
						>
							{t(
								"deviceCertificates.requests.sign",
								"Sign with organisation authority…",
							)}
						</GatedButton>
					) : null}
					<GatedButton
						result={certs.gate(discardAction(request))}
						variant="danger-ghost"
						size="sm"
						onClick={() => onDiscard(request)}
					>
						{t("deviceCertificates.requests.discard", "Discard request…")}
					</GatedButton>
				</div>
			</Td>
		</Tr>
	);
}

function useDiscardRequest(certs: DeviceCertificates) {
	const { t } = useTranslation("devices");
	const actions = useDeviceAction();
	return async (request: CertificateRequest) => {
		const { label } = request;
		const outcome = await actions.run({
			action: discardAction(request),
			deviceId: certs.deviceId,
			label: t(
				"deviceCertificates.requests.discardLabel",
				"Discard the signing request for {{label}}",
				{ label },
			),
			consequence: {
				what: t(
					"deviceCertificates.requests.discardWhat",
					"The device destroys the key it made for this request.",
				),
				who: t(
					"deviceCertificates.requests.discardWho",
					"Nobody notices. A certificate signed from this request elsewhere becomes useless.",
				),
				when: t("deviceCertificates.requests.discardWhen", "Immediately."),
				undo: {
					reversible: false,
					text: t(
						"deviceCertificates.requests.discardUndo",
						"Create a new signing request.",
					),
				},
			},
			strength: "none",
			confirm: {
				title: t(
					"deviceCertificates.requests.discardTitle",
					"Discard the signing request for {{label}}?",
					{ label },
				),
				sub: certs.name,
				tone: "danger",
				icon: Trash2,
			},
			resultKey: requestsResultKey(certs.deviceId),
			activity: activityOf(certs),
			call: (context) =>
				withDeviceReason(context, (call) =>
					deleteCertificateRequest(call, request),
				),
		});
		if (outcome.status === "done") await certs.reload();
	};
}

/** S33: the requests whose keys wait on the device for a signed certificate. */
export function SigningRequests({
	certs,
	onInstall,
	onSign,
}: Readonly<{
	certs: DeviceCertificates;
	onInstall(request: CertificateRequest): void;
	onSign(request: CertificateRequest): void;
}>) {
	const { t } = useTranslation("devices");
	const { requests } = certs;
	const discard = useDiscardRequest(certs);
	const results = useInlineResults(requestsResultKey(certs.deviceId));
	const rows = requests.data;
	// Nothing waiting and nothing to report: the list's "Add certificate" starts a request.
	if (rows && !rows.length && !results.length) return null;
	return (
		<Block
			id="device-certificate-requests"
			icon={FileKey}
			title={t("deviceCertificates.requests.title", "Pending signing requests")}
			{...(rows ? { count: rows.length } : {})}
			stamp={<LiveStamp freshness={requests.freshness} />}
			flush={!!rows?.length}
		>
			{!rows ? (
				requests.failed ? (
					<StateView
						kind="error"
						title={t(
							"deviceCertificates.requests.error",
							"Couldn't read the signing requests from {{device}}",
							{ device: certs.name },
						)}
						actions={
							<DvButton size="sm" onClick={() => void requests.refetch()}>
								{t("deviceCertificates.error.retry", "Retry now")}
							</DvButton>
						}
					/>
				) : (
					<StateView
						kind="loading"
						rows={1}
						title={t(
							"deviceCertificates.requests.loading",
							"Reading signing requests…",
						)}
					/>
				)
			) : rows.length ? (
				<DvTable
					cols={COLS}
					className={TABLE_RESET}
					label={t(
						"deviceCertificates.requests.label",
						"Pending signing requests on {{device}}",
						{ device: certs.name },
					)}
					head={
						<tr>
							<Th>{t("deviceCertificates.requests.col.label", "Label")}</Th>
							<Th>{t("deviceCertificates.requests.col.purpose", "Purpose")}</Th>
							<Th>{t("deviceCertificates.requests.col.names", "Names")}</Th>
							<Th>{t("deviceCertificates.requests.col.state", "State")}</Th>
							<Th>{t("deviceCertificates.requests.col.actions", "Actions")}</Th>
						</tr>
					}
				>
					{rows.map((request) => (
						<RequestRow
							key={request.request_id}
							certs={certs}
							request={request}
							onInstall={onInstall}
							onSign={onSign}
							onDiscard={(target) => void discard(target)}
						/>
					))}
				</DvTable>
			) : (
				<p className="max-w-[80ch] text-ui text-muted-foreground">
					{t(
						"deviceCertificates.requests.none",
						"No signing request is waiting. With a request the device makes its own key; you get the request signed and install the certificate, so the key never leaves the device.",
					)}
				</p>
			)}
			<ActionResults
				scopeKey={requestsResultKey(certs.deviceId)}
				className={rows?.length ? "px-4 py-3" : undefined}
			/>
		</Block>
	);
}

/* Create a signing request (new certificate, or a replacement for one). */

export function CreateRequestSheet({
	certs,
	row,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	row?: CertificateRow;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const alive = useAlive();
	const actions = useDeviceAction();
	const current = row ? certificateLabel(row) : "";
	const [label, setLabel] = useState(current);
	const [names, setNames] = useState(
		row?.detail ? certificateNames(row.detail).join("\n") : "",
	);
	const [touched, setTouched] = useState(false);
	const [ran, setRan] = useState(false);
	const resultKey = requestsResultKey(certs.deviceId);
	const { dns, ips } = splitNames(names);
	const total = dns.length + ips.length;
	const waiting = row
		? certs.requests.data?.find((entry) => entry.certificate_id === row.id)
		: undefined;
	const full = !row && (certs.slots?.used ?? 0) >= MAX_CERTIFICATE_SLOTS;
	const labelError = labelProblem(label);
	const namesError =
		total === 0 ? "empty" : total > MAX_NAMES ? "too_many" : null;
	const gate = certs.gate("csr_create");
	const view = gateView(t, time, gate);
	const automatic = row?.mode === "acme" || row?.mode === "delegated";
	const users = row?.uses ?? 0;
	const blocked = !!waiting || full || !!view;
	const submit = async () => {
		setTouched(true);
		if (labelError || namesError || blocked) return;
		setRan(true);
		const outcome = await actions.run({
			action: "csr_create",
			deviceId: certs.deviceId,
			label: t(
				"deviceCertificates.request.label",
				"Create a signing request for {{label}}",
				{ label: label.trim() },
			),
			resultKey,
			activity: activityOf(certs),
			call: (context) =>
				withDeviceReason(context, (call) =>
					createCertificateRequest(call, {
						certificateId: row?.id ?? crypto.randomUUID(),
						label,
						expectedRevision: row?.detail ? row.revision : 0,
						dnsNames: dns,
						ipAddresses: ips,
					}),
				),
		});
		if (outcome.status !== "done") return;
		await certs.reload();
		if (alive()) onClose();
	};
	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={FileKey}
			title={
				row
					? t(
							"deviceCertificates.request.titleFor",
							"Create a signing request for {{label}}",
							{ label: current },
						)
					: t("deviceCertificates.request.title", "Create a signing request")
			}
			sub={certs.name}
			footNote={
				row
					? undefined
					: t(
							"deviceCertificates.request.slot",
							"Uses 1 certificate slot: {{used, number}} of {{max, number}} after this.",
							{
								used: (certs.slots?.used ?? 0) + 1,
								max: MAX_CERTIFICATE_SLOTS,
							},
						)
			}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("deviceCertificates.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={actions.pending(resultKey)}
						aria-disabled={blocked || undefined}
						onClick={() => void submit()}
					>
						{t("deviceCertificates.request.submit", "Create signing request")}
					</DvButton>
				</>
			}
		>
			<SheetBlocker
				view={view}
				{...(waiting ? { waitingFor: current } : {})}
				full={full}
			/>
			<LabelField
				id={`${id}-label`}
				value={label}
				onChange={setLabel}
				showProblem={touched}
			/>
			<Field
				id={`${id}-names`}
				label={t("deviceCertificates.request.names", "Names")}
				hint={t(
					"deviceCertificates.request.namesHint",
					"One per line: DNS names and IP addresses. The signed certificate must carry exactly these.",
				)}
				error={
					touched && namesError
						? namesError === "empty"
							? t(
									"deviceCertificates.request.namesEmpty",
									"Add at least one DNS name or IP address.",
								)
							: t(
									"deviceCertificates.request.namesTooMany",
									"A request carries at most {{max, number}} names.",
									{ max: MAX_NAMES },
								)
						: undefined
				}
			>
				<DvTextarea
					className="font-mono"
					rows={3}
					spellCheck={false}
					value={names}
					onChange={(event) => setNames(event.target.value)}
				/>
			</Field>
			<ConsequencePreview
				rows={{
					what: t(
						"deviceCertificates.request.what",
						"The device creates a new private key and a signing request for these names. The key never leaves the device.",
					),
					who: t(
						"deviceCertificates.request.who",
						"Nobody: nothing changes for services until you install the signed certificate.",
					),
					...(row
						? {
								stays: automatic
									? t(
											"deviceCertificates.request.staysAutomatic",
											"{{label}} keeps working and renewing as it is. Installing the signed certificate stops its automatic renewal.",
											{ label: current },
										)
									: users
										? t(
												"deviceCertificates.request.staysUsed",
												"{{label}} keeps working as it is, and so do the services that use it.",
												{ label: current },
											)
										: t(
												"deviceCertificates.request.stays",
												"{{label}} keeps working as it is.",
												{ label: current },
											),
							}
						: {}),
					when: t(
						"deviceCertificates.request.when",
						"Now. The request expires in 30 days.",
					),
					undo: {
						reversible: true,
						text: t(
							"deviceCertificates.request.undo",
							"Discard the request. The device destroys its key.",
						),
					},
				}}
			/>
			{ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}

/* Install the signed chain of a request. */

export function InstallChainSheet({
	certs,
	request,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	request: CertificateRequest;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const alive = useAlive();
	const actions = useDeviceAction();
	const [chain, setChain] = useState("");
	const [fileName, setFileName] = useState<string>();
	const [unreadable, setUnreadable] = useState(false);
	const [acknowledged, setAcknowledged] = useState(false);
	const [touched, setTouched] = useState(false);
	const [ran, setRan] = useState(false);
	const issuer = request.purpose === "issuer";
	const resultKey = requestsResultKey(certs.deviceId);
	const names = certificateNames(request);
	const target = certs.rows.find((row) => row.id === request.certificate_id);
	const automatic =
		!issuer && (target?.mode === "acme" || target?.mode === "delegated");
	const needsCheck = issuer || automatic;
	const problem = chainProblem(chain);
	const gate = certs.gate(installAction(request));
	const view = gateView(t, time, gate);
	const users = target?.uses ?? 0;
	const pick = async (files: File[]) => {
		const file = files[0];
		if (!file) return;
		setUnreadable(false);
		try {
			const text = await file.text();
			if (!alive()) return;
			setChain(text);
			setFileName(file.name);
		} catch {
			if (alive()) setUnreadable(true);
		}
	};
	const submit = async () => {
		setTouched(true);
		if (problem || view || (needsCheck && !acknowledged)) return;
		setRan(true);
		const outcome = await actions.run({
			action: installAction(request),
			deviceId: certs.deviceId,
			label: issuer
				? t(
						"deviceCertificates.install.labelIssuer",
						"Install the renewal authority for {{label}}",
						{ label: request.label },
					)
				: t(
						"deviceCertificates.install.label",
						"Install the signed certificate for {{label}}",
						{ label: request.label },
					),
			resultKey,
			activity: activityOf(certs),
			call: (context) =>
				withDeviceReason(context, async (call) => {
					if (issuer) await installCertificateIssuer(call, request, chain);
					else await installCertificateRequest(call, request, chain);
				}),
		});
		if (outcome.status !== "done") return;
		await certs.reload();
		if (alive()) onClose();
	};
	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={FileCheck}
			title={
				issuer
					? t(
							"deviceCertificates.install.titleIssuer",
							"Install the signed renewal authority for {{label}}",
							{ label: request.label },
						)
					: t(
							"deviceCertificates.install.title",
							"Install the signed certificate for {{label}}",
							{ label: request.label },
						)
			}
			sub={certs.name}
			footNote={!view && needsCheck && !acknowledged ? <TickFirst /> : undefined}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("deviceCertificates.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={actions.pending(resultKey)}
						aria-disabled={!!view || (needsCheck && !acknowledged) || undefined}
						onClick={() => void submit()}
					>
						{issuer
							? t(
									"deviceCertificates.install.submitIssuer",
									"Install renewal authority",
								)
							: t("deviceCertificates.install.submit", "Install certificate")}
					</DvButton>
				</>
			}
		>
			{view ? (
				<GateNotice kind={view.gate.kind} title={view.gate.reason} />
			) : null}
			<KeyValueList>
				<KvRow label={t("deviceCertificates.install.request", "Request")}>
					<b className="font-semibold">{request.label}</b>
					{" · "}
					{enumLabel(t, "csrPurpose", request.purpose)}
				</KvRow>
				<KvRow label={t("deviceCertificates.install.names", "Exact names")}>
					<Names names={names} />
				</KvRow>
			</KeyValueList>
			<DropZone
				id={`${id}-file`}
				accept=".pem,.crt,.cer"
				title={t(
					"deviceCertificates.install.drop",
					"Drop the signed chain here or choose a file",
				)}
				hint={t(
					"deviceCertificates.install.dropHint",
					"PEM, up to 12 KiB. Certificates only: the device keeps its own key.",
				)}
				files={
					fileName ? (
						<span className="font-mono text-xs text-ink-2">{fileName}</span>
					) : undefined
				}
				onFiles={(files) => void pick(files)}
			/>
			<Field
				id={`${id}-chain`}
				label={t("deviceCertificates.install.chain", "Signed chain (PEM)")}
				hint={t(
					"deviceCertificates.install.chainHint",
					"Paste the chain your authority sent back, the signed certificate first. The device checks it against the key it made and the exact names above.",
				)}
				error={
					unreadable
						? t(
								"deviceCertificates.install.unreadable",
								"That file couldn't be read.",
							)
						: touched && problem
							? chainProblemText(t, problem)
							: undefined
				}
			>
				<DvTextarea
					className="font-mono"
					rows={5}
					spellCheck={false}
					placeholder="-----BEGIN CERTIFICATE-----"
					value={chain}
					onChange={(event) => {
						setChain(event.target.value);
						setFileName(undefined);
					}}
				/>
			</Field>
			<ConsequencePreview
				rows={
					issuer
						? {
								what: t(
									"deviceCertificates.install.issuerWhat",
									"The device keeps the signed renewal authority and renews {{label}} by itself for exactly {{names}}.",
									{ label: request.label, names: listOf(time.locale, names) },
								),
								who: t(
									"deviceCertificates.install.issuerWho",
									"Nobody notices. {{label}} keeps working while it renews.",
									{ label: request.label },
								),
								when: t(
									"deviceCertificates.install.issuerWhen",
									"Immediately after the check. Renewal works even while the hub is unreachable, until the authority expires.",
								),
								undo: {
									reversible: true,
									text: t(
										"deviceCertificates.install.issuerUndo",
										"Stop renewal at any time. The certificate then expires on its date.",
									),
								},
							}
						: {
								what: target
									? t(
											"deviceCertificates.install.replaceWhat",
											"The device swaps {{label}} for the signed certificate under the same ID.",
											{ label: request.label },
										)
									: t(
											"deviceCertificates.install.what",
											"The device installs it as the new certificate {{label}} for {{names}}.",
											{
												label: request.label,
												names: listOf(time.locale, names),
											},
										),
								who: users
									? t(
											"deviceCertificates.install.whoUsed",
											"The services that use {{label}} pick up the new certificate on their next connection. No restart.",
											{ label: request.label },
										)
									: t(
											"deviceCertificates.install.who",
											"Nobody: nothing uses it until you pick it for a service's web endpoint.",
										),
								when: automatic
									? t(
											"deviceCertificates.install.whenAutomatic",
											"Immediately after the check. Automatic renewal of {{label}} stops.",
											{ label: request.label },
										)
									: t(
											"deviceCertificates.install.when",
											"Immediately after the check.",
										),
								undo: target
									? {
											reversible: false,
											text: t(
												"deviceCertificates.install.replaceUndo",
												"Replace it again. The request is used up.",
											),
										}
									: {
											reversible: true,
											text: t(
												"deviceCertificates.install.undo",
												"Delete the certificate. The request is used up.",
											),
										},
							}
				}
			/>
			{needsCheck ? (
				<CheckField
					id={`${id}-ack`}
					checked={acknowledged}
					onCheckedChange={setAcknowledged}
				>
					{issuer
						? t(
								"deviceCertificates.install.approve",
								"I authorise this device to issue certificates for exactly the names above, valid for at most {{count, number}} days each, until the signed authority expires.",
								{ count: request.leaf_lifetime_days ?? 0 },
							)
						: t(
								"deviceCertificates.install.stopsRenewal",
								"Automatic renewal of {{label}} stops. I'll renew it myself.",
								{ label: request.label },
							)}
				</CheckField>
			) : null}
			{ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}
