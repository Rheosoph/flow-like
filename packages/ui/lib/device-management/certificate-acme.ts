import { z } from "zod";
import type { ManagementCall } from "./telemetry";

const id = z.string().uuid();
const revision = z.number().int().nonnegative().safe();
const timestamp = z.number().int().safe();
const environment = z.enum(["lets_encrypt_staging", "lets_encrypt_production"]);
export const acmeCertificateSchema = z.object({
	certificate_id: id,
	label: z.string().min(1).max(128),
	revision,
	dns_names: z.array(z.string().min(1).max(253)).min(1).max(64),
	environment,
	http_bind: z.string().min(1).max(256),
	next_attempt_at: timestamp,
	last_renewed_at: timestamp.nullable(),
	last_error: z.string().max(4096).nullable(),
});
export type AcmeCertificate = z.infer<typeof acmeCertificateSchema>;
export interface ConfigureAcmeCertificate {
	certificateId: string;
	label: string;
	expectedRevision: number;
	expectedCertificateRevision: number;
	dnsNames: string[];
	environment: AcmeCertificate["environment"];
	httpBind: string;
	termsAgreed: boolean;
}

export async function readAcmeCertificates(
	call: ManagementCall,
): Promise<AcmeCertificate[]> {
	try {
		let after: string | undefined;
		const values: AcmeCertificate[] = [];
		for (let page = 0; page < 32; page++) {
			const response = await call({
				type: "acme_certificates",
				limit: 4,
				...(after ? { after } : {}),
			});
			if (response.state !== "completed") throw new Error();
			const parsed = z
				.object({
					policies: z.array(acmeCertificateSchema).max(4),
					next: id.nullable(),
				})
				.parse(response.result);
			for (const value of parsed.policies) {
				if (after && value.certificate_id <= after) throw new Error();
				after = value.certificate_id;
				values.push(value);
			}
			if (values.length > 32) throw new Error();
			if (!parsed.next) return values;
			if (parsed.policies.at(-1)?.certificate_id !== parsed.next)
				throw new Error();
		}
		throw new Error();
	} catch {
		throw new Error(
			"Automatic public certificate settings could not be read. Owner access is required.",
		);
	}
}

export async function configureAcmeCertificate(
	call: ManagementCall,
	input: ConfigureAcmeCertificate,
): Promise<AcmeCertificate> {
	if (!input.termsAgreed)
		throw new Error(
			"Accept the certificate authority's terms before requesting a public certificate.",
		);
	try {
		id.parse(input.certificateId);
		revision.parse(input.expectedRevision);
		revision.parse(input.expectedCertificateRevision);
		environment.parse(input.environment);
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "configure_acme_certificate",
				certificate_id: input.certificateId,
				label: input.label.trim(),
				expected_revision: input.expectedRevision,
				expected_certificate_revision: input.expectedCertificateRevision,
				dns_names: input.dnsNames,
				environment: input.environment,
				http_bind: input.httpBind.trim(),
				terms_of_service_agreed: true,
			},
			operationId,
		);
		if (response.state !== "completed" || response.operation_id !== operationId)
			throw new Error();
		const value = acmeCertificateSchema.parse(response.result.acme);
		if (
			value.certificate_id !== input.certificateId ||
			value.revision <= input.expectedRevision
		)
			throw new Error();
		return value;
	} catch {
		throw new Error(
			"Public certificate configuration was not confirmed. Refresh settings and check the names, listener address and current revisions.",
		);
	}
}

export async function deleteAcmeCertificate(
	call: ManagementCall,
	certificate: AcmeCertificate,
): Promise<void> {
	try {
		const operationId = crypto.randomUUID();
		const response = await call(
			{
				type: "delete_acme_certificate",
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
			"Public certificate renewal was not stopped. Refresh settings before retrying.",
		);
	}
}
