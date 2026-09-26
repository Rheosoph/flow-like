import { expect, test } from "bun:test";
import {
	authorityEnvelopeSchema,
	certificateAuthorityBackup,
	createLocalCertificateAuthority,
	restoreCertificateAuthority,
	signCertificateRequest,
	renewLocalCertificateAuthority,
	type CertificateAuthorityEnvelope,
} from "./certificate-authority";
import { accountStorageKey } from "./storage";
import type { DeviceCrypto } from "./types";

const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const id = "00000000-0000-4000-8000-000000000001";
const envelope: CertificateAuthorityEnvelope = {
	public_bundle: {
		account_binding: accountStorageKey(scope),
		authority_id: id,
		label: "Organisation",
		dns_suffixes: ["example.com"],
		ip_addresses: [],
		root_certificate_pem: "root",
		issuer_certificate_pem: "issuer",
		sha256_fingerprint: "a".repeat(64),
		not_before: 100,
		not_after: 200,
		issuer_not_after: 180,
	},
	vault: Array(80).fill(1),
	root_vault: Array(80).fill(2),
};
const local = {
	public_bundle: envelope.public_bundle,
	vault: Uint8Array.from(envelope.vault),
};
const backup = JSON.stringify({ version: 1, ...envelope });
const signing = {
	csr_pem: "csr",
	dns_names: ["edge.example.com"],
	ip_addresses: [],
	validity_days: 30,
};

test("authority creation binds encryption to the account, uses an independent password and zeroizes password bytes", async () => {
	let passwordBytes: Uint8Array | undefined;
	const crypto = {
		createCertificateAuthorityVault(
			spec: Record<string, unknown>,
			password: Uint8Array,
		) {
			expect(spec.account_binding).toBe(accountStorageKey(scope));
			expect(new TextDecoder().decode(password)).toBe("authority password");
			passwordBytes = password;
			return envelope;
		},
	} as unknown as DeviceCrypto;
	await createLocalCertificateAuthority(
		scope,
		{
			label: "Org",
			dns_suffixes: ["example.com"],
			ip_addresses: [],
			validity_days: 365,
		},
		"authority password",
		crypto,
	);
	expect(passwordBytes?.every((value) => value === 0)).toBe(true);
	await expect(
		createLocalCertificateAuthority(
			scope,
			{ label: "Org", dns_suffixes: [], ip_addresses: [], validity_days: 365 },
			"short",
			crypto,
		),
	).rejects.toThrow("12 characters");
});

test("encrypted root backup is exportable but restore retains only the issuing vault", async () => {
	const crypto = {
		inspectCertificateAuthorityBackup() {
			return envelope.public_bundle;
		},
	} as unknown as DeviceCrypto;
	const restored = await restoreCertificateAuthority(
		scope,
		backup,
		"password",
		crypto,
	);
	expect(restored).toEqual(local);
	expect(restored).not.toHaveProperty("root_vault");
	expect(
		JSON.parse(await certificateAuthorityBackup(envelope).text()).root_vault,
	).toEqual(envelope.root_vault);
});

test("restoring an authority rejects another account or tampered metadata before it can be saved", async () => {
	let calls = 0;
	const crypto = {
		inspectCertificateAuthorityBackup() {
			calls++;
			return envelope.public_bundle;
		},
	} as unknown as DeviceCrypto;
	await expect(
		restoreCertificateAuthority(
			{ ...scope, account: "other" },
			backup,
			"password",
			crypto,
		),
	).rejects.toThrow("account and hub");
	expect(calls).toBe(0);
	const tampered = JSON.stringify({
		version: 1,
		...envelope,
		public_bundle: {
			...envelope.public_bundle,
			dns_suffixes: ["attacker.test"],
		},
	});
	await expect(
		restoreCertificateAuthority(scope, tampered, "password", crypto),
	).rejects.toThrow("could not be unlocked");
});

test("service and renewal signing use only the issuing vault and scope-check before decrypting", async () => {
	const seen: string[] = [];
	const make =
		(kind: string) =>
		(
			binding: string,
			authorityId: string,
			password: Uint8Array,
			vault: Uint8Array,
		) => {
			seen.push(kind);
			expect(binding).toBe(accountStorageKey(scope));
			expect(authorityId).toBe(id);
			expect([...vault]).toEqual(envelope.vault);
			expect(password.length).toBeGreaterThan(0);
			return {
				certificate_chain_pem: "chain",
				not_before: 100,
				not_after: 150,
			};
		};
	const crypto = {
		signServiceCertificate: make("service"),
		signDeviceCertificateIssuer: make("issuer"),
	} as unknown as DeviceCrypto;
	await signCertificateRequest(scope, local, "password", signing, crypto);
	await signCertificateRequest(
		scope,
		local,
		"password",
		signing,
		crypto,
		"issuer",
	);
	expect(seen).toEqual(["service", "issuer"]);
	await expect(
		signCertificateRequest(
			{ ...scope, apiOrigin: "https://other.test" },
			local,
			"password",
			signing,
			crypto,
		),
	).rejects.toThrow("could not be signed");
	expect(seen).toHaveLength(2);
});

test("authority errors never render private material from cryptography failures", async () => {
	const crypto = {
		signServiceCertificate() {
			throw new Error("-----BEGIN PRIVATE KEY-----SECRET");
		},
	} as unknown as DeviceCrypto;
	try {
		await signCertificateRequest(scope, local, "password", signing, crypto);
		throw new Error("Unexpected success");
	} catch (error) {
		expect(String(error)).toContain("could not be signed");
		expect(String(error)).not.toContain("SECRET");
	}
});

test("issuer renewal requires the matching offline root backup", async () => {
	let calls = 0;
	const crypto = {
		renewCertificateAuthorityVault(
			_binding: string,
			_id: string,
			_password: Uint8Array,
			_issuer: Uint8Array,
			root: Uint8Array,
		) {
			calls++;
			expect([...root]).toEqual(envelope.root_vault);
			return envelope;
		},
	} as unknown as DeviceCrypto;
	expect(
		await renewLocalCertificateAuthority(
			scope,
			local,
			backup,
			"password",
			crypto,
		),
	).toEqual(envelope);
	await expect(
		renewLocalCertificateAuthority(
			scope,
			local,
			JSON.stringify({
				version: 1,
				...envelope,
				public_bundle: {
					...envelope.public_bundle,
					root_certificate_pem: "different root",
				},
			}),
			"password",
			crypto,
		),
	).rejects.toThrow("could not be renewed");
	expect(calls).toBe(1);
	expect(() =>
		authorityEnvelopeSchema.parse({ ...envelope, root_vault: [1] }),
	).toThrow();
});
