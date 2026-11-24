import type { IBit } from "../schema/bit/bit";

/** Jurisdictions a hosted Bit can declare it is served from. Mirrors the API's upsert check. */
export const HOSTING_REGIONS = ["eu"] as const;

export type HostingRegion = (typeof HOSTING_REGIONS)[number];

/** Parameters key the Bit author sets; absent means the region is unknown. */
export const HOSTING_REGION_KEY = "hosting_region";

export function parseHostingRegion(value: unknown): HostingRegion | undefined {
	return HOSTING_REGIONS.find((known) => known === value);
}

export function bitHostingRegion(
	bit: Pick<IBit, "parameters">,
): HostingRegion | undefined {
	return parseHostingRegion(bit.parameters?.[HOSTING_REGION_KEY]);
}

export function isEuHosted(bit: Pick<IBit, "parameters">): boolean {
	return bitHostingRegion(bit) === "eu";
}

export function withHostingRegion(
	parameters: Record<string, unknown>,
	region: HostingRegion | undefined,
): Record<string, unknown> {
	const { [HOSTING_REGION_KEY]: _previous, ...rest } = parameters;
	return region ? { ...rest, [HOSTING_REGION_KEY]: region } : rest;
}
