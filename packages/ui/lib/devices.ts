import { z } from "zod";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";
import type { DeviceRow } from "./device-management/model/types";

export type { DeviceRow } from "./device-management/model/types";

export interface DeviceStatus {
	device_id: string;
	owner_id: string;
	name: string;
	status: "active" | "revoked";
	registered_at: number;
	last_seen_at: number | null;
	auth_epoch: number;
}

export const MAX_DISPLAY_NAME_LENGTH = 64;

const timestamp = z.number().int().safe();
const count = z.number().int().nonnegative().safe();
const publicKey = z.object({
	kty: z.literal("OKP"),
	crv: z.literal("Ed25519"),
	x: z.string().min(1).max(64),
});
const identitySchema = z.object({
	auth_key: publicKey,
	telemetry_key: publicKey,
	management_key: z.array(z.number().int().min(0).max(255)).length(32),
});

/** Fields older hubs do not send; a malformed value reads as absent instead of failing the row. */
const lenient = <T extends z.ZodTypeAny>(schema: T) =>
	schema.optional().catch(undefined);

const rowSchema = z.object({
	device_id: z.string().min(1).max(128),
	owner_id: z.string(),
	name: z.string(),
	status: z.enum(["active", "revoked"]),
	registered_at: timestamp,
	last_seen_at: timestamp.nullable(),
	auth_epoch: count,
	identity: identitySchema,
	display_name: lenient(z.string().nullable()),
	relationship: lenient(z.enum(["owner", "shared", "cloud_approval"])),
	access_expires_at: lenient(timestamp.nullable()),
	access_rules_expire_at: lenient(timestamp.nullable()),
	revoked_at: lenient(timestamp.nullable()),
	cloud_approvals: lenient(
		z
			.object({
				resource_grants: count,
				billing_grants: count,
				expires_at: timestamp,
			})
			.nullable(),
	),
	auth_rejection: lenient(
		z
			.object({
				code: z.enum(["clock_skew", "revoked_credential"]),
				skew_seconds: timestamp.nullable(),
				count,
				first_at: timestamp,
				last_at: timestamp,
			})
			.nullable(),
	),
});

export function parseDeviceRow(value: unknown): DeviceRow {
	return rowSchema.parse(value) as DeviceRow;
}

export function parseDeviceRows(value: unknown): DeviceRow[] {
	return z.array(rowSchema).parse(value) as DeviceRow[];
}

const devicePath = (deviceId: string) =>
	`devices/${encodeURIComponent(deviceId)}`;

export async function listDevices(
	api: IApiState,
	profile: IProfile,
): Promise<DeviceRow[]> {
	return parseDeviceRows(await api.get<unknown>(profile, "devices"));
}

function expectDevice(value: unknown, deviceId: string): DeviceRow {
	const row = parseDeviceRow(value);
	if (row.device_id !== deviceId)
		throw new Error(
			`The hub returned device ${row.device_id} for a request about ${deviceId}.`,
		);
	return row;
}

export async function getDevice(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<DeviceRow> {
	return expectDevice(
		await api.get<unknown>(profile, devicePath(deviceId)),
		deviceId,
	);
}

/** Trim + NFC, 1–64 characters, no control characters; `null` (or blank) clears the name. */
export function normalizeDisplayName(value: string | null): string | null {
	if (value === null) return null;
	const normalized = value.normalize("NFC").trim();
	if (!normalized) return null;
	if ([...normalized].length > MAX_DISPLAY_NAME_LENGTH)
		throw new Error(
			`A device name can have at most ${MAX_DISPLAY_NAME_LENGTH} characters.`,
		);
	if (/\p{Cc}/u.test(normalized))
		throw new Error("A device name cannot contain control characters.");
	return normalized;
}

export async function renameDevice(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	displayName: string | null,
): Promise<DeviceRow> {
	const display_name = normalizeDisplayName(displayName);
	return expectDevice(
		await api.patch<unknown>(profile, devicePath(deviceId), { display_name }),
		deviceId,
	);
}

export function revokeDevice(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<void> {
	return api.del<void>(profile, devicePath(deviceId));
}
