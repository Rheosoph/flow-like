import { z } from "zod";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";

const readiness = z
	.object({
		version: z.literal(1),
		ready: z.boolean(),
		checks: z
			.array(
				z
					.object({
						id: z.enum([
							"policy",
							"signing",
							"api",
							"signaling",
							"release",
							"database",
						]),
						ready: z.boolean(),
						message: z.string().min(1).max(512),
					})
					.strict(),
			)
			.length(6),
	})
	.strict();

export type DeviceSetupReadiness = z.infer<typeof readiness>;

export function parseDeviceSetupReadiness(
	value: unknown,
): DeviceSetupReadiness {
	const parsed = readiness.parse(value);
	if (
		new Set(parsed.checks.map((check) => check.id)).size !== 6 ||
		parsed.ready !== parsed.checks.every((check) => check.ready)
	)
		throw new Error("The hub returned inconsistent device setup checks.");
	return parsed;
}

export async function checkDeviceSetup(
	api: IApiState,
	profile: IProfile,
	signal?: AbortSignal,
): Promise<DeviceSetupReadiness> {
	const value = await api.fetch<unknown>(profile, "devices/setup", {
		method: "GET",
		signal,
	});
	return parseDeviceSetupReadiness(value);
}
