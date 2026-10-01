import type { AcmeCertificate } from "../../certificate-acme";
import type { CertificateIssuer } from "../../certificate-issuance";
import {
	CERTIFICATE_WARNING_SECONDS,
	type DeviceCertificate,
} from "../../certificates";
import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	DAY_S,
	type DeviceFacts,
	HOUR_S,
	attentionCandidate,
	deviceRoute,
	fleetFacts,
	localSource,
	perDevice,
} from "../device-view";
import { classify } from "../freshness";
import type {
	AttentionKey,
	CopyParams,
	DevicesRoute,
	Freshness,
} from "../types";

const AUTHORITY_EXPIRING_S = 30 * DAY_S;
const ROOT_EXPIRING_S = 90 * DAY_S;
const REQUEST_EXPIRING_S = 7 * DAY_S;
const INVENTORY_STALE_S = 2 * HOUR_S;
const MAX_CERTIFICATES = 32;
const CERTIFICATES_NEARLY_FULL = 28;

type Renewal =
	| "authority_expired"
	| "authority_expiring"
	| "delegation_error"
	| "acme_error";

/** One certificate as the hub inventory and the live read describe it together. */
interface CertificateFacts {
	device: DeviceFacts;
	certificateId: string;
	label?: string;
	notBefore?: number;
	notAfter: number;
	services: number;
	source: Freshness;
	issuer?: CertificateIssuer;
	acme?: AcmeCertificate;
}

/** The hub inventory and the live read of one device's certificates. */
interface CertificateSources {
	device: DeviceFacts;
	hubRows: readonly HubCertificate[];
	liveRows: readonly DeviceCertificate[];
	hubSource?: Freshness;
	liveSource: Freshness;
}

type HubCertificate = NonNullable<
	AttentionInputExt["certInventory"][string]
>["certificates"][number];

const byCertificate = <T extends { certificate_id: string }>(
	rows: readonly T[] | undefined,
	certificateId: string,
) => rows?.find((row) => row.certificate_id === certificateId);

const boundServices = (row: DeviceCertificate | undefined) =>
	row ? (row.binding_count ?? row.bindings.length) : 0;

const liveDetails = (
	device: DeviceFacts,
	row: DeviceCertificate | undefined,
	certificateId: string,
): Partial<CertificateFacts> => {
	const issuer = byCertificate(
		device.liveInput?.certificateIssuers,
		certificateId,
	);
	const acme = byCertificate(device.liveInput?.acme, certificateId);
	return {
		...(row ? { label: row.label, notBefore: row.not_before } : {}),
		...(issuer ? { issuer } : {}),
		...(acme ? { acme } : {}),
	};
};

const mergeCertificate = (
	sources: CertificateSources,
	certificateId: string,
): CertificateFacts => {
	const fromHub = byCertificate(sources.hubRows, certificateId);
	const fromLive = byCertificate(sources.liveRows, certificateId);
	// A live read kept from a closed session can predate a renewal the hub already knows.
	const newest =
		fromHub && fromLive && fromHub.revision > fromLive.revision
			? fromHub
			: (fromLive ?? fromHub);
	return {
		device: sources.device,
		certificateId,
		notAfter: newest?.not_after ?? 0,
		services: boundServices(fromLive),
		source:
			fromHub && sources.hubSource ? sources.hubSource : sources.liveSource,
		...liveDetails(sources.device, fromLive, certificateId),
	};
};

const certificateSources = (
	input: AttentionInputExt,
	device: DeviceFacts,
): CertificateSources => {
	const hub = input.certInventory[device.id];
	const sources: CertificateSources = {
		device,
		hubRows: hub?.certificates ?? [],
		liveRows: device.liveInput?.certificates?.certificates ?? [],
		liveSource:
			device.inspectionSource ??
			classify("certificates_live", {
				now: input.now,
				loaded: true,
				sessionOpen: device.liveOpen,
			}),
	};
	if (hub)
		sources.hubSource = classify("cert_inventory", {
			now: input.now,
			at: hub.updated_at,
			loaded: true,
			noFail: true,
		});
	return sources;
};

const certificatesOf = (input: AttentionInputExt, device: DeviceFacts) => {
	const sources = certificateSources(input, device);
	const ids = new Set(
		[...sources.hubRows, ...sources.liveRows].map((row) => row.certificate_id),
	);
	return [...ids].map((certificateId) =>
		mergeCertificate(sources, certificateId),
	);
};

const cache = new WeakMap<AttentionInputExt, CertificateFacts[]>();

function allCertificates(input: AttentionInputExt): CertificateFacts[] {
	const cached = cache.get(input);
	if (cached) return cached;
	const certificates = fleetFacts(input).devices.flatMap((device) =>
		device.active ? certificatesOf(input, device) : [],
	);
	cache.set(input, certificates);
	return certificates;
}

function expiryState(certificate: CertificateFacts, now: number) {
	if (certificate.notAfter <= now) return "expired";
	if (certificate.notAfter - now <= CERTIFICATE_WARNING_SECONDS)
		return "expiring";
	return undefined;
}

/** The renewal problem that explains why a certificate won't renew itself. */
function renewalProblem(
	certificate: CertificateFacts,
	now: number,
): { renewal: Renewal; at?: number } | undefined {
	const { issuer, acme } = certificate;
	if (issuer && issuer.not_after <= now)
		return { renewal: "authority_expired", at: issuer.not_after };
	if (issuer && issuer.not_after - now <= AUTHORITY_EXPIRING_S)
		return { renewal: "authority_expiring", at: issuer.not_after };
	if (issuer?.last_error) return { renewal: "delegation_error" };
	if (acme?.last_error)
		return { renewal: "acme_error", at: acme.next_attempt_at };
	return undefined;
}

/** Automatic renewal is healthy and scheduled before expiry (IA §6.5 Info variant). */
function renewsOnItsOwn(certificate: CertificateFacts, now: number) {
	if (renewalProblem(certificate, now)) return undefined;
	const { issuer, acme } = certificate;
	const at = issuer?.next_renewal_at ?? acme?.next_attempt_at;
	return at !== undefined && at < certificate.notAfter ? at : undefined;
}

function certificateParams(certificate: CertificateFacts): CopyParams {
	return {
		device: certificate.device.name,
		certificateId: certificate.certificateId.slice(0, 8),
		...(certificate.label ? { label: certificate.label } : {}),
	};
}

function certificateRoute(certificate: CertificateFacts): DevicesRoute {
	return deviceRoute(
		certificate.device.id,
		"certificates",
		certificate.certificateId,
	);
}

function certificateItem(
	key: AttentionKey,
	severity: AttentionCandidateExt["severity"],
	certificate: CertificateFacts,
	params: CopyParams,
	action: AttentionCandidateExt["action"],
	since?: number,
): AttentionCandidateExt {
	return attentionCandidate({
		key,
		severity,
		subject: {
			kind: "certificate",
			deviceId: certificate.device.id,
			certificateId: certificate.certificateId,
		},
		params: { ...certificateParams(certificate), ...params },
		action,
		source: certificate.source,
		lastKnown: certificate.source.age === "lastknown",
		since,
	});
}

function perCertificate(
	key: AttentionKey,
	evaluate: (
		input: AttentionInputExt,
		certificate: CertificateFacts,
	) => AttentionCandidateExt | undefined,
): AttentionRuleExt {
	return {
		key,
		evaluate: (input) =>
			allCertificates(input).flatMap(
				(certificate) => evaluate(input, certificate) ?? [],
			),
	};
}

function renewalParams(problem: ReturnType<typeof renewalProblem>): CopyParams {
	if (!problem) return {};
	return problem.at === undefined
		? { renewal: problem.renewal }
		: { renewal: problem.renewal, renewalAt: problem.at };
}

const expired = perCertificate("certificate_expired", (input, certificate) => {
	if (expiryState(certificate, input.now) !== "expired") return undefined;
	const problem = renewalProblem(certificate, input.now);
	return certificateItem(
		"certificate_expired",
		certificate.services > 0 ? "critical" : "warning",
		certificate,
		{
			expiredAt: certificate.notAfter,
			services: certificate.services,
			...renewalParams(problem),
		},
		{
			code: problem ? "fix_renewal" : "renew",
			target: certificateRoute(certificate),
		},
		certificate.notAfter,
	);
});

const expiring = perCertificate(
	"certificate_expiring",
	(input, certificate) => {
		if (expiryState(certificate, input.now) !== "expiring") return undefined;
		const problem = renewalProblem(certificate, input.now);
		const renewsAt = renewsOnItsOwn(certificate, input.now);
		return certificateItem(
			"certificate_expiring",
			renewsAt === undefined ? "warning" : "info",
			certificate,
			{
				expiresAt: certificate.notAfter,
				days: Math.ceil((certificate.notAfter - input.now) / DAY_S),
				...renewalParams(problem),
				...(renewsAt === undefined ? {} : { renewsAt }),
			},
			{
				code: problem ? "fix_renewal" : "renew",
				target: certificateRoute(certificate),
			},
			certificate.notAfter - CERTIFICATE_WARNING_SECONDS,
		);
	},
);

const notYetValid = perCertificate(
	"certificate_not_yet_valid",
	(input, certificate) =>
		certificate.notBefore !== undefined && certificate.notBefore > input.now
			? certificateItem(
					"certificate_not_yet_valid",
					"notice",
					certificate,
					{ validFrom: certificate.notBefore },
					{ code: "view_certificate", target: certificateRoute(certificate) },
				)
			: undefined,
);

/** Renewal items stand alone only when the certificate itself isn't expiring yet. */
function standaloneRenewal(
	input: AttentionInputExt,
	certificate: CertificateFacts,
) {
	return expiryState(certificate, input.now)
		? undefined
		: renewalProblem(certificate, input.now);
}

const authorityExpiring = perCertificate(
	"renewal_authority_expiring",
	(input, certificate) => {
		const problem = standaloneRenewal(input, certificate);
		if (
			problem?.renewal !== "authority_expired" &&
			problem?.renewal !== "authority_expiring"
		)
			return undefined;
		return certificateItem(
			"renewal_authority_expiring",
			"warning",
			certificate,
			{ authorityExpiresAt: problem.at ?? 0 },
			{
				code: "install_renewal_authority",
				target: certificateRoute(certificate),
			},
		);
	},
);

const delegationError = perCertificate(
	"renewal_delegation_error",
	(input, certificate) => {
		const problem = standaloneRenewal(input, certificate);
		if (problem?.renewal !== "delegation_error") return undefined;
		return certificateItem(
			"renewal_delegation_error",
			"warning",
			certificate,
			{ message: certificate.issuer?.last_error ?? "" },
			{ code: "fix_renewal", target: certificateRoute(certificate) },
		);
	},
);

const acmeError = perCertificate("acme_error", (input, certificate) => {
	const problem = standaloneRenewal(input, certificate);
	if (problem?.renewal !== "acme_error") return undefined;
	return certificateItem(
		"acme_error",
		"warning",
		certificate,
		{ nextAttemptAt: certificate.acme?.next_attempt_at ?? 0 },
		{ code: "review", target: certificateRoute(certificate) },
	);
});

const acmeStaging = perCertificate(
	"acme_staging_in_use",
	(_input, certificate) =>
		certificate.acme?.environment === "lets_encrypt_staging" &&
		certificate.services > 0
			? certificateItem(
					"acme_staging_in_use",
					"notice",
					certificate,
					{},
					{
						code: "switch_to_production",
						target: certificateRoute(certificate),
					},
				)
			: undefined,
);

const signingRequest = perDevice(
	"signing_request_attention",
	(input, device) => {
		if (!device.active) return undefined;
		const certificates = device.liveInput?.certificates?.certificates ?? [];
		return (device.liveInput?.certificateRequests ?? []).flatMap((request) => {
			const current = certificates.find(
				(entry) => entry.certificate_id === request.certificate_id,
			);
			const stale =
				current !== undefined && current.revision !== request.expected_revision;
			const expiringSoon = request.expires_at - input.now <= REQUEST_EXPIRING_S;
			if (!stale && !expiringSoon) return [];
			return [
				attentionCandidate({
					key: "signing_request_attention",
					severity: "notice",
					subject: {
						kind: "certificate",
						deviceId: device.id,
						certificateId: request.certificate_id,
					},
					params: {
						device: device.name,
						label: request.label,
						reason: stale ? "stale" : "expiring",
						expiresAt: request.expires_at,
					},
					action: {
						code: "finish_or_discard",
						target: deviceRoute(
							device.id,
							"certificates",
							request.certificate_id,
						),
					},
					source:
						device.inspectionSource ??
						classify("certificates_live", { now: input.now, loaded: true }),
					lastKnown: !device.liveOpen,
				}),
			];
		});
	},
);

const inventoryStale = perDevice(
	"certificate_inventory_stale",
	(input, device) => {
		const inventory = input.certInventory[device.id];
		if (!device.active || device.presence.kind !== "online" || !inventory)
			return undefined;
		const updatedAt = inventory.updated_at;
		const liveCount = device.liveInput?.certificates?.certificates.length ?? 0;
		const stale =
			updatedAt === null
				? liveCount > 0
				: input.now - updatedAt > INVENTORY_STALE_S;
		if (!stale) return undefined;
		return attentionCandidate({
			key: "certificate_inventory_stale",
			severity: "notice",
			subject: { kind: "certificate", deviceId: device.id },
			params:
				updatedAt === null
					? { device: device.name }
					: { device: device.name, updatedAt },
			action: {
				code: "connect_live",
				target: { kind: "connect", deviceId: device.id },
			},
			source: classify("cert_inventory", {
				now: input.now,
				at: updatedAt,
				loaded: true,
				noFail: true,
			}),
			since: updatedAt === null ? undefined : updatedAt + INVENTORY_STALE_S,
		});
	},
);

const slotsNearlyFull = perDevice(
	"certificate_slots_nearly_full",
	(input, device) => {
		const used = device.active
			? (device.liveInput?.certificates?.certificates.length ??
				input.certInventory[device.id]?.certificates.length ??
				0)
			: 0;
		if (used < CERTIFICATES_NEARLY_FULL) return undefined;
		return attentionCandidate({
			key: "certificate_slots_nearly_full",
			severity: "notice",
			subject: { kind: "certificate", deviceId: device.id },
			params: { device: device.name, used, max: MAX_CERTIFICATES },
			action: {
				code: "review_certificates",
				target: deviceRoute(device.id, "certificates"),
			},
			source:
				device.inspectionSource ??
				classify("cert_inventory", {
					now: input.now,
					at: input.certInventory[device.id]?.updated_at,
					loaded: true,
					noFail: true,
				}),
		});
	},
);

interface AuthorityFacts {
	authorityId: string;
	label: string;
	issuingNotAfter: number;
	rootNotAfter: number;
}

/** Organisation authorities kept on this computer (summary and public bundles, by id). */
function authoritiesOf(input: AttentionInputExt): AuthorityFacts[] {
	const byId = new Map<string, AuthorityFacts>(
		input.local.authorities.map((entry) => [entry.authorityId, entry]),
	);
	for (const { public_bundle: bundle } of input.authorities)
		if (!byId.has(bundle.authority_id))
			byId.set(bundle.authority_id, {
				authorityId: bundle.authority_id,
				label: bundle.label,
				issuingNotAfter: bundle.issuer_not_after,
				rootNotAfter: bundle.not_after,
			});
	return [...byId.values()];
}

const authoritiesRoute: DevicesRoute = {
	screen: "certificates",
	tab: "authorities",
};

const signingKeyExpiring: AttentionRuleExt = {
	key: "org_ca_signing_key_expiring",
	evaluate(input) {
		return authoritiesOf(input).flatMap((authority) =>
			authority.issuingNotAfter - input.now <= AUTHORITY_EXPIRING_S
				? [
						attentionCandidate({
							key: "org_ca_signing_key_expiring",
							severity:
								authority.issuingNotAfter <= input.now ? "critical" : "warning",
							subject: {
								kind: "authority",
								authorityId: authority.authorityId,
							},
							params: { label: authority.label, at: authority.issuingNotAfter },
							action: { code: "renew_signing_key", target: authoritiesRoute },
							source: localSource(input),
						}),
					]
				: [],
		);
	},
};

const rootExpiring: AttentionRuleExt = {
	key: "org_ca_root_expiring",
	evaluate(input) {
		return authoritiesOf(input).flatMap((authority) =>
			authority.rootNotAfter - input.now <= ROOT_EXPIRING_S
				? [
						attentionCandidate({
							key: "org_ca_root_expiring",
							severity: "warning",
							subject: {
								kind: "authority",
								authorityId: authority.authorityId,
							},
							params: { label: authority.label, at: authority.rootNotAfter },
							action: { code: "plan_replacement", target: authoritiesRoute },
							source: localSource(input),
						}),
					]
				: [],
		);
	},
};

export const CERTIFICATE_RULES: readonly AttentionRuleExt[] = [
	expired,
	expiring,
	notYetValid,
	authorityExpiring,
	delegationError,
	acmeError,
	acmeStaging,
	signingRequest,
	inventoryStale,
	slotsNearlyFull,
	signingKeyExpiring,
	rootExpiring,
];
