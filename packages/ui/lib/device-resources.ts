import { z } from "zod";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";

const integer = z.number().int().safe().nonnegative();
const identifier = z
	.string()
	.min(1)
	.max(128)
	.regex(/^[A-Za-z0-9_.-]+$/)
	.refine((value) => value !== "." && value !== "..");
const expiry = integer.max(253402300799);
const status = z.enum(["active", "revoked"]);
const resourceSchema = z.object({
	grant_id: identifier,
	device_id: identifier,
	placement_id: identifier,
	deployment_id: identifier,
	project_id: identifier,
	app_id: identifier.nullable(),
	delegating_user_id: z.string(),
	authz_version: integer.positive(),
	model_ids: z.array(identifier).max(64),
	online_access: z.enum(["read_only", "read_write"]).nullable().optional(),
	max_instances: integer.min(1).max(100),
	expires_at: expiry,
	status,
});
const billingSchema = z.object({
	billing_grant_id: identifier,
	grant_id: identifier,
	payer_id: z.string(),
	authz_version: integer.positive(),
	limit_micros: integer.positive(),
	used_micros: integer,
	reserved_micros: integer,
	expires_at: expiry,
	status,
});
const instanceSchema = z.object({
	instance_id: identifier,
	purpose: z.enum(["workload", "rollout_validation"]).default("workload"),
	device_id: identifier,
	grant_id: identifier,
	billing_grant_id: identifier.nullable().optional(),
	registered_at: expiry,
	lease_expires_at: expiry,
});
export type ResourceGrant = z.infer<typeof resourceSchema>;
export type BillingGrant = z.infer<typeof billingSchema>;
export type InstanceLease = z.infer<typeof instanceSchema>;
export interface DeviceResources {
	grants: ResourceGrant[];
	billing: BillingGrant[];
	instances: InstanceLease[];
}
export interface OfflinePlacementIdentity {
	placement_id: string;
	deployment_id: string;
	project_id: string;
	app_id?: string | null;
}
export interface CreateResourceGrant extends OfflinePlacementIdentity {
	app_id: string | null;
	online_access?: "read_only" | "read_write";
	model_ids: string[];
	max_instances: number;
	expires_at: number;
}
export const MAX_BILLING_MICROS = 1_000_000_000_000;
const MAX_GRANT_SECONDS = 365 * 24 * 60 * 60;

/** Select only public identity. Placement variables and filesystem paths are discarded. */
export function importOfflinePlacement(text: string): OfflinePlacementIdentity {
	if (new TextEncoder().encode(text).length > 1024 * 1024)
		throw new Error("Placement files must be at most 1 MiB.");
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		throw new Error("Choose a valid placement JSON file.");
	}
	const parsed = z
		.object({
			id: identifier,
			deployment_id: identifier,
			project_id: identifier,
			source: z.literal("offline"),
		})
		.safeParse(value);
	if (!parsed.success)
		throw new Error(
			"Choose an offline placement with valid placement, deployment, and project IDs.",
		);
	return {
		placement_id: parsed.data.id,
		deployment_id: parsed.data.deployment_id,
		project_id: parsed.data.project_id,
	};
}

export function importResourcePlacement(
	text: string,
): OfflinePlacementIdentity {
	if (new TextEncoder().encode(text).length > 1024 * 1024)
		throw new Error("Placement files must be at most 1 MiB.");
	const value = z
		.object({
			id: identifier,
			deployment_id: identifier,
			project_id: identifier,
			source: z.enum(["online", "offline"]),
		})
		.parse(JSON.parse(text));
	return {
		placement_id: value.id,
		deployment_id: value.deployment_id,
		project_id: value.project_id,
		app_id: value.source === "online" ? value.project_id : null,
	};
}

export function createResourceRequest(
	placement: OfflinePlacementIdentity,
	models: string,
	maxInstances: string,
	expiresAt: string,
	now = Math.floor(Date.now() / 1000),
	onlineAccess?: "read_only" | "read_write",
): CreateResourceGrant {
	if (
		![
			placement.placement_id,
			placement.deployment_id,
			placement.project_id,
		].every((id) => identifier.safeParse(id).success)
	)
		throw new Error(
			"The placement identity is invalid. Import the placement again.",
		);
	const model_ids = models.split(/[\s,]+/).filter(Boolean);
	if (
		(model_ids.length < 1 && !onlineAccess) ||
		model_ids.length > 64 ||
		model_ids.some((id) => !identifier.safeParse(id).success) ||
		new Set(model_ids).size !== model_ids.length
	)
		throw new Error(
			"Enter 1 to 64 distinct, exact model Bit IDs. Wildcards are not supported.",
		);
	if (
		!/^\d+$/.test(maxInstances) ||
		Number(maxInstances) < 1 ||
		Number(maxInstances) > 100
	)
		throw new Error("The instance limit must be a whole number from 1 to 100.");
	const expires_at = parseExpiry(expiresAt, now, now + MAX_GRANT_SECONDS);
	if (onlineAccess && !placement.app_id)
		throw new Error(
			"Online resource access requires an online project you own.",
		);
	return {
		placement_id: placement.placement_id,
		deployment_id: placement.deployment_id,
		project_id: placement.project_id,
		app_id: placement.app_id ?? null,
		...(onlineAccess ? { online_access: onlineAccess } : {}),
		model_ids,
		max_instances: Number(maxInstances),
		expires_at,
	};
}

export function parseExpiry(
	value: string,
	now: number,
	maximum: number,
): number {
	const seconds = Date.parse(value) / 1000;
	if (!Number.isSafeInteger(seconds) || seconds <= now || seconds > maximum)
		throw new Error("Choose a future expiry within the allowed period.");
	return seconds;
}

/** Decimal text is converted with integer arithmetic, never rounded into consent. */
export function eurosToMicros(value: string): number {
	const match = /^(\d+)(?:\.(\d{1,6}))?$/.exec(value.trim());
	if (!match || value.length > 32)
		throw new Error(
			"Enter a EUR amount with a decimal point and at most 6 decimal places.",
		);
	const micros =
		BigInt(match[1]) * 1_000_000n + BigInt((match[2] ?? "").padEnd(6, "0"));
	if (micros <= 0 || micros > BigInt(MAX_BILLING_MICROS))
		throw new Error(
			"The personal allowance must be greater than €0 and at most €1,000,000.",
		);
	return Number(micros);
}

export function formatEuroMicros(value: number): string {
	if (!Number.isSafeInteger(value) || value < 0) return "Unavailable";
	const micros = BigInt(value);
	const fraction = (micros % 1_000_000n)
		.toString()
		.padStart(6, "0")
		.replace(/0+$/, "")
		.padEnd(2, "0");
	return `€${micros / 1_000_000n}.${fraction}`;
}

export function localExpiryValue(seconds: number): string {
	const date = new Date(seconds * 1000);
	return new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
		.toISOString()
		.slice(0, 16);
}

export function isActiveGrant(
	grant: { status: string; expires_at: number },
	now = Date.now() / 1000,
): boolean {
	return grant.status === "active" && grant.expires_at > now;
}

export function publicResourceBinding(
	grant: ResourceGrant,
	billing: BillingGrant | undefined,
	deviceId: string,
	account: string,
	now = Date.now() / 1000,
	deviceOwnerId?: string,
): string {
	if (
		!isActiveGrant(grant, now) ||
		(billing ? !isActiveGrant(billing, now) : !grant.online_access) ||
		grant.device_id !== deviceId ||
		(grant.delegating_user_id !== account && deviceOwnerId !== account) ||
		(billing && billing.grant_id !== grant.grant_id)
	)
		throw new Error(
			"Both approvals must be active for this device and an authorized participant.",
		);
	return JSON.stringify(
		{
			resource_grant: {
				grant_id: grant.grant_id,
				authz_version: grant.authz_version,
				...(billing
					? {
							billing_grant_id: billing.billing_grant_id,
							billing_authz_version: billing.authz_version,
						}
					: {}),
			},
		},
		null,
		2,
	);
}

const pathFor = (deviceId: string) => `devices/${encodeURIComponent(deviceId)}`;

export async function loadDeviceResources(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<DeviceResources> {
	const path = pathFor(deviceId);
	const [grants, billing, instances] = await Promise.all([
		api.get<unknown>(profile, `${path}/resource-grants`),
		api.get<unknown>(profile, `${path}/billing-grants`),
		api.get<unknown>(profile, `${path}/instances`),
	]);
	const result = {
		grants: z.array(resourceSchema).parse(grants),
		billing: z.array(billingSchema).parse(billing),
		instances: z.array(instanceSchema).parse(instances),
	};
	if (
		result.grants.some((grant) => grant.device_id !== deviceId) ||
		result.instances.some((instance) => instance.device_id !== deviceId)
	)
		throw new Error("The hub returned resources for another device.");
	return result;
}

export async function createDeviceResourceGrant(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	request: CreateResourceGrant,
) {
	return resourceSchema.parse(
		await api.post<unknown>(
			profile,
			`${pathFor(deviceId)}/resource-grants`,
			request,
		),
	);
}

export async function approveDeviceBilling(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	grantId: string,
	request: { limit_micros: number; expires_at: number },
) {
	return billingSchema.parse(
		await api.post<unknown>(
			profile,
			`${pathFor(deviceId)}/resource-grants/${encodeURIComponent(grantId)}/billing`,
			request,
		),
	);
}

export function revokeDeviceGrant(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	kind: "resource" | "billing",
	id: string,
) {
	return api.del<void>(
		profile,
		`${pathFor(deviceId)}/${kind === "resource" ? "resource-grants" : "billing-grants"}/${encodeURIComponent(id)}`,
	);
}
