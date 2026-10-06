import type { Capability, InventoryScope } from "../types";
import type { AgentFeature, AgentFeatures } from "./types";

interface CapabilityInfo {
	/** IA §6.6.2 "whole device only". */
	deviceOnly: boolean;
	/** Lets the holder run workflows on the device (trust confirmation without a required sandbox). */
	runsCode: boolean;
	/** An agent without this flag rejects every access rule that holds the capability. */
	needs?: AgentFeature;
}

/** IA §6.6.2, in display order. */
const CAPABILITY_INFO = {
	status: { deviceOnly: false, runsCode: false },
	logs: { deviceOnly: false, runsCode: false },
	metrics: { deviceOnly: false, runsCode: false },
	service_connect: { deviceOnly: false, runsCode: true },
	deploy: { deviceOnly: false, runsCode: true },
	start: { deviceOnly: false, runsCode: true },
	stop: { deviceOnly: false, runsCode: false },
	restart: { deviceOnly: false, runsCode: true },
	remove: { deviceOnly: false, runsCode: false },
	scale: { deviceOnly: false, runsCode: true },
	update_agent: { deviceOnly: true, runsCode: false },
	reboot: { deviceOnly: true, runsCode: false },
	manage_certificates: { deviceOnly: true, runsCode: false },
	model_use: { deviceOnly: true, runsCode: false, needs: "model_host" },
	model_manage: { deviceOnly: true, runsCode: false, needs: "model_host" },
} as const satisfies Record<Capability, CapabilityInfo>;

const info = (capability: Capability): CapabilityInfo =>
	CAPABILITY_INFO[capability];

/** Every capability this client knows, in display order. */
export const KNOWN_CAPABILITIES = Object.keys(
	CAPABILITY_INFO,
) as readonly Capability[];

/** The capabilities every agent accepts; `capabilitiesAllowedFor` adds the flag-gated ones. */
export const CAPABILITIES = KNOWN_CAPABILITIES.filter(
	(capability) => !info(capability).needs,
);

export const DEVICE_ONLY_CAPABILITIES = CAPABILITIES.filter(
	(capability) => info(capability).deviceOnly,
);

/** Whether an agent with `features` accepts access rules that hold `capability`. */
export function agentAccepts(
	capability: Capability,
	features: AgentFeatures | undefined,
): boolean {
	const needs = info(capability).needs;
	return !needs || features?.[needs] === 1;
}

/** Older hubs verify only the permissions that predate agent feature flags. */
export function hubAccepts(
	capability: Capability,
	supported: readonly string[] | undefined,
): boolean {
	return (supported ?? CAPABILITIES).includes(capability);
}

/** Offered on every agent. */
export const PERMISSION_PRESET_IDS = [
	"viewer",
	"operator",
	"deployer",
	"device_admin",
] as const;
/** Offered only by `presetsFor` with the agent flag their permissions need. */
export const GATED_PRESET_IDS = ["model_user"] as const;
export type PermissionPreset =
	| (typeof PERMISSION_PRESET_IDS)[number]
	| (typeof GATED_PRESET_IDS)[number];
const ALL_PRESET_IDS: readonly PermissionPreset[] = [
	...PERMISSION_PRESET_IDS,
	...GATED_PRESET_IDS,
];

const VIEWER: readonly Capability[] = ["status", "logs", "metrics"];
const OPERATOR: readonly Capability[] = [
	...VIEWER,
	"start",
	"stop",
	"restart",
	"scale",
];
const DEPLOYER: readonly Capability[] = [...OPERATOR, "deploy", "remove"];

/** Client-side convenience; grants store plain capability lists. */
export const PERMISSION_PRESETS: Readonly<
	Record<PermissionPreset, readonly Capability[]>
> = {
	viewer: VIEWER,
	operator: OPERATOR,
	deployer: DEPLOYER,
	device_admin: CAPABILITIES,
	model_user: ["model_use"],
};

type ScopeKind = InventoryScope["kind"];
const kindOf = (scope: InventoryScope | ScopeKind): ScopeKind =>
	typeof scope === "string" ? scope : scope.kind;

/** Distinct, known capabilities in IA §6.6.2 order; `unknownCapabilities` names the rest. */
export function orderCapabilities(
	capabilities: Iterable<Capability>,
): Capability[] {
	const held = new Set(capabilities);
	return KNOWN_CAPABILITIES.filter((capability) => held.has(capability));
}

/**
 * Entries this client doesn't know: a newer client gave them, and the
 * verified rules read each as `unsupported`. Re-signing a grant without them
 * would take them away.
 */
export function unknownCapabilities(capabilities: Iterable<string>): string[] {
	const known = new Set<string>(KNOWN_CAPABILITIES);
	return [...capabilities].filter((capability) => !known.has(capability));
}

const sameSet = (a: readonly Capability[], b: readonly Capability[]) =>
	a.length === b.length && a.every((capability) => b.includes(capability));

/**
 * Device admin is every permission the agent accepts: every ungated one, plus
 * each flag-gated one of a flag the grant holds any of (all 15 on agents that
 * host models, as the picker gives them there).
 */
function isDeviceAdmin(held: readonly Capability[]): boolean {
	const flags = new Set(
		held.flatMap((capability) => info(capability).needs ?? []),
	);
	const all = KNOWN_CAPABILITIES.filter((capability) => {
		const needs = info(capability).needs;
		return !needs || flags.has(needs);
	});
	return sameSet(all, held);
}

/** R10: a preset name plus "N permissions"; anything else is "custom". */
export function presetOf(capabilities: readonly Capability[]): {
	preset: PermissionPreset | "custom";
	count: number;
} {
	const held = orderCapabilities(capabilities);
	const preset = isDeviceAdmin(held)
		? "device_admin"
		: ALL_PRESET_IDS.find((id) => sameSet(PERMISSION_PRESETS[id], held));
	return { preset: preset ?? "custom", count: held.length };
}

/**
 * What may be offered for `scope` on an agent with `features`: device-only
 * capabilities are refused for project and service scopes, and flag-gated ones
 * (`model_use`, `model_manage`) require both the agent flag and a hub capability
 * signal. Pass the intersection of the target devices' support for group grants.
 */
export function capabilitiesAllowedFor(
	scope: InventoryScope | ScopeKind,
	features?: AgentFeatures,
	supportedCapabilities?: readonly string[],
): readonly Capability[] {
	const whole = kindOf(scope) === "device";
	return KNOWN_CAPABILITIES.filter(
		(capability) =>
			agentAccepts(capability, features) &&
			hubAccepts(capability, supportedCapabilities) &&
			(whole || !info(capability).deviceOnly),
	);
}

/** The capabilities in `capabilities` that `scope` cannot hold. */
export function refusedFor(
	scope: InventoryScope | ScopeKind,
	capabilities: readonly Capability[],
): Capability[] {
	return kindOf(scope) === "device"
		? []
		: orderCapabilities(capabilities).filter(isDeviceOnly);
}

/** Presets that fit `scope` and the agent: Device admin needs the whole device. */
export function presetsFor(
	scope: InventoryScope | ScopeKind,
	features?: AgentFeatures,
	supportedCapabilities?: readonly string[],
): readonly PermissionPreset[] {
	const allowed = capabilitiesAllowedFor(
		scope,
		features,
		supportedCapabilities,
	);
	return ALL_PRESET_IDS.filter((id) =>
		PERMISSION_PRESETS[id].every((capability) => allowed.includes(capability)),
	);
}

export function runsCode(capabilities: readonly Capability[]): boolean {
	return capabilities.some((capability) => info(capability)?.runsCode);
}

export function isDeviceOnly(capability: Capability): boolean {
	return info(capability).deviceOnly;
}
