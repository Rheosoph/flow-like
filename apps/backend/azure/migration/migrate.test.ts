import { describe, expect, test } from "bun:test";
import {
	ConfigError,
	type Environment,
	composeDatabaseUrl,
	parseConfig,
} from "./migrate";

const CLIENT_ID = "11111111-2222-4333-8444-555555555555";

function validSettings(): Environment {
	return {
		AZURE_POSTGRES_AUTH_MODE: "managed_identity",
		AZURE_POSTGRES_HOST: "flow-like-dev.flow-like.postgres.database.azure.com",
		AZURE_POSTGRES_DATABASE: "flow_like",
		AZURE_POSTGRES_USER: "flowlike-dev-migration-identity",
		AZURE_CLIENT_ID: CLIENT_ID,
		IDENTITY_ENDPOINT: "http://localhost:42356/msi/token",
		IDENTITY_HEADER: "11111111-2222-4333-8444-555555555555",
	};
}

describe("parseConfig", () => {
	test("accepts only managed-identity configuration", () => {
		const config = parseConfig(validSettings());
		expect(config.clientId).toBe(CLIENT_ID);
		expect(config.database).toBe("flow_like");
		expect(config.user).toBe("flowlike-dev-migration-identity");
	});

	test("rejects passwords and connection strings even when empty", () => {
		for (const forbidden of [
			"DATABASE_URL",
			"PGPASSWORD",
			"AZURE_POSTGRES_PASSWORD",
		]) {
			const env = { ...validSettings(), [forbidden]: "" };
			expect(() => parseConfig(env)).toThrow(
				new RegExp(`^${forbidden} is forbidden`),
			);
		}
	});

	test("rejects non-azure or ambiguous hosts", () => {
		for (const host of [
			"database.example.com",
			"HTTPS://server.postgres.database.azure.com",
			"server.postgres.database.azure.com:5432",
			"-server.postgres.database.azure.com",
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), AZURE_POSTGRES_HOST: host }),
			).toThrow(ConfigError);
		}
	});

	test("rejects static auth mode and missing client id", () => {
		expect(() =>
			parseConfig({ ...validSettings(), AZURE_POSTGRES_AUTH_MODE: "password" }),
		).toThrow(ConfigError);

		const { AZURE_CLIENT_ID: _omitted, ...withoutClientId } = validSettings();
		expect(() => parseConfig(withoutClientId)).toThrow(/AZURE_CLIENT_ID/);
	});

	test("rejects non-local identity endpoint and alternate credential sources", () => {
		expect(() =>
			parseConfig({
				...validSettings(),
				IDENTITY_ENDPOINT: "https://identity.example.com/token",
			}),
		).toThrow(ConfigError);

		expect(() =>
			parseConfig({
				...validSettings(),
				IMDS_ENDPOINT: "http://localhost:1234",
			}),
		).toThrow(/^IMDS_ENDPOINT is forbidden/);

		expect(() =>
			parseConfig({
				...validSettings(),
				HTTPS_PROXY: "https://proxy.example.com",
			}),
		).toThrow(/^HTTPS_PROXY is forbidden/);
	});
});

describe("composeDatabaseUrl", () => {
	test("uses the token as password with quaint's verify-full spelling", () => {
		const config = parseConfig(validSettings());
		const url = new URL(composeDatabaseUrl(config, "eyJ.header/payload+sig="));

		expect(url.protocol).toBe("postgresql:");
		expect(url.username).toBe("flowlike-dev-migration-identity");
		expect(decodeURIComponent(url.password)).toBe("eyJ.header/payload+sig=");
		expect(url.host).toBe(
			"flow-like-dev.flow-like.postgres.database.azure.com:5432",
		);
		expect(url.pathname).toBe("/flow_like");
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("strict");
		expect(url.searchParams.get("connect_timeout")).toBe("15");
	});
});
