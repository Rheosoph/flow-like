import { z } from "zod";
import type { ManagementCall } from "./telemetry";
import { managementRejection } from "./types";

export interface ServiceTunnelListener {
	id: string;
	host: string;
	port: number;
	protocol: "tcp" | "http" | "https";
	tls_server_name?: string;
}

/** This read reveals connection targets without exposing deployment settings. */
export async function readServiceListeners(
	call: ManagementCall,
	placementId: string,
	signal?: AbortSignal,
): Promise<ServiceTunnelListener[]> {
	const identifier = z.string().regex(/^[A-Za-z0-9._:-]{1,128}$/);
	identifier.parse(placementId);
	signal?.throwIfAborted();
	const response = await call({
		type: "service_listeners",
		placement_id: placementId,
	});
	signal?.throwIfAborted();
	if (response.state !== "completed") {
		const rejection = managementRejection(response);
		throw new Error(
			rejection?.error ??
				"The device did not return service listeners. Connecting requires service_connect access to this placement.",
		);
	}
	if (
		new TextEncoder().encode(JSON.stringify(response.result)).length >
		16 * 1024
	)
		throw new Error("Device service listeners exceed their response limit.");
	const result = z
		.object({
			placement_id: z.literal(placementId),
			project_id: identifier,
			config_revision: z.number().int().positive().safe(),
			services: z
				.array(
					z
						.object({
							id: identifier,
							host: z.string().ip(),
							port: z.number().int().min(1).max(65535),
							protocol: z.enum(["tcp", "http", "https"]),
							tls_server_name: z
								.string()
								.min(1)
								.max(253)
								.refine(serverName)
								.optional(),
						})
						.strict(),
				)
				.max(17),
		})
		.strict()
		.parse(response.result);
	if (
		new Set(result.services.map((service) => service.id)).size !==
			result.services.length ||
		result.services.some(
			(service) => service.id !== "hosting" && !loopback(service.host),
		)
	)
		throw new Error("Device service listeners are invalid.");
	return result.services;
}

function loopback(host: string): boolean {
	if (z.string().ip({ version: "v6" }).safeParse(host).success)
		return new URL(`http://[${host}]/`).hostname === "[::1]";
	const parts = host.split(".");
	return (
		parts.length === 4 &&
		parts[0] === "127" &&
		parts.every(
			(part) => /^(0|[1-9]\d{0,2})$/.test(part) && Number(part) <= 255,
		)
	);
}

function serverName(name: string): boolean {
	if (z.string().ip().safeParse(name).success) return true;
	const labels = (name.endsWith(".") ? name.slice(0, -1) : name).split(".");
	return (
		labels.every(
			(label) =>
				label.length <= 63 &&
				/^[A-Za-z0-9_](?:[A-Za-z0-9_-]*[A-Za-z0-9_])?$/.test(label),
		) && !/^\d+$/.test(labels.at(-1) ?? "")
	);
}

export const tunnelServiceSchema = z
	.object({
		id: z
			.string()
			.regex(/^[A-Za-z0-9._-]{1,128}$/)
			.refine(
				(id) => !["hosting", ".", ".."].includes(id),
				"The hosting service is configured separately.",
			),
		host: z
			.string()
			.refine(
				loopback,
				"Use a device loopback address, such as 127.0.0.1 or ::1.",
			),
		port: z.number().int().min(1).max(65535),
		protocol: z.enum(["tcp", "http", "https"]),
		tls_server_name: z
			.string()
			.min(1)
			.max(253)
			.refine(serverName, "Use a TLS DNS name or IP address without a port.")
			.nullish(),
		tls_sha256_fingerprint: z
			.string()
			.regex(/^[a-f0-9]{64}$/)
			.nullish(),
	})
	.strict()
	.refine(
		(value) =>
			value.protocol === "https"
				? !!value.tls_server_name && !!value.tls_sha256_fingerprint
				: value.tls_server_name == null && value.tls_sha256_fingerprint == null,
		"HTTPS needs a TLS server name and the server certificate's SHA-256 fingerprint.",
	);

export const tunnelServicesSchema = z
	.array(tunnelServiceSchema)
	.max(16)
	.refine(
		(values) => new Set(values.map((value) => value.id)).size === values.length,
		"Each listener needs a unique service ID.",
	);
export type TunnelService = z.infer<typeof tunnelServiceSchema>;

export function configuredTunnelServices(config: {
	hosting?: { host: string; port: number } | null;
	tls_certificate_id?: string | null;
	tunnel_services?: unknown;
}): ServiceTunnelListener[] {
	const additional = tunnelServicesSchema.parse(
		config.tunnel_services === undefined ? [] : config.tunnel_services,
	);
	return [
		...(config.hosting
			? [
					{
						id: "hosting",
						host: config.hosting.host,
						port: config.hosting.port,
						protocol: config.tls_certificate_id
							? ("https" as const)
							: ("http" as const),
					},
				]
			: []),
		...additional.map(({ id, host, port, protocol, tls_server_name }) => ({
			id,
			host,
			port,
			protocol,
			...(tls_server_name ? { tls_server_name } : {}),
		})),
	];
}

/** One header per line. Keeping credentials out of URLs avoids accidental link sharing. */
export function parseTunnelHeaders(text: string): Record<string, string> {
	if (new TextEncoder().encode(text).length > 32 * 1024)
		throw new Error("Request headers exceed 32 KiB.");
	const headers: Record<string, string> = Object.create(null);
	for (const line of text.split(/\r?\n/)) {
		if (!line.trim()) continue;
		const split = line.indexOf(":");
		if (split < 1) throw new Error("Enter each header as Name: value.");
		const name = line.slice(0, split).trim().toLowerCase();
		const value = line.slice(split + 1).trim();
		if (
			!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(name) ||
			Array.from(value).some(
				(character) =>
					character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
			) ||
			Object.hasOwn(headers, name)
		)
			throw new Error(
				"Request headers contain an invalid or repeated name or value.",
			);
		headers[name] = value;
	}
	return headers;
}
