import { z } from "zod";
import {
	MAX_CERTIFICATE_PEM_BYTES,
	certificateMetadataSchema,
	type DeviceCertificate,
} from "./certificates";
import type { ManagementCall } from "./telemetry";

const id = z.string().uuid();
const revision = z.number().int().nonnegative().safe();
const timestamp = z.number().int().safe();
export const certificateRequestSchema = z.object({
	request_id: id,
	certificate_id: id,
	label: z.string().min(1).max(128),
	expected_revision: revision,
	dns_names: z.array(z.string().min(1).max(253)).max(64),
	ip_addresses: z.array(z.string().min(1).max(45)).max(64),
	csr_pem: z.string().min(1).max(MAX_CERTIFICATE_PEM_BYTES),
	created_at: timestamp,
	expires_at: timestamp,
	purpose: z.enum(["service", "issuer"]),
	leaf_lifetime_days: z.number().int().min(1).max(397).optional(),
});
export type CertificateRequest = z.infer<typeof certificateRequestSchema>;
export const certificateIssuerSchema = z.object({
	certificate_id: id,
	revision,
	dns_names: z.array(z.string().min(1).max(253)).max(64),
	ip_addresses: z.array(z.string().min(1).max(45)).max(64),
	leaf_lifetime_days: z.number().int().min(1).max(397),
	not_after: timestamp,
	last_renewed_at: timestamp.nullable(),
	next_renewal_at: timestamp,
	last_error: z.string().max(4096).nullable(),
});
export type CertificateIssuer = z.infer<typeof certificateIssuerSchema>;

export function certificateNames(value: string): string[] {
	return [...new Set(value.split(/[\s,]+/u).filter(Boolean))];
}

export async function readCertificateRequests(
	call: ManagementCall,
): Promise<CertificateRequest[]> {
	try {
		const requests: CertificateRequest[] = [];
		let after: string | undefined;
		for (let page = 0; page < 32; page++) {
			const response = await call({
				type: "certificate_requests",
				limit: 1,
				...(after ? { after } : {}),
			});
			if (response.state !== "completed") throw new Error();
			const parsed = z
				.object({
					requests: z.array(certificateRequestSchema).max(1),
					next: id.nullable(),
				})
				.parse(response.result);
			for (const request of parsed.requests) {
				if (after && request.request_id <= after) throw new Error();
				requests.push(request);
			}
			if (!parsed.next) return requests;
			if (parsed.requests.at(-1)?.request_id !== parsed.next) throw new Error();
			after = parsed.next;
		}
		throw new Error();
	} catch {
		throw new Error(
			"Certificate requests could not be read. Reconnect and refresh.",
		);
	}
}

export async function createCertificateRequest(
	call: ManagementCall,
	input: {
		certificateId: string;
		label: string;
		expectedRevision: number;
		dnsNames: string[];
		ipAddresses: string[];
	},
): Promise<CertificateRequest> {
	if (
		!input.label.trim() ||
		new TextEncoder().encode(input.label.trim()).length > 128 ||
		/\p{Cc}/u.test(input.label) ||
		(!input.dnsNames.length && !input.ipAddresses.length)
	)
		throw new Error(
			"Enter a certificate label and at least one DNS name or IP address.",
		);
	try {
		id.parse(input.certificateId);
		revision.parse(input.expectedRevision);
		const requestId = crypto.randomUUID();
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "create_certificate_request",
				certificate_id: input.certificateId,
				request_id: requestId,
				label: input.label.trim(),
				expected_revision: input.expectedRevision,
				dns_names: input.dnsNames,
				ip_addresses: input.ipAddresses,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const request = certificateRequestSchema.parse(response.result.request);
		if (
			request.purpose !== "service" ||
			request.request_id !== requestId ||
			request.certificate_id !== input.certificateId ||
			request.expected_revision !== input.expectedRevision
		)
			throw new Error();
		return request;
	} catch {
		throw new Error(
			"Device key generation was not confirmed. Refresh requests before retrying. Check names, permissions and the current certificate revision.",
		);
	}
}

export async function installCertificateRequest(
	call: ManagementCall,
	request: CertificateRequest,
	chain: string,
): Promise<DeviceCertificate> {
	if (request.purpose !== "service")
		throw new Error(
			"Use renewal delegation to install an issuing certificate.",
		);
	if (
		new TextEncoder().encode(chain).length > MAX_CERTIFICATE_PEM_BYTES ||
		!/^[\s]*-----BEGIN CERTIFICATE-----[\s\S]+-----END CERTIFICATE-----\s*$/u.test(
			chain,
		) ||
		/PRIVATE KEY/u.test(chain)
	)
		throw new Error(
			"Choose a PEM certificate chain of at most 12 KiB, starting with the signed service certificate.",
		);
	try {
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "install_certificate_request",
				request_id: request.request_id,
				certificate_chain_pem: chain,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const certificate = certificateMetadataSchema.parse(
			response.result.certificate,
		);
		if (
			certificate.certificate_id !== request.certificate_id ||
			certificate.revision !== request.expected_revision + 1
		)
			throw new Error();
		return certificate;
	} catch {
		throw new Error(
			"The signed certificate was not installed. Refresh requests and certificates. The chain must match this device key and the exact requested names.",
		);
	}
}

export async function readCertificateIssuers(
	call: ManagementCall,
): Promise<CertificateIssuer[]> {
	try {
		let after: string | undefined;
		const issuers: CertificateIssuer[] = [];
		for (let page = 0; page < 32; page++) {
			const response = await call({
				type: "certificate_issuers",
				limit: 4,
				...(after ? { after } : {}),
			});
			if (response.state !== "completed") throw new Error();
			const parsed = z
				.object({
					issuers: z.array(certificateIssuerSchema).max(4),
					next: id.nullable(),
				})
				.parse(response.result);
			for (const issuer of parsed.issuers) {
				if (after && issuer.certificate_id <= after) throw new Error();
				after = issuer.certificate_id;
				issuers.push(issuer);
			}
			if (issuers.length > 32) throw new Error();
			if (!parsed.next) return issuers;
			if (parsed.issuers.at(-1)?.certificate_id !== parsed.next)
				throw new Error();
		}
		throw new Error();
	} catch {
		throw new Error(
			"Renewal policies could not be read. Owner access is required.",
		);
	}
}

export async function createCertificateIssuerRequest(
	call: ManagementCall,
	input: {
		certificateId: string;
		expectedRevision: number;
		dnsNames: string[];
		ipAddresses: string[];
		leafLifetimeDays: number;
	},
): Promise<CertificateRequest> {
	try {
		id.parse(input.certificateId);
		revision.parse(input.expectedRevision);
		const requestId = crypto.randomUUID();
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "create_certificate_issuer_request",
				request_id: requestId,
				certificate_id: input.certificateId,
				expected_revision: input.expectedRevision,
				dns_names: input.dnsNames,
				ip_addresses: input.ipAddresses,
				leaf_lifetime_days: input.leafLifetimeDays,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const request = certificateRequestSchema.parse(response.result.request);
		if (
			request.purpose !== "issuer" ||
			request.certificate_id !== input.certificateId ||
			request.request_id !== requestId ||
			request.expected_revision !== input.expectedRevision ||
			request.leaf_lifetime_days !== input.leafLifetimeDays
		)
			throw new Error();
		return request;
	} catch {
		throw new Error(
			"Renewal delegation was not prepared. Refresh policies and requests before retrying. Owner access is required.",
		);
	}
}

export async function installCertificateIssuer(
	call: ManagementCall,
	request: CertificateRequest,
	chain: string,
): Promise<CertificateIssuer> {
	if (request.purpose !== "issuer")
		throw new Error("Select an issuing certificate request.");
	try {
		if (
			new TextEncoder().encode(chain).length > MAX_CERTIFICATE_PEM_BYTES ||
			/PRIVATE KEY/u.test(chain)
		)
			throw new Error();
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "install_certificate_issuer",
				request_id: request.request_id,
				certificate_chain_pem: chain,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const issuer = certificateIssuerSchema.parse(response.result.issuer);
		if (issuer.certificate_id !== request.certificate_id || issuer.revision < 1)
			throw new Error();
		return issuer;
	} catch {
		throw new Error(
			"Renewal delegation was not installed. Check the signed chain, exact approved names, current policy revision and owner access.",
		);
	}
}

export async function deleteCertificateIssuer(
	call: ManagementCall,
	issuer: CertificateIssuer,
): Promise<void> {
	try {
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "delete_certificate_issuer",
				certificate_id: issuer.certificate_id,
				expected_revision: issuer.revision,
			},
			operationId,
		);
		if (
			response.state !== "completed" ||
			response.operation_id !== operationId ||
			response.result.certificate_id !== issuer.certificate_id ||
			response.result.deleted !== true
		)
			throw new Error();
	} catch {
		throw new Error(
			"Renewal delegation removal was not confirmed. Refresh policies before retrying.",
		);
	}
}

export async function deleteCertificateRequest(
	call: ManagementCall,
	request: CertificateRequest,
): Promise<void> {
	try {
		const operationId = crypto.randomUUID();
		const response = await call(
			{ type: "delete_certificate_request", request_id: request.request_id },
			operationId,
		);
		if (
			response.state !== "completed" ||
			response.operation_id !== operationId ||
			response.result.request_id !== request.request_id ||
			response.result.deleted !== true
		)
			throw new Error();
	} catch {
		throw new Error(
			"Certificate request deletion was not confirmed. Refresh before retrying.",
		);
	}
}
