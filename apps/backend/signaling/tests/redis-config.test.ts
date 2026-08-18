import { describe, expect, test } from "bun:test";
import { parseRedisConfig } from "../redis";

const CLIENT_ID = "12345678-1234-4123-8123-123456789abc";

describe("Redis configuration", () => {
	test("preserves local Redis defaults", () => {
		expect(parseRedisConfig({})).toEqual({
			authMode: "local",
			url: "redis://127.0.0.1:6379",
			hostname: "127.0.0.1",
			clusterMode: false,
		});
	});

	test("accepts a keyless TLS Azure Managed Redis endpoint", () => {
		expect(
			parseRedisConfig({
				REDIS_AUTH_MODE: "azure-entra",
				REDIS_URL: "rediss://flow-like.westeurope.redis.azure.net:10000",
				AZURE_CLIENT_ID: CLIENT_ID,
			}),
		).toMatchObject({
			authMode: "azure-entra",
			hostname: "flow-like.westeurope.redis.azure.net",
			azureClientId: CLIENT_ID,
			clusterMode: false,
		});
	});

	test.each([
		["plaintext transport", "redis://flow-like.westeurope.redis.azure.net:10000"],
		[
			"password in URL",
			"rediss://default:secret@flow-like.westeurope.redis.azure.net:10000",
		],
		["IP address", "rediss://10.0.0.4:10000"],
		["untrusted host", "rediss://redis.attacker.example:10000"],
	])("rejects %s in Azure mode", (_label, url) => {
		expect(() =>
			parseRedisConfig({
				REDIS_AUTH_MODE: "azure-entra",
				REDIS_URL: url,
				AZURE_CLIENT_ID: CLIENT_ID,
			}),
		).toThrow();
	});

	test("rejects access keys and service-principal secrets in Azure mode", () => {
		expect(() =>
			parseRedisConfig({
				REDIS_AUTH_MODE: "azure-entra",
				REDIS_URL: "rediss://flow-like.westeurope.redis.azure.net:10000",
				AZURE_CLIENT_ID: CLIENT_ID,
				REDIS_PASSWORD: "forbidden",
				AZURE_CLIENT_SECRET: "also-forbidden",
			}),
		).toThrow(/static credential variables/);
	});

	test("rejects a credential-chain fallback in Azure mode", () => {
		expect(() =>
			parseRedisConfig({
				REDIS_AUTH_MODE: "azure-entra",
				REDIS_URL: "rediss://flow-like.westeurope.redis.azure.net:10000",
				AZURE_CLIENT_ID: CLIENT_ID,
				AZURE_TOKEN_CREDENTIALS: "prod",
			}),
		).toThrow(/ManagedIdentityCredential/);
	});
});
