"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Check,
	Copy,
	ExternalLink,
	FileBadge,
	Globe,
	KeyRound,
	LoaderCircle,
	Lock,
	QrCode,
	TriangleAlert,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import type { PlacementConfiguration } from "../../../../lib/device-management/deployment";
import type { NetworkInterface } from "../../../../lib/device-management/model/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { ExpiryRail } from "../primitives/expiry-rail";
import { DvInput, Field } from "../primitives/form-fields";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { useCopy } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import type { ServiceTabProps } from "../screen-props";
import { useAppView, useAttentionState } from "../workspace";
import { exposureText } from "./config-diff";
import {
	type HostingView,
	type PlacementConfig,
	hostingOf,
	servicePageAddress,
} from "./config-model";
import {
	ActionResults,
	ConfigStamp,
	ConfigUnavailable,
	LINK,
	Mono,
	type Note,
	gateLine,
} from "./config-parts";
import { NewTokenSheet } from "./secret-fields";
import {
	EditSettingsSheet,
	type SettingsEditor,
	useSettingsEditor,
} from "./settings-sheets";
import {
	type SecretWrite,
	type ServiceConfigRead,
	useSecretWrite,
	useServiceConfig,
} from "./use-service-config";

/* BG20: with a wildcard address the device can't tell which name people use; the owner types it and it stays on this computer. */

const addressKey = (scopeKey: string, deviceId: string, serviceId: string) =>
	`flow-like:devices:service-address:${scopeKey}:${deviceId}:${serviceId}`;

/** Browser storage may be absent or blocked (private windows): the address then lives for this page only. */
function addressStore() {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

function readAddress(key: string) {
	try {
		return addressStore()?.getItem(key) ?? "";
	} catch {
		return "";
	}
}

function writeAddress(key: string, value: string) {
	const store = addressStore();
	if (!store) return false;
	try {
		if (value) store.setItem(key, value);
		else store.removeItem(key);
		return true;
	} catch {
		return false;
	}
}

/** A host name or IP address, without scheme, port or path. */
const ADDRESS =
	/^(\[[0-9a-fA-F:]+\]|[A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?)$/;

function useSavedAddress(deviceId: string, serviceId: string) {
	const { workspace } = useAttentionState();
	const key = addressKey(workspace.scopeKey, deviceId, serviceId);
	const [saved, setSaved] = useState(() => ({ key, value: readAddress(key) }));
	const value = saved.key === key ? saved.value : readAddress(key);
	const save = useCallback(
		(next: string) => {
			const stored = writeAddress(key, next);
			setSaved({ key, value: next });
			return stored;
		},
		[key],
	);
	return { address: value, save };
}

const routable = (address: string) => !address.startsWith("fe80:");

function lanAddresses(interfaces: readonly NetworkInterface[] | undefined) {
	const addresses: string[] = [];
	for (const entry of interfaces ?? [])
		if (!entry.loopback) addresses.push(...entry.addresses.filter(routable));
	return addresses.slice(0, 6);
}

interface EndpointFacts {
	hosting: HostingView;
	tls: boolean;
	certificate?: { id: string; label: string; notAfter?: number };
	url: string | undefined;
}

/** A device's certificates as this account sees them: the device's own list and the hub's public one. */
function useCertificateLists(deviceId: string) {
	const { input } = useAttentionState();
	const live = input.live[deviceId]?.certificates?.certificates;
	const listed = input.certInventory[deviceId]?.certificates;
	return useMemo(
		() => ({ live: live ?? [], listed: listed ?? [] }),
		[live, listed],
	);
}

type CertificateLists = ReturnType<typeof useCertificateLists>;

/** The certificate the settings name: its label from the device, its expiry from the device or the hub. */
function namedCertificate(
	id: string,
	lists: CertificateLists,
	unnamed: string,
) {
	const same = (entry: { certificate_id: string }) =>
		entry.certificate_id === id;
	const known = lists.live.find(same);
	const dated = known ?? lists.listed.find(same);
	const named = { id, label: known ? known.label : unnamed };
	return dated ? { ...named, notAfter: dated.not_after } : named;
}

function endpointFacts(
	config: PlacementConfig,
	address: string,
	certificate: (id: string) => NonNullable<EndpointFacts["certificate"]>,
) {
	const hosting = hostingOf(config);
	if (!hosting) return null;
	const id = config.tls_certificate_id ?? undefined;
	const facts: EndpointFacts = {
		hosting,
		tls: !!id,
		url: servicePageAddress(hosting, !!id, address),
	};
	if (id) facts.certificate = certificate(id);
	return facts;
}

function useEndpoint(
	deviceId: string,
	configuration: PlacementConfiguration | undefined,
	address: string,
) {
	const { t } = useTranslation("devices");
	const lists = useCertificateLists(deviceId);
	return useMemo(() => {
		if (!configuration) return null;
		return endpointFacts(configuration.config, address, (id) =>
			namedCertificate(
				id,
				lists,
				t("serviceConfig.cert.unnamed", "Certificate {{id}}", {
					id: id.slice(0, 8),
				}),
			),
		);
	}, [t, configuration, lists, address]);
}

/* The link. */

function QrSheet({
	url,
	loopback,
	device,
	open,
	onOpenChange,
}: Readonly<{
	url: string;
	loopback: boolean;
	device: string;
	open: boolean;
	onOpenChange(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			icon={QrCode}
			title={t(
				"serviceConfig.endpoint.qrTitle",
				"QR code for the service page",
			)}
			sub={url}
			foot={
				<>
					<DvButton icon={copied ? Check : Copy} onClick={() => void copy(url)}>
						{copied
							? t("serviceConfig.endpoint.copied", "Copied")
							: t("serviceConfig.endpoint.copy", "Copy link")}
					</DvButton>
					<DvButton onClick={() => onOpenChange(false)}>
						{t("serviceConfig.sheet.close", "Close")}
					</DvButton>
				</>
			}
		>
			<div className="flex flex-wrap items-start gap-4">
				{/* A code scans only dark on light, in both themes. */}
				<div className="rounded-lg border border-border bg-white p-3 text-black">
					<QRCodeSVG
						value={url}
						size={176}
						bgColor="transparent"
						fgColor="currentColor"
						role="img"
						aria-label={t(
							"serviceConfig.endpoint.qrLabel",
							"QR code for {{url}}",
							{ url },
						)}
					/>
				</div>
				<div className="flex min-w-[18ch] flex-1 flex-col gap-1.5 text-ui">
					<p className="text-ui">
						{t(
							"serviceConfig.endpoint.qrScan",
							"Scan to open the page on a phone.",
						)}
					</p>
					<p className="text-xs text-muted-foreground">
						{loopback
							? t(
									"serviceConfig.endpoint.qrLoopback",
									"This address only works on {{device}} itself, so a phone can't open it.",
									{ device },
								)
							: t(
									"serviceConfig.endpoint.qrNetwork",
									"The phone must be on a network that can reach {{device}}.",
									{ device },
								)}
					</p>
				</div>
			</div>
		</DvSheet>
	);
}

function LinkBlock({
	facts,
	device,
}: Readonly<{ facts: EndpointFacts; device: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const [qr, setQr] = useState(false);
	const { url, hosting } = facts;
	const loopback = hosting.exposure === "loopback";
	const missing: Gate | null = url
		? null
		: {
				kind: "unsupported",
				reason: t(
					"serviceConfig.endpoint.noAddress",
					"Enter the address people use below to get a link.",
				),
			};
	return (
		<div className="flex min-w-0 flex-col gap-2.5">
			<div
				data-endpoint-url=""
				title={url}
				className="max-w-full self-start truncate rounded-lg border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-ui"
			>
				{url ??
					t(
						"serviceConfig.endpoint.urlUnknown",
						"{{scheme}}://…:{{port}}/ui/",
						{
							scheme: facts.tls ? "https" : "http",
							port: hosting.port,
						},
					)}
			</div>
			<div className="flex flex-wrap items-start gap-2">
				<GatedAction gate={missing}>
					<DvButton
						size="sm"
						icon={copied ? Check : Copy}
						data-act="endpoint-copy"
						onClick={() => void copy(url ?? "")}
					>
						{copied
							? t("serviceConfig.endpoint.copied", "Copied")
							: t("serviceConfig.endpoint.copy", "Copy link")}
					</DvButton>
				</GatedAction>
				{url ? (
					<>
						<DvButton asChild size="sm" icon={ExternalLink}>
							<a
								href={url}
								target="_blank"
								rel="noopener noreferrer"
								data-act="endpoint-open"
							>
								{t("serviceConfig.endpoint.open", "Open")}
							</a>
						</DvButton>
						<DvButton
							size="sm"
							icon={QrCode}
							data-act="endpoint-qr"
							onClick={() => setQr(true)}
						>
							{t("serviceConfig.endpoint.qr", "QR code")}
						</DvButton>
						<QrSheet
							url={url}
							loopback={loopback}
							device={device}
							open={qr}
							onOpenChange={setQr}
						/>
					</>
				) : null}
			</div>
			{loopback ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceConfig.endpoint.loopbackHint",
						"Opens in this computer's browser. The address works only on {{device}} itself.",
						{ device },
					)}
				</p>
			) : null}
		</div>
	);
}

/* Rows. */

function EncryptionRow({
	facts,
	deviceId,
}: Readonly<{ facts: EndpointFacts; deviceId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { certificate, hosting } = facts;
	if (certificate) {
		const components = {
			1: (
				<a
					className={`${LINK} font-semibold`}
					{...link({
						screen: "device",
						deviceId,
						tab: "certificates",
						certificateId: certificate.id,
					})}
				>
					{certificate.label}
				</a>
			),
		};
		return (
			<span className="flex flex-wrap items-center gap-x-2 gap-y-1">
				<StatusChip tone="good" icon={Lock}>
					{t("serviceConfig.endpoint.https", "HTTPS")}
				</StatusChip>
				<span>
					{certificate.notAfter === undefined ? (
						<Trans
							t={t}
							i18nKey="serviceConfig.endpoint.certificate"
							defaults="certificate <1/>"
							components={components}
						/>
					) : (
						<Trans
							t={t}
							i18nKey="serviceConfig.endpoint.certificateExpires"
							defaults="certificate <1/> · expires {{date}}"
							values={{ date: time.at(certificate.notAfter) }}
							components={components}
						/>
					)}
				</span>
				{certificate.notAfter === undefined ? null : (
					<ExpiryRail notAfter={certificate.notAfter} />
				)}
			</span>
		);
	}
	if (hosting.exposure === "loopback")
		return (
			<>
				{t(
					"serviceConfig.endpoint.plainLoopback",
					"Unencrypted (HTTP) · only this device can reach it",
				)}
			</>
		);
	return (
		<span
			data-exposure-warning=""
			className="flex flex-wrap items-center gap-x-2 gap-y-1"
		>
			<StatusChip tone="warning" icon={TriangleAlert}>
				{t("serviceConfig.endpoint.plain", "Unencrypted (HTTP)")}
			</StatusChip>
			{t(
				"serviceConfig.endpoint.plainExposed",
				"reachable from the network without encryption",
			)}
		</span>
	);
}

function AddressRow({
	saved,
	onSave,
	suggestions,
	supported,
}: Readonly<{
	saved: string;
	onSave(value: string): boolean;
	/** Addresses the device reports for its network interfaces. */
	suggestions: readonly string[];
	/** The agent reports its interfaces (BG20). */
	supported: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [value, setValue] = useState(saved);
	const [error, setError] = useState<string | null>(null);
	const [result, setResult] = useState<Note | null>(null);
	const save = (next: string) => {
		const address = next.trim();
		if (address && !ADDRESS.test(address)) {
			setError(
				t(
					"serviceConfig.endpoint.addressInvalid",
					"Enter a host name or an IP address, without http://, a port or a path.",
				),
			);
			return;
		}
		setError(null);
		setValue(address);
		const stored = onSave(address);
		setResult(
			!stored
				? {
						tone: "warning",
						text: t(
							"serviceConfig.endpoint.addressNotStored",
							"This browser doesn't keep data, so the address lasts only until you leave this page.",
						),
					}
				: address
					? {
							tone: "good",
							text: t(
								"serviceConfig.endpoint.addressSaved",
								"Saved on this computer at {{time}}. Links and the QR code use {{address}}.",
								{ time: time.clock(time.nowS), address },
							),
						}
					: {
							tone: "good",
							text: t(
								"serviceConfig.endpoint.addressCleared",
								"Cleared. There is no link until you enter an address.",
							),
						},
		);
	};
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<Field
				id="svc-endpoint-address"
				label={
					<span className="sr-only">
						{t("serviceConfig.endpoint.address", "Address people use")}
					</span>
				}
				error={error ?? undefined}
				hint={t(
					"serviceConfig.endpoint.addressHint",
					"The device listens on all networks, so it can't tell which name people use. Saved on this computer only.",
				)}
				className="max-w-115"
			>
				<div className="flex flex-wrap items-center gap-2">
					<DvInput
						id="svc-endpoint-address"
						mono
						value={value}
						placeholder={t(
							"serviceConfig.endpoint.addressPlaceholder",
							"edge.example.com",
						)}
						autoComplete="off"
						spellCheck={false}
						className="min-w-[18ch] flex-1"
						onChange={(event) => {
							setValue(event.target.value);
							setError(null);
						}}
					/>
					<DvButton
						size="sm"
						data-act="endpoint-address-save"
						onClick={() => save(value)}
					>
						{t("serviceConfig.endpoint.addressSave", "Save")}
					</DvButton>
				</div>
			</Field>
			{suggestions.length ? (
				<div
					data-lan-addresses=""
					className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground"
				>
					{t("serviceConfig.endpoint.lan", "The device reports:")}
					{suggestions.map((address) => (
						<DvButton
							key={address}
							size="xs"
							variant="ghost"
							className="font-mono"
							onClick={() =>
								save(address.includes(":") ? `[${address}]` : address)
							}
						>
							{address}
						</DvButton>
					))}
				</div>
			) : supported ? null : (
				<p
					data-lan-addresses="unsupported"
					className="text-xs text-muted-foreground"
				>
					{t(
						"serviceConfig.endpoint.lanUnsupported",
						"Update the device agent to see the addresses the device has on its networks.",
					)}
				</p>
			)}
			{result ? (
				<InlineResult tone={result.tone} onDismiss={() => setResult(null)}>
					{result.text}
				</InlineResult>
			) : null}
		</div>
	);
}

function TokenRow({
	editor,
	writer,
	onOpen,
}: Readonly<{
	editor: SettingsEditor;
	writer: SecretWrite;
	onOpen(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { gates } = editor;
	const gate: Gate | null =
		gateLine(t, time, gates.secret) ??
		(gates.rollout
			? { kind: "busy", reason: rolloutReason(t, time, gates) }
			: null);
	return (
		<div className="flex min-w-0 flex-col items-start gap-1.5">
			<span data-token-state="">
				{gates.pendingSecret || writer.pending ? (
					<StatusChip tone="info" icon={LoaderCircle} spin>
						{t("serviceConfig.endpoint.tokenWaiting", "Waiting for device")}
					</StatusChip>
				) : (
					t("serviceConfig.endpoint.tokenSet", "Set")
				)}
			</span>
			<GatedAction gate={gate}>
				<DvButton
					size="sm"
					icon={KeyRound}
					data-act="endpoint-token"
					onClick={onOpen}
				>
					{t("serviceConfig.endpoint.tokenNew", "Set a new token…")}
				</DvButton>
			</GatedAction>
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.endpoint.tokenHint",
					"Values are never readable. People with the service page need the token.",
				)}
			</p>
		</div>
	);
}

/** "An update is in progress…" / "An update is staged…": why settings can't change now. */
export function rolloutReason(
	t: DevicesT,
	time: ReturnType<typeof useAreaTime>,
	gates: SettingsEditor["gates"],
): string {
	if (gates.rollout?.staged)
		return t(
			"devices:serviceConfig.gate.staged",
			"An update is staged. Activate or discard it on Status first.",
		);
	const by = gates.rollout?.deadlineAt;
	return by
		? t(
				"devices:serviceConfig.gate.updatingBy",
				"An update is in progress. This works again after it finishes (by {{time}} at the latest).",
				{ time: time.clock(by) },
			)
		: t(
				"devices:serviceConfig.gate.updating",
				"An update is in progress. This works again after it finishes.",
			);
}

function CertificateRow({
	facts,
	editor,
	onOpen,
}: Readonly<{ facts: EndpointFacts; editor: SettingsEditor; onOpen(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { gates } = editor;
	if (facts.hosting.exposure === "loopback" && !facts.tls)
		return (
			<span className="text-muted-foreground">
				{t(
					"serviceConfig.endpoint.certNotNeeded",
					"Not needed: only this device can reach it",
				)}
			</span>
		);
	const gate: Gate | null =
		gateLine(t, time, gates.gates.change_tls) ??
		(gates.rollout
			? { kind: "busy", reason: rolloutReason(t, time, gates) }
			: editor.editGate
				? { kind: editor.editGate.kind, reason: editor.editGate.text }
				: null);
	return (
		<div className="flex min-w-0 flex-col items-start gap-1.5">
			<GatedAction gate={gate}>
				<DvButton
					size="sm"
					icon={FileBadge}
					data-act="endpoint-cert"
					onClick={onOpen}
				>
					{facts.tls
						? t("serviceConfig.endpoint.certChange", "Change certificate…")
						: t("serviceConfig.endpoint.certAssign", "Assign certificate…")}
				</DvButton>
			</GatedAction>
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.endpoint.certHint",
					"Changing the certificate goes through Edit settings and a safe update.",
				)}
			</p>
		</div>
	);
}

function ExposureRow({ hosting }: Readonly<{ hosting: HostingView }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			{exposureText(t, hosting.exposure)}{" "}
			<Mono className="text-muted-foreground">({hosting.host})</Mono>
		</>
	);
}

function Rows({
	deviceId,
	facts,
	editor,
	writer,
	saved,
	onOpenToken,
	onOpenCertificate,
}: Readonly<{
	deviceId: string;
	facts: EndpointFacts;
	editor: SettingsEditor;
	writer: SecretWrite;
	saved: ReturnType<typeof useSavedAddress>;
	onOpenToken(): void;
	onOpenCertificate(): void;
}>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const inspection = input.live[deviceId]?.inspection?.value;
	const { hosting } = facts;
	return (
		<KeyValueList>
			<KvRow label={t("serviceConfig.endpoint.encryption", "Encryption")}>
				<EncryptionRow facts={facts} deviceId={deviceId} />
			</KvRow>
			<KvRow label={t("serviceConfig.endpoint.exposure", "Exposure")}>
				<ExposureRow hosting={hosting} />
			</KvRow>
			{hosting.exposure === "all" ? (
				<KvRow
					label={t("serviceConfig.endpoint.address", "Address people use")}
				>
					<AddressRow
						saved={saved.address}
						onSave={saved.save}
						suggestions={lanAddresses(inspection?.network?.interfaces)}
						supported={inspection?.features.network_interfaces === 1}
					/>
				</KvRow>
			) : null}
			<KvRow label={t("serviceConfig.token.name", "Access token")}>
				<TokenRow editor={editor} writer={writer} onOpen={onOpenToken} />
			</KvRow>
			<KvRow label={t("serviceConfig.endpoint.exposed", "Exposed")}>
				{t(
					"serviceConfig.endpoint.exposedText",
					"Pages and chats, listed on the service page · REST and MCP endpoints work but aren't listed there",
				)}
			</KvRow>
			<KvRow label={t("serviceConfig.endpoint.limits", "Limits")}>
				<span className="tabular-nums">
					{t(
						"serviceConfig.endpoint.limitsText",
						"{{parallel, number}} parallel requests · {{timeout, number}} s timeout · 10 MiB per request · attachments about 3.5 MiB",
						{ parallel: hosting.maxInFlight, timeout: hosting.timeoutS },
					)}
				</span>
			</KvRow>
			<KvRow label={t("serviceConfig.field.cert", "Certificate")}>
				<CertificateRow
					facts={facts}
					editor={editor}
					onOpen={onOpenCertificate}
				/>
			</KvRow>
		</KeyValueList>
	);
}

/* Services without a web endpoint. */

function useEventKinds(projectId: string | undefined, eventIds: string[]) {
	const { t } = useTranslation("devices");
	const { view } = useAppView(projectId);
	const kinds: Record<string, string> = {
		rest: t("serviceConfig.endpoint.kindRest", "REST"),
		mcp: t("serviceConfig.endpoint.kindMcp", "MCP"),
		daemon: t("serviceConfig.endpoint.kindBackground", "Background"),
	};
	const rows = view ? [...view.events.rows, ...view.events.ineligible] : [];
	const found = eventIds.flatMap((id) => {
		const kind = kinds[rows.find((row) => row.eventId === id)?.eventType ?? ""];
		return kind ? [kind] : [];
	});
	return [...new Set(found)];
}

function NoEndpoint({
	read,
	configuration,
}: Readonly<{
	read: ServiceConfigRead;
	configuration: PlacementConfiguration;
}>) {
	const { t } = useTranslation("devices");
	const kinds = useEventKinds(
		configuration.project_id,
		configuration.config.events.map((event) => event.event_id),
	);
	return (
		<Block
			id="svc-endpoint"
			icon={Globe}
			title={t("serviceConfig.endpoint.title", "Endpoint")}
			stamp={<ConfigStamp read={read} />}
		>
			<StateView
				kind="empty"
				icon={Globe}
				title={t(
					"serviceConfig.endpoint.none",
					"This service has no web endpoint.",
				)}
				text={
					kinds.length
						? t(
								"serviceConfig.endpoint.noneKinds",
								"Its events run as {{kinds}}, so it has no service page.",
								{ kinds: new Intl.ListFormat().format(kinds) },
							)
						: t(
								"serviceConfig.endpoint.noneText",
								"Its events don't answer page or chat requests, so it has no service page.",
							)
				}
			/>
		</Block>
	);
}

function Hosted({
	deviceId,
	serviceId,
	read,
	editor,
	configuration,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	read: ServiceConfigRead;
	editor: SettingsEditor;
	configuration: PlacementConfiguration;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const saved = useSavedAddress(deviceId, serviceId);
	const facts = useEndpoint(deviceId, configuration, saved.address);
	const writer = useSecretWrite(
		deviceId,
		serviceId,
		read.service,
		read.deviceLabel,
	);
	const [sheet, setSheet] = useState<"token" | "settings" | null>(null);
	const [note, setNote] = useState<Note | null>(null);
	if (!facts) return <NoEndpoint read={read} configuration={configuration} />;
	const body: ReactNode = (
		<>
			<LinkBlock facts={facts} device={read.deviceLabel} />
			<hr className="border-hairline" />
			<Rows
				deviceId={deviceId}
				facts={facts}
				editor={editor}
				writer={writer}
				saved={saved}
				onOpenToken={() => setSheet("token")}
				onOpenCertificate={() => setSheet("settings")}
			/>
			<ActionResults resultKey={writer.resultKey} />
			<ActionResults
				resultKey={editor.apply.resultKey}
				note={note}
				onDismiss={() => setNote(null)}
			/>
		</>
	);
	return (
		<Block
			id="svc-endpoint"
			icon={Globe}
			title={t("serviceConfig.endpoint.page", "Service page")}
			stamp={<ConfigStamp read={read} />}
		>
			{body}
			<NewTokenSheet
				open={sheet === "token"}
				onOpenChange={(open) => setSheet(open ? "token" : null)}
				serviceId={serviceId}
				deviceLabel={read.deviceLabel}
				reference={facts.hosting.authSecret}
				revision={configuration.config_revision}
				writer={writer}
				onSent={() =>
					setNote({
						tone: "info",
						text: t(
							"serviceConfig.endpoint.tokenSent",
							"New token sent at {{time}}. The old one works until the device saves it.",
							{ time: time.clock(time.nowS) },
						),
					})
				}
			/>
			<EditSettingsSheet
				editor={editor}
				open={sheet === "settings"}
				onOpenChange={(open) => setSheet(open ? "settings" : null)}
				onNote={setNote}
				focus="cert"
			/>
		</Block>
	);
}

/** SPEC §5.3 Endpoint: the service page's link, how it is reached and protected, and its access token. */
export function ServiceEndpointTab({
	deviceId,
	serviceId,
}: Readonly<ServiceTabProps>) {
	const { t } = useTranslation("devices");
	const read = useServiceConfig(deviceId, serviceId);
	const editor = useSettingsEditor(deviceId, serviceId, read);
	if (!read.configuration || !editor)
		return (
			<Block
				id="svc-endpoint"
				icon={Globe}
				title={t("serviceConfig.endpoint.title", "Endpoint")}
			>
				<ConfigUnavailable read={read} serviceId={serviceId} />
			</Block>
		);
	return (
		<Hosted
			deviceId={deviceId}
			serviceId={serviceId}
			read={read}
			editor={editor}
			configuration={read.configuration}
		/>
	);
}
