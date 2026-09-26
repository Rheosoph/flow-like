import { z } from "zod";
import { withPassword } from "./crypto";
import { accountStorageKey, type DeviceAccountScope } from "./storage";
import type { DeviceCrypto } from "./types";

const boundedPem = z.string().min(1).max(16_384);
export const authorityPublicSchema = z.object({
	account_binding: z.string().min(1).max(4096),
	authority_id: z.string().uuid(),
	label: z
		.string()
		.min(1)
		.refine((value) => new TextEncoder().encode(value).length <= 64),
	dns_suffixes: z.array(z.string().min(1).max(253)).max(32),
	ip_addresses: z.array(z.string().min(1).max(45)).max(32),
	root_certificate_pem: boundedPem,
	issuer_certificate_pem: boundedPem,
	sha256_fingerprint: z.string().regex(/^[a-f0-9]{64}$/u),
	not_before: z.number().int().safe(),
	not_after: z.number().int().safe(),
	issuer_not_after: z.number().int().safe(),
});
const encryptedBytes = z
	.array(z.number().int().min(0).max(255))
	.min(64)
	.max(65_600);
export const authorityEnvelopeSchema = z.object({
	public_bundle: authorityPublicSchema,
	vault: encryptedBytes,
	root_vault: encryptedBytes,
});
export type CertificateAuthorityPublic = z.infer<typeof authorityPublicSchema>;
export type CertificateAuthorityEnvelope = z.infer<
	typeof authorityEnvelopeSchema
>;
export interface CertificateAuthoritySpec {
	account_binding: string;
	authority_id: string;
	label: string;
	dns_suffixes: string[];
	ip_addresses: string[];
	validity_days: number;
}
export interface CertificateAuthoritySigningRequest {
	csr_pem: string;
	dns_names: string[];
	ip_addresses: string[];
	validity_days: number;
}
export interface SignedCertificateChain {
	certificate_chain_pem: string;
	not_before: number;
	not_after: number;
}
export interface LocalCertificateAuthority {
	public_bundle: CertificateAuthorityPublic;
	vault: Uint8Array;
}

export function certificateAuthorityBackup(
	envelope: CertificateAuthorityEnvelope,
): Blob {
	return new Blob(
		[
			JSON.stringify({
				version: 1,
				...authorityEnvelopeSchema.parse(envelope),
			}),
		],
		{ type: "application/json" },
	);
}

export async function restoreCertificateAuthority(
	scope: DeviceAccountScope,
	text: string,
	password: string,
	crypto: DeviceCrypto,
): Promise<LocalCertificateAuthority> {
	try {
		if (new TextEncoder().encode(text).length > 1024 * 1024) throw new Error();
		const parsed = z
			.object({ version: z.literal(1) })
			.merge(authorityEnvelopeSchema)
			.parse(JSON.parse(text));
		if (parsed.public_bundle.account_binding !== accountStorageKey(scope))
			throw new Error();
		const publicBundle = await withPassword(password, (bytes) =>
			crypto.inspectCertificateAuthorityBackup(
				accountStorageKey(scope),
				parsed.public_bundle.authority_id,
				bytes,
				Uint8Array.from(parsed.vault),
				Uint8Array.from(parsed.root_vault),
			),
		);
		const verified = authorityPublicSchema.parse(publicBundle);
		if (JSON.stringify(verified) !== JSON.stringify(parsed.public_bundle))
			throw new Error();
		return { public_bundle: verified, vault: Uint8Array.from(parsed.vault) };
	} catch {
		throw new Error(
			"The authority backup could not be unlocked for this account and hub. Check the backup and its password.",
		);
	}
}

export async function createLocalCertificateAuthority(
	scope: DeviceAccountScope,
	spec: Omit<CertificateAuthoritySpec, "account_binding" | "authority_id">,
	password: string,
	crypto: DeviceCrypto,
): Promise<CertificateAuthorityEnvelope> {
	if (password.length < 12)
		throw new Error("Use at least 12 characters for the authority password.");
	try {
		const result = await withPassword(password, (bytes) =>
			crypto.createCertificateAuthorityVault(
				{
					...spec,
					account_binding: accountStorageKey(scope),
					authority_id: globalThis.crypto.randomUUID(),
				},
				bytes,
				Math.floor(Date.now() / 1000),
			),
		);
		const envelope = authorityEnvelopeSchema.parse(result);
		if (envelope.public_bundle.account_binding !== accountStorageKey(scope))
			throw new Error();
		return envelope;
	} catch {
		throw new Error(
			"The certificate authority could not be created. Check its label, DNS suffixes, IP addresses and validity.",
		);
	}
}

export async function signCertificateRequest(
	scope: DeviceAccountScope,
	authority: LocalCertificateAuthority,
	password: string,
	request: CertificateAuthoritySigningRequest,
	crypto: DeviceCrypto,
	kind: "service" | "issuer" = "service",
): Promise<SignedCertificateChain> {
	try {
		const accountBinding = accountStorageKey(scope);
		if (authority.public_bundle.account_binding !== accountBinding)
			throw new Error();
		const result = await withPassword(password, (bytes) =>
			(kind === "issuer"
				? crypto.signDeviceCertificateIssuer
				: crypto.signServiceCertificate)(
				accountBinding,
				authority.public_bundle.authority_id,
				bytes,
				authority.vault,
				request,
				Math.floor(Date.now() / 1000),
			),
		);
		return z
			.object({
				certificate_chain_pem: boundedPem,
				not_before: z.number().int().safe(),
				not_after: z.number().int().safe(),
			})
			.parse(result);
	} catch {
		throw new Error(
			"The request could not be signed. Check the authority password, approved names, expiry and requested lifetime.",
		);
	}
}

export async function renewLocalCertificateAuthority(
	scope: DeviceAccountScope,
	authority: LocalCertificateAuthority,
	backupText: string,
	password: string,
	crypto: DeviceCrypto,
): Promise<CertificateAuthorityEnvelope> {
	try {
		if (new TextEncoder().encode(backupText).length > 1024 * 1024)
			throw new Error();
		const backup = z
			.object({ version: z.literal(1) })
			.merge(authorityEnvelopeSchema)
			.parse(JSON.parse(backupText));
		if (
			backup.public_bundle.account_binding !== accountStorageKey(scope) ||
			backup.public_bundle.authority_id !==
				authority.public_bundle.authority_id ||
			backup.public_bundle.root_certificate_pem !==
				authority.public_bundle.root_certificate_pem
		)
			throw new Error();
		const result = await withPassword(password, (bytes) =>
			crypto.renewCertificateAuthorityVault(
				accountStorageKey(scope),
				authority.public_bundle.authority_id,
				bytes,
				authority.vault,
				Uint8Array.from(backup.root_vault),
				Math.floor(Date.now() / 1000),
			),
		);
		return authorityEnvelopeSchema.parse(result);
	} catch {
		throw new Error(
			"The issuing authority could not be renewed. Choose this authority's encrypted root backup and check its password and root expiry.",
		);
	}
}
