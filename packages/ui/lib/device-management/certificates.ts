import { z } from "zod";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import type { ManagementCall } from "./telemetry";

export const CERTIFICATE_WARNING_SECONDS = 7 * 24 * 60 * 60;
export const MAX_CERTIFICATE_PEM_BYTES = 12 * 1024;
const revision = z.number().int().nonnegative().safe();
const timestamp = z.number().int().safe();
const certificateId = z.string().uuid();
export const certificateMetadataSchema = z.object({
	certificate_id: certificateId,
	label: z.string().min(1).max(128),
	revision: revision.refine((value) => value > 0),
	subject: z.string().max(4096),
	issuer: z.string().max(4096),
	dns_names: z.array(z.string().max(253)).max(256),
	ip_addresses: z.array(z.string().max(45)).max(256),
	sha256_fingerprint: z.string().regex(/^[a-f0-9]{64}$/),
	not_before: timestamp,
	not_after: timestamp,
	bindings: z
		.array(
			z.object({
				placement_id: z.string().min(1).max(128),
				project_id: z.string().min(1).max(128),
				service: z.string().min(1).max(128),
			}),
		)
		.max(1024),
	binding_count: z.number().int().nonnegative().safe().optional(),
});
const inventorySchema = z.object({
	certificates: z.array(certificateMetadataSchema).max(32),
	inventory_revision: revision,
});
const pageSchema = inventorySchema.extend({ next: certificateId.nullable() });
const publicInventorySchema = z.object({
	revision,
	updated_at: timestamp.nullable(),
	certificates: z
		.array(
			z.object({
				certificate_id: certificateId,
				revision: revision,
				fingerprint_sha256: z.string().regex(/^[a-f0-9]{64}$/),
				not_after: timestamp,
			}),
		)
		.max(32),
});
export type DeviceCertificate = z.infer<typeof certificateMetadataSchema>;
export type CertificateInventory = z.infer<typeof inventorySchema>;
export type PublicCertificateInventory = z.infer<typeof publicInventorySchema>;

export function certificateStatus(
	certificate: { not_after: number; not_before?: number },
	now = Math.floor(Date.now() / 1000),
): "expired" | "not_yet_valid" | "expiring" | "valid" {
	if (certificate.not_after <= now) return "expired";
	if (certificate.not_before !== undefined && certificate.not_before > now)
		return "not_yet_valid";
	return certificate.not_after - now <= CERTIFICATE_WARNING_SECONDS
		? "expiring"
		: "valid";
}

export function certificateWarnings(
	certificates: { not_after: number }[],
	now?: number,
) {
	return certificates.reduce(
		(counts, certificate) => {
			const status = certificateStatus(certificate, now);
			if (status === "expired" || status === "expiring") counts[status]++;
			return counts;
		},
		{ expired: 0, expiring: 0 },
	);
}

export async function readCertificates(
	call: ManagementCall,
): Promise<CertificateInventory> {
	try {
		for (let restart = 0; restart < 3; restart++) {
			let after: string | undefined;
			let inventoryRevision: number | undefined;
			const certificates: DeviceCertificate[] = [];
			const seen = new Set<string>();
			for (let page = 0; page < 64; page++) {
				const response = await call({
					type: "certificates",
					limit: 4,
					...(after ? { after } : {}),
				});
				if (response.state !== "completed") throw new Error();
				const parsed = pageSchema.parse(response.result);
				if (
					inventoryRevision !== undefined &&
					parsed.inventory_revision !== inventoryRevision
				)
					break;
				inventoryRevision = parsed.inventory_revision;
				for (const certificate of parsed.certificates) {
					if (seen.has(certificate.certificate_id)) throw new Error();
					seen.add(certificate.certificate_id);
					certificates.push(certificate);
				}
				if (certificates.length > 32) throw new Error();
				if (!parsed.next)
					return { certificates, inventory_revision: inventoryRevision };
				if (parsed.next === after || !parsed.certificates.length)
					throw new Error();
				after = parsed.next;
			}
		}
		throw new Error();
	} catch {
		throw new Error(
			"Certificates could not be read. Check your device access and reconnect.",
		);
	}
}

export async function readPublicCertificates(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<PublicCertificateInventory> {
	try {
		return publicInventorySchema.parse(
			await api.get(
				profile,
				`devices/${encodeURIComponent(deviceId)}/certificate-inventory`,
			),
		);
	} catch {
		throw new Error("Certificate expiry information is unavailable.");
	}
}

export function validateCertificateImport(
	label: string,
	chain: string,
	key: string,
): void {
	if (
		!label.trim() ||
		new TextEncoder().encode(label.trim()).length > 128 ||
		/\p{Cc}/u.test(label)
	)
		throw new Error(
			"Use a certificate label of up to 128 UTF-8 bytes without control characters.",
		);
	if (
		new TextEncoder().encode(chain).length +
			new TextEncoder().encode(key).length >
		MAX_CERTIFICATE_PEM_BYTES
	)
		throw new Error(
			"The certificate chain and private key must total at most 12 KiB.",
		);
	if (
		!/^\s*-----BEGIN CERTIFICATE-----[\s\S]+-----END CERTIFICATE-----\s*$/u.test(
			chain,
		) ||
		/PRIVATE KEY/u.test(chain)
	)
		throw new Error(
			"Choose a PEM certificate chain, starting with the service certificate.",
		);
	if (
		!/^\s*-----BEGIN (PRIVATE KEY|RSA PRIVATE KEY|EC PRIVATE KEY)-----[\s\S]+-----END \1-----\s*$/u.test(
			key,
		) ||
		/ENCRYPTED/u.test(key)
	)
		throw new Error(
			"Choose an unencrypted PEM private key matching the certificate.",
		);
}

export async function putCertificate(
	call: ManagementCall,
	input: {
		certificateId: string;
		label: string;
		expectedRevision: number;
		chain: string;
		key: string;
	},
): Promise<DeviceCertificate> {
	validateCertificateImport(input.label, input.chain, input.key);
	try {
		certificateId.parse(input.certificateId);
		revision.parse(input.expectedRevision);
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "put_certificate",
				certificate_id: input.certificateId,
				label: input.label.trim(),
				expected_revision: input.expectedRevision,
				certificate_chain_pem: input.chain,
				private_key_pem: input.key,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const value = certificateMetadataSchema.parse(response.result.certificate);
		if (
			value.certificate_id !== input.certificateId ||
			value.revision !== input.expectedRevision + 1
		)
			throw new Error();
		return value;
	} catch {
		// Never render transport or parser errors containing the submitted key.
		throw new Error(
			"Certificate import was not confirmed. Refresh the list before retrying. Check the PEM pair, certificate validity, permissions and revision.",
		);
	}
}

export async function deleteCertificate(
	call: ManagementCall,
	certificate: DeviceCertificate,
): Promise<void> {
	if (certificate.bindings.length || certificate.binding_count)
		throw new Error(
			"Remove this certificate from every service before deleting it.",
		);
	try {
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "delete_certificate",
				certificate_id: certificate.certificate_id,
				expected_revision: certificate.revision,
			},
			operationId,
		);
		if (
			response.state !== "completed" ||
			response.operation_id !== operationId ||
			response.result.deleted !== true ||
			response.result.certificate_id !== certificate.certificate_id
		)
			throw new Error();
	} catch {
		throw new Error(
			"Certificate deletion was not confirmed. Refresh the list and remove any service assignments before retrying.",
		);
	}
}
