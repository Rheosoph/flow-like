import { isIP } from "node:net";
import { DefaultAzureCredential } from "@azure/identity";
import {
	type RedisClientType,
	type RedisClusterType,
	createClient,
	createCluster,
} from "@redis/client";
import {
	EntraIdCredentialsProviderFactory,
	REDIS_SCOPE_DEFAULT,
} from "@redis/entraid";

export type RedisAuthMode = "local" | "azure-entra";

// `local` keeps every message inside this process; `redis` republishes through
// Redis so sibling replicas can deliver to their own sockets.
export type FanoutMode = "local" | "redis";

export type SignalFanoutConfig =
	| { mode: "local" }
	| { mode: "redis"; redis: SignalRedisConfig };

export type SignalRedisConfig = {
	authMode: RedisAuthMode;
	url: string;
	hostname: string;
	clusterMode: boolean;
	azureClientId?: string;
	azureHostSuffix?: string;
};

type Environment = Record<string, string | undefined>;

// Keep the signaling service independent from node-redis' large generic type
// surface. Both the standalone and cluster clients implement this exact subset.
export type SignalRedisClient = {
	readonly isOpen: boolean;
	readonly isReady: boolean;
	connect(): Promise<unknown>;
	ping(): Promise<string>;
	publish(channel: string, message: string): Promise<number>;
	subscribe(
		channels: string | string[],
		listener: (message: string, channel: string) => unknown,
	): Promise<void>;
	hVals(key: string): Promise<string[]>;
	hSet(key: string, field: string, value: string): Promise<number>;
	hDel(key: string, field: string): Promise<number>;
	expire(key: string, seconds: number): Promise<boolean>;
	close(): Promise<void>;
	destroy(): void;
	on(event: string, listener: (...args: any[]) => void): SignalRedisClient;
};

const AZURE_AUTH_MODE = "azure-entra";
const DEFAULT_AZURE_REDIS_HOST_SUFFIX = ".redis.azure.net";
const UUID_PATTERN =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function parseBoolean(value: string | undefined, name: string): boolean {
	if (value === undefined || value.trim() === "") return false;
	if (value.trim().toLowerCase() === "true") return true;
	if (value.trim().toLowerCase() === "false") return false;
	throw new Error(`${name} must be either true or false`);
}

function assertNoAzureStaticCredentials(env: Environment): void {
	const forbidden = [
		"REDIS_PASSWORD",
		"REDIS_USERNAME",
		"REDIS_ACCESS_KEY",
		"AZURE_CLIENT_SECRET",
		"AZURE_CLIENT_CERTIFICATE_PATH",
		"AZURE_CLIENT_CERTIFICATE_PASSWORD",
	];
	const configured = forbidden.filter((name) => Boolean(env[name]?.trim()));
	if (configured.length > 0) {
		throw new Error(
			`Azure Redis mode forbids static credential variables: ${configured.join(", ")}`,
		);
	}
}

function normalizeAzureHostSuffix(value: string | undefined): string {
	const suffix = (value || DEFAULT_AZURE_REDIS_HOST_SUFFIX)
		.trim()
		.toLowerCase();
	if (!/^\.[a-z0-9.-]+$/.test(suffix) || suffix.includes("..")) {
		throw new Error(
			"AZURE_REDIS_HOST_SUFFIX must be a DNS suffix beginning with a dot",
		);
	}
	return suffix;
}

export function parseRedisConfig(
	env: Environment = process.env,
): SignalRedisConfig {
	const authMode = (env.REDIS_AUTH_MODE || "local").trim().toLowerCase();
	if (authMode !== "local" && authMode !== AZURE_AUTH_MODE) {
		throw new Error("REDIS_AUTH_MODE must be local or azure-entra");
	}

	const rawUrl = (env.REDIS_URL || "redis://127.0.0.1:6379").trim();
	let parsed: URL;
	try {
		parsed = new URL(rawUrl);
	} catch {
		throw new Error("REDIS_URL must be a valid redis:// or rediss:// URL");
	}
	if (parsed.protocol !== "redis:" && parsed.protocol !== "rediss:") {
		throw new Error("REDIS_URL must use the redis:// or rediss:// scheme");
	}
	if (!parsed.hostname) throw new Error("REDIS_URL must include a hostname");

	const clusterMode = parseBoolean(
		env.REDIS_CLUSTER_MODE,
		"REDIS_CLUSTER_MODE",
	);
	if (authMode === "local") {
		return {
			authMode,
			url: rawUrl,
			hostname: parsed.hostname,
			clusterMode,
		};
	}

	assertNoAzureStaticCredentials(env);
	if (!env.REDIS_URL?.trim()) {
		throw new Error("REDIS_URL is required when REDIS_AUTH_MODE=azure-entra");
	}
	if (parsed.protocol !== "rediss:") {
		throw new Error("Azure Redis requires TLS; REDIS_URL must use rediss://");
	}
	if (/^rediss:\/\/[^/@]*@/i.test(rawUrl)) {
		throw new Error("Azure Redis REDIS_URL must not contain user information");
	}
	if (parsed.username || parsed.password) {
		throw new Error("Azure Redis REDIS_URL must not contain credentials");
	}
	if (parsed.search || parsed.hash) {
		throw new Error(
			"Azure Redis REDIS_URL must not contain query or fragment data",
		);
	}
	if (
		parsed.pathname !== "" &&
		parsed.pathname !== "/" &&
		parsed.pathname !== "/0"
	) {
		throw new Error("Azure Redis supports only database 0");
	}
	if (isIP(parsed.hostname)) {
		throw new Error(
			"Azure Redis REDIS_URL must use its DNS hostname for TLS validation",
		);
	}

	const azureClientId = env.AZURE_CLIENT_ID?.trim();
	if (!azureClientId || !UUID_PATTERN.test(azureClientId)) {
		throw new Error(
			"AZURE_CLIENT_ID must be the user-assigned managed identity client UUID",
		);
	}

	const azureTokenCredentials = env.AZURE_TOKEN_CREDENTIALS?.trim();
	if (
		azureTokenCredentials &&
		azureTokenCredentials.toLowerCase() !== "managedidentitycredential"
	) {
		throw new Error(
			"AZURE_TOKEN_CREDENTIALS must be ManagedIdentityCredential in Azure Redis mode",
		);
	}

	const azureHostSuffix = normalizeAzureHostSuffix(env.AZURE_REDIS_HOST_SUFFIX);
	const hostname = parsed.hostname.toLowerCase();
	if (
		!hostname.endsWith(azureHostSuffix) ||
		hostname.length <= azureHostSuffix.length
	) {
		throw new Error(
			`Azure Redis hostname must end with the configured ${azureHostSuffix} suffix`,
		);
	}

	return {
		authMode,
		url: rawUrl,
		hostname,
		clusterMode,
		azureClientId,
		azureHostSuffix,
	};
}

export function parseFanoutConfig(
	env: Environment = process.env,
): SignalFanoutConfig {
	const mode = (env.REALTIME_FANOUT_MODE || "redis").trim().toLowerCase();
	if (mode !== "local" && mode !== "redis") {
		throw new Error("REALTIME_FANOUT_MODE must be local or redis");
	}
	if (mode === "local") {
		return { mode };
	}

	if (!env.REDIS_URL?.trim()) {
		throw new Error("REALTIME_FANOUT_MODE=redis requires REDIS_URL");
	}
	return { mode, redis: parseRedisConfig(env) };
}

function reconnectDelay(retries: number): number {
	const exponential = Math.min(250 * 2 ** Math.min(retries, 7), 30_000);
	return Math.round(exponential * (0.8 + Math.random() * 0.4));
}

function azureCredentialsProvider(config: SignalRedisConfig, role: string) {
	if (!config.azureClientId) {
		throw new Error("Azure Redis managed identity was not configured");
	}

	// Restrict DefaultAzureCredential to managed identity. This prevents an
	// accidentally injected service-principal secret or developer login from
	// becoming an authentication fallback in the Azure runtime.
	process.env.AZURE_TOKEN_CREDENTIALS = "ManagedIdentityCredential";
	const credential = new DefaultAzureCredential({
		managedIdentityClientId: config.azureClientId,
		requiredEnvVars: ["AZURE_CLIENT_ID", "AZURE_TOKEN_CREDENTIALS"],
	});

	return EntraIdCredentialsProviderFactory.createForDefaultAzureCredential({
		credential,
		scopes: REDIS_SCOPE_DEFAULT,
		options: {},
		tokenManagerConfig: {
			// Refresh with 30% of the token lifetime left. The streaming provider
			// sends AUTH again on RESP3 connections, including pub/sub sockets.
			expirationRefreshRatio: 0.7,
			retry: {
				maxAttempts: 6,
				initialDelayMs: 500,
				maxDelayMs: 15_000,
				backoffMultiplier: 2,
				jitterPercentage: 20,
			},
		},
		onRetryableError: () => {
			console.warn(`[Redis:${role}] Microsoft Entra token refresh will retry`);
		},
		onReAuthenticationError: () => {
			console.error(
				`[Redis:${role}] Microsoft Entra reauthentication failed; no access-key fallback is permitted`,
			);
		},
	});
}

function mapClusterAddress(config: SignalRedisConfig, address: string) {
	const separator = address.lastIndexOf(":");
	if (separator < 1) return undefined;
	const advertisedHost = address.slice(0, separator).replace(/^\[|\]$/g, "");
	const advertisedPort = Number(address.slice(separator + 1));
	if (!Number.isInteger(advertisedPort) || advertisedPort < 1) return undefined;

	// Azure private endpoints can expose IP addresses in cluster topology. Route
	// those IPs through the configured hostname so TLS SNI/certificate validation
	// remains intact while preserving the server-advertised shard port.
	if (isIP(advertisedHost)) {
		return { host: config.hostname, port: advertisedPort };
	}
	return undefined;
}

function asSignalClient(
	client: RedisClientType<{}, {}, {}, 3> | RedisClusterType<{}, {}, {}, 3>,
): SignalRedisClient {
	return client as unknown as SignalRedisClient;
}

export function createSignalRedisClient(
	config: SignalRedisConfig,
	role: string,
): SignalRedisClient {
	const isAzure = config.authMode === AZURE_AUTH_MODE;
	const credentialsProvider = isAzure
		? azureCredentialsProvider(config, role)
		: undefined;
	const secureSocket = isAzure
		? {
				tls: true as const,
				servername: config.hostname,
				rejectUnauthorized: true,
				connectTimeout: 15_000,
				keepAlive: true,
				keepAliveInitialDelay: 30_000,
				reconnectStrategy: reconnectDelay,
			}
		: undefined;

	if (config.clusterMode) {
		return asSignalClient(
			createCluster({
				RESP: 3,
				rootNodes: [{ url: config.url }],
				defaults: {
					credentialsProvider,
					socket: secureSocket,
					disableOfflineQueue: isAzure,
					commandsQueueMaxLength: 1_000,
					pingInterval: 30_000,
				},
				minimizeConnections: true,
				nodeAddressMap: isAzure
					? (address) => mapClusterAddress(config, address)
					: undefined,
			}),
		);
	}

	return asSignalClient(
		createClient({
			RESP: 3,
			url: config.url,
			credentialsProvider,
			socket: secureSocket,
			disableOfflineQueue: isAzure,
			commandsQueueMaxLength: 1_000,
			pingInterval: 30_000,
		}),
	);
}

export function attachRedisLifecycleLogging(
	client: SignalRedisClient,
	role: string,
): void {
	client.on("error", (error: unknown) => {
		const message =
			error instanceof Error ? error.message : "unknown Redis error";
		console.error(`[Redis:${role}] ${message}`);
	});
	client.on("reconnecting", () => {
		console.warn(`[Redis:${role}] reconnecting`);
	});
}

export async function closeSignalRedisClient(
	client: SignalRedisClient | null,
): Promise<void> {
	if (!client) return;
	if (!client.isOpen) {
		client.destroy();
		return;
	}
	await client.close();
}
