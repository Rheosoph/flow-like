import { expect, test } from "bun:test";
import {
	CERTIFICATE_WARNING_SECONDS,
	certificateStatus,
	certificateWarnings,
	deleteCertificate,
	putCertificate,
	readCertificates,
	validateCertificateImport,
	type DeviceCertificate,
} from "./certificates";
import type { ManagementCall } from "./telemetry";

const id = "00000000-0000-4000-8000-000000000001";
const secondId = "00000000-0000-4000-8000-000000000002";
const certificate: DeviceCertificate = {
	certificate_id: id,
	label: "Gateway",
	revision: 2,
	subject: "CN=gateway.test",
	issuer: "CN=Local issuer",
	dns_names: ["gateway.test"],
	ip_addresses: ["127.0.0.1"],
	sha256_fingerprint: "a".repeat(64),
	not_before: 100,
	not_after: 200,
	bindings: [],
};
const chain = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----";
const key =
	"-----BEGIN PRIVATE KEY-----\nsecret-material\n-----END PRIVATE KEY-----";
const response = (
	result: Record<string, unknown>,
	operationId = "operation",
) => ({
	operation_id: operationId,
	state: "completed" as const,
	result,
});

test("certificate expiry covers the exact seven-day and expiration boundaries", () => {
	expect(
		certificateStatus({ not_after: 100 + CERTIFICATE_WARNING_SECONDS }, 100),
	).toBe("expiring");
	expect(
		certificateStatus({ not_after: 101 + CERTIFICATE_WARNING_SECONDS }, 100),
	).toBe("valid");
	expect(certificateStatus({ not_after: 100 }, 100)).toBe("expired");
	expect(certificateStatus({ not_after: 200, not_before: 101 }, 100)).toBe(
		"not_yet_valid",
	);
	expect(
		certificateWarnings(
			[
				{ not_after: 99 },
				{ not_after: 101 },
				{ not_after: 100 + CERTIFICATE_WARNING_SECONDS + 1 },
			],
			100,
		),
	).toEqual({ expired: 1, expiring: 1 });
});

test("certificate listing retries an inventory revision change and strips unexpected secret fields", async () => {
	const commands: Record<string, unknown>[] = [];
	const pages = [
		{ certificates: [certificate], inventory_revision: 1, next: id },
		{
			certificates: [{ ...certificate, certificate_id: secondId }],
			inventory_revision: 2,
			next: null,
		},
		{
			certificates: [{ ...certificate, private_key_pem: key }],
			inventory_revision: 2,
			next: id,
		},
		{
			certificates: [{ ...certificate, certificate_id: secondId }],
			inventory_revision: 2,
			next: null,
		},
	];
	const call: ManagementCall = async (command) => {
		commands.push(command);
		return response(pages.shift()!);
	};
	const actual = await readCertificates(call);
	expect(actual.certificates).toHaveLength(2);
	expect(actual.inventory_revision).toBe(2);
	expect(commands).toEqual([
		{ type: "certificates", limit: 4 },
		{ type: "certificates", limit: 4, after: id },
		{ type: "certificates", limit: 4 },
		{ type: "certificates", limit: 4, after: id },
	]);
	expect(JSON.stringify(actual)).not.toContain("secret-material");
});

test("certificate listing rejects duplicate pages and hides invalid response contents", async () => {
	await expect(
		readCertificates(async () =>
			response({
				certificates: [certificate],
				inventory_revision: 1,
				next: id,
			}),
		),
	).rejects.toThrow("Certificates could not be read");
	await expect(
		readCertificates(async () => response({ certificates: [key] })),
	).rejects.toThrow("Certificates could not be read");
});

test("import uses the device management channel with revision guards and never returns PEM", async () => {
	const commands: Record<string, unknown>[] = [];
	const result = await putCertificate(
		async (command, operationId) => {
			commands.push(command);
			return response(
				{ certificate: { ...certificate, private_key_pem: key } },
				operationId,
			);
		},
		{ certificateId: id, expectedRevision: 1, label: "Gateway", chain, key },
	);
	expect(commands).toEqual([
		{
			type: "put_certificate",
			certificate_id: id,
			expected_revision: 1,
			label: "Gateway",
			certificate_chain_pem: chain,
			private_key_pem: key,
		},
	]);
	expect(result).toEqual(certificate);
});

test("import errors hide submitted key material and reject incorrect acknowledgements", async () => {
	const input = {
		certificateId: id,
		expectedRevision: 1,
		label: "Gateway",
		chain,
		key,
	};
	const invalidCalls: ManagementCall[] = [
		async () => {
			throw new Error(key);
		},
		async (_, operationId) =>
			response(
				{ certificate: { ...certificate, certificate_id: secondId } },
				operationId,
			),
		async (_, operationId) =>
			response({ certificate: { ...certificate, revision: 3 } }, operationId),
		async () => response({ certificate }),
	];
	for (const call of invalidCalls) {
		try {
			await putCertificate(call, input);
			throw new Error("Unexpected success");
		} catch (error) {
			expect(String(error)).toContain("Certificate import was not confirmed");
			expect(String(error)).not.toContain("secret-material");
		}
	}
});

test("PEM inputs are bounded and unsupported or mixed material never reaches transport", () => {
	expect(() => validateCertificateImport("Gateway", chain, key)).not.toThrow();
	expect(() => validateCertificateImport("Gateway", chain + key, key)).toThrow(
		"PEM certificate chain",
	);
	expect(() =>
		validateCertificateImport(
			"Gateway",
			chain,
			key.replaceAll("PRIVATE KEY", "ENCRYPTED PRIVATE KEY"),
		),
	).toThrow("unencrypted");
	expect(() =>
		validateCertificateImport("Gateway", chain + "a".repeat(12288), key),
	).toThrow("12 KiB");
});

test("deletion blocks assigned certificates and checks the returned identity", async () => {
	let calls = 0;
	const call: ManagementCall = async (_, operationId) => {
		calls++;
		return response({ certificate_id: id, deleted: true }, operationId);
	};
	await expect(
		deleteCertificate(call, {
			...certificate,
			bindings: [
				{ placement_id: "placement", project_id: "project", service: "https" },
			],
		}),
	).rejects.toThrow("Remove this certificate");
	expect(calls).toBe(0);
	await deleteCertificate(call, certificate);
	expect(calls).toBe(1);
	await expect(
		deleteCertificate(
			async (_, operationId) =>
				response({ certificate_id: secondId, deleted: true }, operationId),
			certificate,
		),
	).rejects.toThrow("deletion was not confirmed");
	await expect(
		deleteCertificate(call, { ...certificate, binding_count: 1 }),
	).rejects.toThrow("Remove this certificate");
});
