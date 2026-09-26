import { expect, test } from "bun:test";
import {
	configureAcmeCertificate,
	readAcmeCertificates,
} from "./certificate-acme";
import {
	createCertificateRequest,
	createCertificateIssuerRequest,
	installCertificateRequest,
	installCertificateIssuer,
	readCertificateRequests,
	readCertificateIssuers,
	type CertificateRequest,
} from "./certificate-issuance";
import type { ManagementCall } from "./telemetry";

const id = "00000000-0000-4000-8000-000000000001";
const other = "00000000-0000-4000-8000-000000000002";
const request: CertificateRequest = {
	request_id: id,
	certificate_id: id,
	label: "Gateway",
	expected_revision: 3,
	dns_names: ["edge.example.com"],
	ip_addresses: [],
	csr_pem: "public CSR",
	created_at: 100,
	expires_at: 200,
	purpose: "service",
};
const cert = {
	certificate_id: id,
	label: "Gateway",
	revision: 4,
	subject: "CN=edge.example.com",
	issuer: "CN=Org",
	dns_names: request.dns_names,
	ip_addresses: [],
	sha256_fingerprint: "a".repeat(64),
	not_before: 100,
	not_after: 200,
	bindings: [],
};
const chain = "-----BEGIN CERTIFICATE-----\nYWI=\n-----END CERTIFICATE-----";
const response = (result: Record<string, unknown>, operation_id = "read") => ({
	state: "completed" as const,
	operation_id,
	result,
});

test("device CSR creation binds acknowledgements and never sends a private key", async () => {
	const calls: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command, operation) => {
		calls.push(command);
		return response(
			{ request: { ...request, request_id: command.request_id } },
			operation,
		);
	};
	const result = await createCertificateRequest(call, {
		certificateId: id,
		label: "Gateway",
		expectedRevision: 3,
		dnsNames: request.dns_names,
		ipAddresses: [],
	});
	expect(result.purpose).toBe("service");
	expect(calls[0].type).toBe("create_certificate_request");
	expect(calls[0].expected_revision).toBe(3);
	expect(calls[0]).not.toHaveProperty("private_key_pem");
	await expect(
		createCertificateRequest(
			async (_, operation) =>
				response({ request: { ...request, certificate_id: other } }, operation),
			{
				certificateId: id,
				label: "Gateway",
				expectedRevision: 3,
				dnsNames: request.dns_names,
				ipAddresses: [],
			},
		),
	).rejects.toThrow("not confirmed");
});

test("pending requests reject cursor overlap and strip unexpected private fields", async () => {
	const result = await readCertificateRequests(async () =>
		response({
			requests: [{ ...request, private_key_pem: "never display" }],
			next: null,
		}),
	);
	expect(JSON.stringify(result)).not.toContain("never display");
	await expect(
		readCertificateRequests(async () =>
			response({ requests: [request], next: id }),
		),
	).rejects.toThrow("could not be read");
});

test("signed service installation rejects issuer requests, key material and incorrect certificate identity", async () => {
	let calls = 0;
	const call: ManagementCall = async (_, operation) => {
		calls++;
		return response({ certificate: cert }, operation);
	};
	await expect(
		installCertificateRequest(call, { ...request, purpose: "issuer" }, chain),
	).rejects.toThrow("renewal delegation");
	await expect(
		installCertificateRequest(call, request, `${chain}\nPRIVATE KEY`),
	).rejects.toThrow("PEM certificate");
	expect(calls).toBe(0);
	expect((await installCertificateRequest(call, request, chain)).revision).toBe(
		4,
	);
	await expect(
		installCertificateRequest(
			async (_, operation) =>
				response(
					{ certificate: { ...cert, certificate_id: other } },
					operation,
				),
			request,
			chain,
		),
	).rejects.toThrow("not installed");
});

test("issuer requests preserve owner-approved names and leaf lifetime but policy revision is independent of leaf revision", async () => {
	const issuerRequest = {
		...request,
		purpose: "issuer" as const,
		leaf_lifetime_days: 30,
	};
	const result = await createCertificateIssuerRequest(
		async (command, operation) =>
			response(
				{ request: { ...issuerRequest, request_id: command.request_id } },
				operation,
			),
		{
			certificateId: id,
			expectedRevision: 3,
			dnsNames: request.dns_names,
			ipAddresses: [],
			leafLifetimeDays: 30,
		},
	);
	expect(result.expected_revision).toBe(3);
	const issuer = {
		certificate_id: id,
		revision: 1,
		dns_names: request.dns_names,
		ip_addresses: [],
		leaf_lifetime_days: 30,
		not_after: 200,
		last_renewed_at: null,
		next_renewal_at: 150,
		last_error: null,
	};
	expect(
		await installCertificateIssuer(
			async (_, operation) => response({ issuer }, operation),
			issuerRequest,
			chain,
		),
	).toEqual(issuer);
	await expect(
		installCertificateIssuer(
			async () => {
				throw new Error("PRIVATE KEY secret");
			},
			issuerRequest,
			chain,
		),
	).rejects.toThrow("Renewal delegation was not installed");
	await expect(
		readCertificateIssuers(async () =>
			response({ issuers: [issuer, issuer], next: null }),
		),
	).rejects.toThrow("could not be read");
});

test("ACME configuration requires explicit terms consent and verifies acknowledgements after re-enabling", async () => {
	const input = {
		certificateId: id,
		label: "Gateway",
		expectedRevision: 0,
		expectedCertificateRevision: 4,
		dnsNames: request.dns_names,
		environment: "lets_encrypt_staging" as const,
		httpBind: "0.0.0.0:80",
		termsAgreed: false,
	};
	let calls = 0;
	const policy = {
		certificate_id: id,
		label: "Gateway",
		revision: 7,
		dns_names: request.dns_names,
		environment: input.environment,
		http_bind: input.httpBind,
		next_attempt_at: 100,
		last_renewed_at: null,
		last_error: null,
	};
	const call: ManagementCall = async (command, operation) => {
		calls++;
		expect(command.terms_of_service_agreed).toBe(true);
		expect(command.expected_certificate_revision).toBe(4);
		return response({ acme: policy }, operation);
	};
	await expect(configureAcmeCertificate(call, input)).rejects.toThrow("Accept");
	expect(calls).toBe(0);
	expect(
		(await configureAcmeCertificate(call, { ...input, termsAgreed: true }))
			.revision,
	).toBe(7);
	await expect(
		configureAcmeCertificate(
			async (_, operation) =>
				response({ acme: { ...policy, certificate_id: other } }, operation),
			{ ...input, termsAgreed: true },
		),
	).rejects.toThrow("not confirmed");
});

test("ACME listing fails closed for inconsistent pagination and strips account secrets", async () => {
	const policy = {
		certificate_id: id,
		label: "Gateway",
		revision: 1,
		dns_names: request.dns_names,
		environment: "lets_encrypt_production",
		http_bind: "0.0.0.0:80",
		next_attempt_at: 100,
		last_renewed_at: null,
		last_error: null,
		account_key: "secret",
	};
	expect(
		JSON.stringify(
			await readAcmeCertificates(async () =>
				response({ policies: [policy], next: null }),
			),
		),
	).not.toContain("secret");
	await expect(
		readAcmeCertificates(async () =>
			response({ policies: [policy], next: other }),
		),
	).rejects.toThrow("could not be read");
});
