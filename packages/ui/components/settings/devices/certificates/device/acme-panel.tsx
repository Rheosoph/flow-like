"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { CircleStop, Globe } from "lucide-react";
import { useId, useState } from "react";
import {
	type AcmeCertificate,
	configureAcmeCertificate,
	deleteAcmeCertificate,
} from "../../../../../lib/device-management/certificate-acme";
import { readExistingDeployment } from "../../../../../lib/device-management/deployment";
import type { PlacementConfigFacts } from "../../../../../lib/device-management/model/types";
import { enumLabel } from "../../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { Checklist, type ChecklistItem } from "../../primitives/checklist";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import {
	CheckField,
	DvInput,
	DvTextarea,
	Field,
} from "../../primitives/form-fields";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import {
	deviceCall,
	useDeviceAction,
	useDeviceWorkspace,
	useInlineResults,
} from "../../workspace";
import type { CertificateFlows } from "./certificate-list";
import {
	ActionResults,
	FootReason,
	GatedButton,
	LINK,
	LabelField,
	LiveStamp,
	Names,
	SheetBlocker,
	TickFirst,
	certificateResultKey,
	failureSummary,
	gateView,
	labelProblem,
	lastErrorText,
	listOf,
	useAlive,
	useDay,
	withDeviceReason,
} from "./certificate-parts";
import { splitNames } from "./signing-requests";
import {
	type CertificateRow,
	type DeviceCertificates,
	MAX_CERTIFICATE_SLOTS,
	certificateLabel,
} from "./use-device-certificates";

type Environment = AcmeCertificate["environment"];

const DEFAULT_BIND = "0.0.0.0:80";
const MAX_NAMES = 64;
/** Service settings read for the port check when the sheet opens; more are reported as unchecked. */
const ENDPOINT_READS = 16;
const ANY_ADDRESS = new Set(["0.0.0.0", "::", "::0"]);
const BIND = /^(?:\[([0-9a-f:.]+)\]|(\d{1,3}(?:\.\d{1,3}){3})):(\d{1,5})$/iu;
const TERMS_URL = "https://letsencrypt.org/repository/";

export const acmeResultKey = (deviceId: string) =>
	certificateResultKey(deviceId, "acme");

/** `0.0.0.0:80` or `[::]:80`; null when it isn't an IP address with a port. */
export function parseBind(
	value: string,
): { host: string; port: number } | null {
	const match = BIND.exec(value.trim());
	const host = match?.[1] ?? match?.[2];
	const port = Number(match?.[3]);
	if (!host || !Number.isInteger(port) || port < 1 || port > 65_535)
		return null;
	return { host, port };
}

/** Services that already listen where the challenge has to be answered. */
export function portConflicts(
	bind: { host: string; port: number },
	endpoints: DeviceCertificates["endpoints"],
) {
	return endpoints.filter(
		(endpoint) =>
			endpoint.port === bind.port &&
			(endpoint.host === bind.host ||
				ANY_ADDRESS.has(endpoint.host) ||
				ANY_ADDRESS.has(bind.host)),
	);
}

/**
 * S36 port check: reads the settings of the services whose address isn't
 * known on this computer yet, so a conflict shows before the device tries.
 */
function useServiceEndpoints(certs: DeviceCertificates) {
	const workspace = useDeviceWorkspace();
	const { deviceId, services, checkedServices } = certs;
	const known = new Set(checkedServices);
	const missing = services
		.filter((service) => !known.has(service.id))
		.slice(0, ENDPOINT_READS);
	const query = useQuery({
		queryKey: [
			"devices",
			workspace.scopeKey,
			"device-certificates",
			deviceId,
			"endpoints",
			missing.map((service) => service.id).join(","),
		],
		queryFn: async () => {
			const call = deviceCall(workspace, deviceId, "poll");
			const read: Record<string, PlacementConfigFacts> = {};
			for (const service of missing) {
				try {
					const { config } = await readExistingDeployment(
						call,
						service.id,
						service.projectId,
					);
					read[service.id] = {
						...workspace.facts.get(deviceId)?.placements?.[service.id],
						...(config.hosting
							? { host: config.hosting.host, port: config.hosting.port }
							: {}),
						tlsCertificateId: config.tls_certificate_id ?? null,
					};
				} catch {
					// A service whose settings can't be read stays unchecked.
				}
			}
			if (
				Object.keys(read).length &&
				workspace.keys.snapshot(deviceId).state === "unlocked"
			)
				workspace.facts.record(deviceId, { placements: read });
			return Object.keys(read).length;
		},
		enabled: certs.live && missing.length > 0,
		staleTime: Number.POSITIVE_INFINITY,
		gcTime: 0,
		retry: false,
		meta: { persist: false },
	});
	return {
		reading: query.isFetching,
		total: services.length,
		checked: services.filter((service) => known.has(service.id)).length,
	};
}

interface PortCheck {
	bind: { host: string; port: number } | null;
	conflicts: DeviceCertificates["endpoints"];
	reading: boolean;
	total: number;
	checked: number;
}

function portItem(
	t: DevicesT,
	check: PortCheck,
	healthy: boolean,
): ChecklistItem {
	const port = check.bind?.port ?? 80;
	const label = t(
		"devices:deviceCertificates.acme.check.port",
		"Port {{port}} is free for the challenge: no service of this device listens on it",
		{ port },
	);
	const base = { id: "port", label };
	if (check.conflicts.length)
		return {
			...base,
			state: "fail",
			note: t("devices:deviceCertificates.acme.check.portTaken", {
				count: check.conflicts.length,
				services: check.conflicts
					.map((endpoint) => endpoint.serviceId)
					.join(", "),
				port,
				defaultValue_one:
					"{{services}} already listens on port {{port}}. Pick another challenge address and forward /.well-known/acme-challenge/ to it, or move the service.",
				defaultValue_other:
					"{{services}} already listen on port {{port}}. Pick another challenge address and forward /.well-known/acme-challenge/ to it, or move the services.",
			}),
		};
	if (healthy) return { ...base, state: "pass" };
	if (check.reading)
		return {
			...base,
			state: "active",
			note: t(
				"devices:deviceCertificates.acme.check.portReading",
				"Reading the addresses of this device's services…",
			),
		};
	if (check.checked < check.total)
		return {
			...base,
			state: "warn",
			note: t(
				"devices:deviceCertificates.acme.check.portPartial",
				"Checked {{checked, number}} of {{count, number}} services. The device reports a taken port after its first attempt.",
				{ checked: check.checked, count: check.total },
			),
		};
	return {
		...base,
		state: "pass",
		note: t("devices:deviceCertificates.acme.check.portFree", {
			count: check.total,
			defaultValue_one: "Checked the device's {{count, number}} service.",
			defaultValue_other: "Checked the device's {{count, number}} services.",
			defaultValue_zero: "This device runs no services.",
		}),
	};
}

/** What Let's Encrypt needs. Only the port can be checked from here; the rest shows once the device tried. */
function prerequisites(
	t: DevicesT,
	check: PortCheck,
	policy?: AcmeCertificate,
): ChecklistItem[] {
	const tried = !!policy && policy.last_renewed_at !== null;
	const healthy = tried && !policy.last_error && (policy.failures ?? 0) === 0;
	const cause = policy?.error_category;
	const outcome = (failed: boolean) =>
		failed ? "fail" : healthy ? "pass" : "pending";
	const untried = t(
		"devices:deviceCertificates.acme.check.untried",
		"Let's Encrypt checks this when the device asks for the certificate.",
	);
	const port = portItem(t, check, healthy);
	return [
		{
			id: "dns",
			state: outcome(cause === "dns"),
			label: t(
				"devices:deviceCertificates.acme.check.dns",
				"The public DNS names point at this device or its proxy",
			),
			...(healthy || cause === "dns" ? {} : { note: untried }),
		},
		cause === "port_bind" && port.state !== "fail"
			? {
					...port,
					state: "fail",
					note: t(
						"devices:deviceCertificates.acme.check.portBind",
						"The device couldn't open the challenge address on its last attempt.",
					),
				}
			: port,
		{
			id: "outbound",
			state: outcome(cause === "network"),
			label: t(
				"devices:deviceCertificates.acme.check.outbound",
				"The device and Let's Encrypt reach each other",
			),
			...(healthy || cause === "network" ? {} : { note: untried }),
		},
	];
}

function useStopAcme(certs: DeviceCertificates) {
	const { t } = useTranslation("devices");
	const day = useDay();
	const actions = useDeviceAction();
	return async (policy: AcmeCertificate, row: CertificateRow | undefined) => {
		const { label } = policy;
		const users = row?.uses ?? 0;
		const outcome = await actions.run({
			action: "acme_configure",
			deviceId: certs.deviceId,
			label: t(
				"deviceCertificates.acme.stopLabel",
				"Stop Let's Encrypt for {{label}}",
				{ label },
			),
			consequence: {
				what: t(
					"deviceCertificates.acme.stopWhat",
					"The device stops renewing {{label}} with Let's Encrypt and discards its Let's Encrypt account for it.",
					{ label },
				),
				who: users
					? t(
							"deviceCertificates.acme.stopWhoUsed",
							"Nobody yet: the services that use {{label}} keep working until it expires.",
							{ label },
						)
					: t(
							"deviceCertificates.acme.stopWho",
							"Nobody: no service uses {{label}} today.",
							{ label },
						),
				when: row
					? t(
							"deviceCertificates.acme.stopWhen",
							"{{label}} keeps working until {{date}}, then expires.",
							{ label, date: day(row.notAfter) },
						)
					: t(
							"deviceCertificates.acme.stopWhenUnissued",
							"Immediately. No certificate was issued yet.",
						),
				undo: {
					reversible: true,
					text: t(
						"deviceCertificates.acme.stopUndo",
						"Set up Let's Encrypt again.",
					),
				},
			},
			strength: "none",
			confirm: {
				title: t(
					"deviceCertificates.acme.stopTitle",
					"Stop Let's Encrypt for {{label}}?",
					{ label },
				),
				sub: certs.name,
				tone: "danger",
				icon: CircleStop,
			},
			resultKey: acmeResultKey(certs.deviceId),
			call: (context) =>
				withDeviceReason(context, (call) =>
					deleteAcmeCertificate(call, policy),
				),
		});
		if (outcome.status === "done") await certs.reload();
	};
}

function AcmePolicy({
	certs,
	policy,
	flows,
	onStop,
}: Readonly<{
	certs: DeviceCertificates;
	policy: AcmeCertificate;
	flows: CertificateFlows;
	onStop(policy: AcmeCertificate, row: CertificateRow | undefined): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const row = certs.rows.find((entry) => entry.id === policy.certificate_id);
	const summary = failureSummary(t, policy, certs.support.failureDetail);
	const error = lastErrorText(t, policy);
	const failing = !!policy.last_error || (policy.failures ?? 0) > 0;
	const bind = parseBind(policy.http_bind);
	const check: PortCheck = {
		bind,
		conflicts: bind ? portConflicts(bind, certs.endpoints) : [],
		reading: false,
		total: certs.services.length,
		checked: certs.services.filter((service) =>
			certs.checkedServices.includes(service.id),
		).length,
	};
	const gate = certs.gate("acme_configure");
	return (
		<div
			data-acme-policy={policy.certificate_id}
			className="flex flex-col gap-3"
		>
			{failing ? (
				<Banner
					tone="warning"
					title={t(
						"deviceCertificates.acme.failingTitle",
						"Let's Encrypt couldn't renew {{label}}.",
						{ label: policy.label },
					)}
				>
					{[summary.attempts, error].filter(Boolean).join(". ")}{" "}
					{t(
						"deviceCertificates.acme.failingNext",
						"The device tries again {{at}}.",
						{ at: time.at(policy.next_attempt_at) },
					)}
				</Banner>
			) : null}
			<KeyValueList>
				<KvRow label={t("deviceCertificates.acme.certificate", "Certificate")}>
					<b className="font-semibold">{policy.label}</b>
					{row ? null : (
						<span className="ml-1.5 text-xs text-muted-foreground">
							{t("deviceCertificates.acme.notIssued", "not issued yet")}
						</span>
					)}
				</KvRow>
				<KvRow label={t("deviceCertificates.acme.names", "Names")}>
					<Names names={policy.dns_names} />
				</KvRow>
				<KvRow label={t("deviceCertificates.acme.environment", "Environment")}>
					{enumLabel(t, "acmeEnvironment", policy.environment)}
				</KvRow>
				<KvRow label={t("deviceCertificates.acme.bind", "Challenge address")}>
					<span className="font-mono text-xs">{policy.http_bind}</span>
				</KvRow>
				<KvRow label={t("deviceCertificates.acme.next", "Next attempt")}>
					{time.at(policy.next_attempt_at)}
				</KvRow>
				<KvRow label={t("deviceCertificates.acme.last", "Last issued")}>
					{policy.last_renewed_at === null
						? t("deviceCertificates.renewal.never", "Not yet")
						: time.at(policy.last_renewed_at)}
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
			<Checklist
				label={t(
					"deviceCertificates.acme.check.label",
					"What Let's Encrypt needs",
				)}
				items={prerequisites(t, check, policy)}
			/>
			<div className="flex flex-wrap items-start gap-2">
				<GatedButton
					result={gate}
					icon={Globe}
					onClick={() => flows.setupAcme(policy.certificate_id)}
				>
					{t("deviceCertificates.acme.change", "Change settings…")}
				</GatedButton>
				<GatedButton
					result={gate}
					variant="danger-ghost"
					onClick={() => onStop(policy, row)}
				>
					{t("deviceCertificates.acme.stop", "Stop Let's Encrypt…")}
				</GatedButton>
			</div>
		</div>
	);
}

/** S36: public certificates the device gets and renews from Let's Encrypt. */
export function AcmePanel({
	certs,
	flows,
}: Readonly<{ certs: DeviceCertificates; flows: CertificateFlows }>) {
	const { t } = useTranslation("devices");
	const { acme } = certs;
	const stop = useStopAcme(certs);
	const results = useInlineResults(acmeResultKey(certs.deviceId));
	const rows = acme.data;
	// No Let's Encrypt setup and nothing to report: "Add certificate" and a certificate's menu start one.
	if (rows && !rows.length && !results.length) return null;
	return (
		<Block
			id="device-certificate-acme"
			icon={Globe}
			title={t(
				"deviceCertificates.acme.title",
				"Automatic renewal · Let's Encrypt",
			)}
			stamp={<LiveStamp freshness={acme.freshness} />}
		>
			{!rows ? (
				acme.failed ? (
					<StateView
						kind="error"
						title={t(
							"deviceCertificates.acme.error",
							"Couldn't read the Let's Encrypt settings from {{device}}",
							{ device: certs.name },
						)}
						actions={
							<DvButton size="sm" onClick={() => void acme.refetch()}>
								{t("deviceCertificates.error.retry", "Retry now")}
							</DvButton>
						}
					/>
				) : (
					<StateView
						kind="loading"
						rows={2}
						title={t(
							"deviceCertificates.acme.loading",
							"Reading Let's Encrypt settings…",
						)}
					/>
				)
			) : rows.length ? (
				rows.map((policy, index) => (
					<div
						key={policy.certificate_id}
						className={index ? "border-t border-hairline pt-3" : undefined}
					>
						<AcmePolicy
							certs={certs}
							policy={policy}
							flows={flows}
							onStop={(target, row) => void stop(target, row)}
						/>
					</div>
				))
			) : (
				<>
					<p className="max-w-[80ch] text-ui text-muted-foreground">
						{t(
							"deviceCertificates.acme.none",
							"No certificate renews with Let's Encrypt yet. It issues certificates for public DNS names that browsers trust, without an authority of your own. The names must point at this device, and port 80 must reach it from the internet.",
						)}
					</p>
					<div>
						<GatedButton
							result={certs.gate("acme_configure")}
							icon={Globe}
							onClick={() => flows.setupAcme()}
						>
							{t("deviceCertificates.add.acme", "Set up Let's Encrypt…")}
						</GatedButton>
					</div>
				</>
			)}
			<ActionResults scopeKey={acmeResultKey(certs.deviceId)} />
		</Block>
	);
}

/* Set up or change Let's Encrypt for one certificate. */

type NameProblem = "empty" | "too_many" | "ip" | "not_public";

function nameProblem(dns: string[], ips: string[]): NameProblem | null {
	if (ips.length) return "ip";
	if (!dns.length) return "empty";
	if (dns.length > MAX_NAMES) return "too_many";
	return dns.every((name) => name.includes(".")) ? null : "not_public";
}

/** What the sheet sets up: an existing Let's Encrypt setup, an existing certificate, or a new one. */
interface SetupTarget {
	certificateId?: string;
	policy?: AcmeCertificate;
	row?: CertificateRow;
	/** The label an existing certificate or setup already has. */
	label?: string;
	/** IP names the certificate carries today; Let's Encrypt can't keep them. */
	dropped: string[];
}

function setupTarget(
	certs: DeviceCertificates,
	certificateId: string | undefined,
): SetupTarget {
	const policy = certs.acme.data?.find(
		(entry) => entry.certificate_id === certificateId,
	);
	const row = certs.rows.find((entry) => entry.id === certificateId);
	const label = policy?.label ?? (row ? certificateLabel(row) : undefined);
	return {
		...(certificateId ? { certificateId } : {}),
		...(policy ? { policy } : {}),
		...(row ? { row } : {}),
		...(label === undefined ? {} : { label }),
		dropped: policy ? [] : (row?.detail?.ip_addresses ?? []),
	};
}

interface SetupForm {
	label: string;
	names: string;
	environment: Environment;
	bind: string;
	agreed: boolean;
	dropAcknowledged: boolean;
	touched: boolean;
	ran: boolean;
}

function initialForm({ policy, row, label }: SetupTarget): SetupForm {
	return {
		label: label ?? "",
		names: (policy?.dns_names ?? row?.detail?.dns_names ?? []).join("\n"),
		environment: policy?.environment ?? "lets_encrypt_staging",
		bind: policy?.http_bind ?? DEFAULT_BIND,
		agreed: false,
		dropAcknowledged: false,
		touched: false,
		ran: false,
	};
}

function NamesField({
	id,
	value,
	onChange,
	showProblem,
}: Readonly<{
	id: string;
	value: string;
	onChange(value: string): void;
	showProblem: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { dns, ips } = splitNames(value);
	const problem = showProblem ? nameProblem(dns, ips) : null;
	const texts: Record<NameProblem, string> = {
		empty: t(
			"deviceCertificates.acme.namesEmpty",
			"Add at least one public DNS name.",
		),
		too_many: t(
			"deviceCertificates.acme.namesTooMany",
			"Let's Encrypt setups carry at most {{max, number}} names.",
			{ max: MAX_NAMES },
		),
		ip: t(
			"deviceCertificates.acme.namesIp",
			"Let's Encrypt can't issue for IP addresses. Remove {{ips}}.",
			{ ips: listOf(time.locale, ips) },
		),
		not_public: t(
			"deviceCertificates.acme.namesNotPublic",
			"Every name must be a public DNS name, like api.example.com.",
		),
	};
	return (
		<Field
			id={id}
			label={t("deviceCertificates.acme.namesField", "Public DNS names")}
			hint={t(
				"deviceCertificates.acme.namesHint",
				"One per line. Each must resolve to this device or its proxy from the internet.",
			)}
			error={problem ? texts[problem] : undefined}
		>
			<DvTextarea
				className="font-mono"
				rows={3}
				spellCheck={false}
				placeholder="api.example.com"
				value={value}
				onChange={(event) => onChange(event.target.value)}
			/>
		</Field>
	);
}

function EnvironmentField({
	value,
	onChange,
}: Readonly<{ value: Environment; onChange(value: Environment): void }>) {
	const { t } = useTranslation("devices");
	const label = t("deviceCertificates.acme.environment", "Environment");
	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-[13px]/[18px] font-medium">{label}</span>
			<Segmented<Environment>
				label={label}
				className="self-start"
				size="sm"
				wrap
				value={value}
				onChange={onChange}
				options={[
					{
						value: "lets_encrypt_staging",
						label: enumLabel(t, "acmeEnvironment", "lets_encrypt_staging"),
					},
					{
						value: "lets_encrypt_production",
						label: enumLabel(t, "acmeEnvironment", "lets_encrypt_production"),
					},
				]}
			/>
			<p className="text-xs text-muted-foreground">
				{value === "lets_encrypt_staging"
					? t(
							"deviceCertificates.acme.stagingHint",
							"Test certificates prove the setup works. Browsers don't trust them: switch to Production afterwards.",
						)
					: t(
							"deviceCertificates.acme.productionHint",
							"Trusted by browsers. The names become public in certificate transparency logs.",
						)}
			</p>
		</div>
	);
}

function whatHappens(t: DevicesT, { policy, row, label }: SetupTarget) {
	if (policy)
		return t(
			"devices:deviceCertificates.acme.whatChange",
			"The device replaces the Let's Encrypt setup of {{label}}: its old account and any pending order are discarded, and it asks for a new certificate.",
			{ label: policy.label },
		);
	if (!row)
		return t(
			"devices:deviceCertificates.acme.whatNew",
			"The device asks Let's Encrypt for a certificate for these names and renews it by itself, 30 days before each expiry.",
		);
	return row.mode === "delegated"
		? t(
				"devices:deviceCertificates.acme.whatConvertDelegated",
				"The device renews {{label}} with Let's Encrypt from now on. Renewal with your authority stops.",
				{ label },
			)
		: t(
				"devices:deviceCertificates.acme.whatConvert",
				"The device renews {{label}} with Let's Encrypt from now on, 30 days before each expiry.",
				{ label },
			);
}

function whoNotices(t: DevicesT, locale: string, target: SetupTarget) {
	if (target.dropped.length)
		return t(
			"devices:deviceCertificates.acme.whoDropped",
			"Clients that connect by {{ips}} get certificate errors: Let's Encrypt can't issue for IP addresses.",
			{ ips: listOf(locale, target.dropped) },
		);
	return target.row?.uses
		? t(
				"devices:deviceCertificates.acme.whoUsed",
				"Nobody: the services that use {{label}} keep serving throughout.",
				{ label: target.label },
			)
		: t(
				"devices:deviceCertificates.acme.who",
				"Nobody: nothing uses it until you pick it for a service's web endpoint.",
			);
}

/** SPEC §6.5 "Configure Let's Encrypt on an existing certificate", and the rows of a new setup. */
function setupRows(
	t: DevicesT,
	locale: string,
	target: SetupTarget,
): ConsequenceRows {
	return {
		what: whatHappens(t, target),
		who: whoNotices(t, locale, target),
		...(target.row
			? {
					stays: t(
						"devices:deviceCertificates.acme.stays",
						"The certificate ID and the services that use it.",
					),
				}
			: {}),
		when: t(
			"devices:deviceCertificates.acme.when",
			"The first request runs within a minute. Port 80 must stay reachable while Let's Encrypt checks.",
		),
		undo: {
			reversible: true,
			text: t(
				"devices:deviceCertificates.acme.undo",
				"Stop Let's Encrypt at any time. The certificate then expires on its date.",
			),
		},
	};
}

/** The two ticks: IP names that get dropped, and the subscriber agreement (only once the device's settings were read). */
function Consent({
	id,
	target,
	form,
	patch,
	settings,
}: Readonly<{
	id: string;
	target: SetupTarget;
	form: SetupForm;
	patch(change: Partial<SetupForm>): void;
	settings: "read" | "reading" | "failed";
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { dropped } = target;
	return (
		<>
			{dropped.length ? (
				<CheckField
					id={`${id}-dropped`}
					checked={form.dropAcknowledged}
					onCheckedChange={(dropAcknowledged) => patch({ dropAcknowledged })}
				>
					{t("deviceCertificates.acme.dropped", {
						count: dropped.length,
						ips: listOf(time.locale, dropped),
						defaultValue_one:
							"I understand {{ips}} is dropped from the certificate.",
						defaultValue_other:
							"I understand {{ips}} are dropped from the certificate.",
					})}
				</CheckField>
			) : null}
			<CheckField
				id={`${id}-terms`}
				checked={form.agreed}
				disabled={settings !== "read"}
				onCheckedChange={(agreed) => patch({ agreed })}
			>
				<Trans
					t={t}
					i18nKey="deviceCertificates.acme.terms"
					defaults="I accept the <1>Let's Encrypt subscriber agreement</1> and authorise this device to request and renew certificates for these names."
					components={{
						1: (
							// biome-ignore lint/a11y/useAnchorContent: the translation supplies the link text
							<a
								href={TERMS_URL}
								target="_blank"
								rel="noopener noreferrer"
								className={LINK}
							/>
						),
					}}
				/>
			</CheckField>
			{settings === "read" ? null : (
				<output className="text-xs text-muted-foreground">
					{settings === "failed"
						? t(
								"deviceCertificates.acme.termsUnread",
								"The device's current Let's Encrypt settings couldn't be read, so nothing can be changed yet. Refresh the tab.",
							)
						: t(
								"deviceCertificates.acme.termsReading",
								"Reading the device's current Let's Encrypt settings before you can agree…",
							)}
				</output>
			)}
		</>
	);
}

function sheetTitle(t: DevicesT, { policy, label }: SetupTarget): string {
	if (policy)
		return t(
			"devices:deviceCertificates.acme.changeTitle",
			"Change Let's Encrypt for {{label}}",
			{ label: policy.label },
		);
	return label === undefined
		? t("devices:deviceCertificates.acme.setupTitle", "Set up Let's Encrypt")
		: t(
				"devices:deviceCertificates.acme.setupTitleFor",
				"Set up Let's Encrypt for {{label}}",
				{ label },
			);
}

export function AcmeSetupSheet({
	certs,
	certificateId,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	/** An existing certificate or Let's Encrypt setup; absent for a new certificate. */
	certificateId?: string;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const alive = useAlive();
	const actions = useDeviceAction();
	const target = setupTarget(certs, certificateId);
	const [form, setForm] = useState(() => initialForm(target));
	const patch = (change: Partial<SetupForm>) =>
		setForm((current) => ({ ...current, ...change }));
	const endpoints = useServiceEndpoints(certs);
	const resultKey = acmeResultKey(certs.deviceId);
	const { dns, ips } = splitNames(form.names);
	const bind = parseBind(form.bind);
	const conflicts = bind ? portConflicts(bind, certs.endpoints) : [];
	const settingsRead = certs.acme.data !== undefined;
	const waiting = certs.requests.data?.find(
		(entry) => entry.certificate_id === certificateId,
	);
	const full =
		!target.policy &&
		!target.row &&
		(certs.slots?.used ?? 0) >= MAX_CERTIFICATE_SLOTS;
	const view = gateView(t, time, certs.gate("acme_configure"));
	const unanswered =
		!settingsRead ||
		!form.agreed ||
		(target.dropped.length > 0 && !form.dropAcknowledged);
	const blocked =
		!!view || !!waiting || full || unanswered || conflicts.length > 0;
	// The gate, a waiting request and full slots are explained at the top of the sheet.
	const footReason =
		view || waiting || full ? undefined : conflicts.length ? (
			<FootReason>
				{t(
					"deviceCertificates.acme.footPort",
					"The challenge port is taken by a service on this device.",
				)}
			</FootReason>
		) : !settingsRead ? (
			<FootReason>
				{t(
					"deviceCertificates.acme.footSettings",
					"The device's current settings aren't read yet.",
				)}
			</FootReason>
		) : unanswered ? (
			<TickFirst />
		) : undefined;
	const invalid =
		(target.label === undefined && labelProblem(form.label) !== null) ||
		nameProblem(dns, ips) !== null ||
		!bind;

	const submit = async () => {
		patch({ touched: true });
		if (blocked || invalid) return;
		patch({ ran: true });
		const name = (target.label ?? form.label).trim();
		const outcome = await actions.run({
			action: "acme_configure",
			deviceId: certs.deviceId,
			label: t(
				"deviceCertificates.acme.setupLabel",
				"Set up Let's Encrypt for {{label}}",
				{ label: name },
			),
			resultKey,
			call: (context) =>
				withDeviceReason(context, async (call) => {
					await configureAcmeCertificate(call, {
						certificateId: certificateId ?? crypto.randomUUID(),
						label: name,
						expectedRevision: target.policy?.revision ?? 0,
						expectedCertificateRevision: target.row?.detail
							? target.row.revision
							: 0,
						dnsNames: dns,
						environment: form.environment,
						httpBind: form.bind,
						termsAgreed: true,
					});
				}),
		});
		if (alive()) patch({ agreed: false });
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
			icon={Globe}
			title={sheetTitle(t, target)}
			sub={certs.name}
			footNote={footReason}
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
						{target.policy
							? t("deviceCertificates.acme.save", "Save and request again")
							: t("deviceCertificates.acme.setup", "Set up Let's Encrypt")}
					</DvButton>
				</>
			}
		>
			<SheetBlocker
				view={view}
				{...(waiting ? { waitingFor: target.label ?? waiting.label } : {})}
				full={full}
			/>
			{target.label === undefined ? (
				<LabelField
					id={`${id}-label`}
					value={form.label}
					onChange={(label) => patch({ label })}
					showProblem={form.touched}
				/>
			) : null}
			<NamesField
				id={`${id}-names`}
				value={form.names}
				onChange={(names) => patch({ names, agreed: false })}
				showProblem={form.touched}
			/>
			<EnvironmentField
				value={form.environment}
				onChange={(environment) => patch({ environment, agreed: false })}
			/>
			<Field
				id={`${id}-bind`}
				label={t("deviceCertificates.acme.bind", "Challenge address")}
				hint={t(
					"deviceCertificates.acme.bindHint",
					"Where the device answers Let's Encrypt. Port 80 of the names must reach it, directly or through a proxy that forwards /.well-known/acme-challenge/.",
				)}
				error={
					form.touched && !bind
						? t(
								"deviceCertificates.acme.bindError",
								"Enter an IP address and a port, like 0.0.0.0:80.",
							)
						: undefined
				}
			>
				<DvInput
					mono
					className="max-w-60"
					autoComplete="off"
					value={form.bind}
					onChange={(event) => patch({ bind: event.target.value })}
				/>
			</Field>
			<Checklist
				label={t(
					"deviceCertificates.acme.check.label",
					"What Let's Encrypt needs",
				)}
				items={prerequisites(t, { bind, conflicts, ...endpoints })}
			/>
			<ConsequencePreview rows={setupRows(t, time.locale, target)} />
			<Consent
				id={id}
				target={target}
				form={form}
				patch={patch}
				settings={
					settingsRead ? "read" : certs.acme.failed ? "failed" : "reading"
				}
			/>
			{form.ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}
