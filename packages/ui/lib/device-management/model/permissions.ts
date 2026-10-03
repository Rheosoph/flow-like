import type { Capability, InventoryScope } from "../types";

interface CapabilityInfo {
	/** IA §6.6.2 "whole device only". */
	deviceOnly: boolean;
	/** Lets the holder run workflows on the device (trust confirmation without a required sandbox). */
	runsCode: boolean;
}

/** IA §6.6.2, in display order. */
const CAPABILITY_INFO = {
	status: { deviceOnly: false, runsCode: false },
	logs: { deviceOnly: false, runsCode: false },
	metrics: { deviceOnly: false, runsCode: false },
	deploy: { deviceOnly: false, runsCode: true },
	start: { deviceOnly: false, runsCode: true },
	stop: { deviceOnly: false, runsCode: false },
	restart: { deviceOnly: false, runsCode: true },
	remove: { deviceOnly: false, runsCode: false },
	scale: { deviceOnly: false, runsCode: true },
	update_agent: { deviceOnly: true, runsCode: false },
	reboot: { deviceOnly: true, runsCode: false },
	manage_certificates: { deviceOnly: true, runsCode: false },
} as const satisfies Record<Capability, CapabilityInfo>;

export const CAPABILITIES = Object.keys(
	CAPABILITY_INFO,
) as readonly Capability[];

export const DEVICE_ONLY_CAPABILITIES = CAPABILITIES.filter(
	(capability) => CAPABILITY_INFO[capability].deviceOnly,
);

export const PERMISSION_PRESET_IDS = [
	"viewer",
	"operator",
	"deployer",
	"device_admin",
] as const;
export type PermissionPreset = (typeof PERMISSION_PRESET_IDS)[number];

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
};

type ScopeKind = InventoryScope["kind"];
const kindOf = (scope: InventoryScope | ScopeKind): ScopeKind =>
	typeof scope === "string" ? scope : scope.kind;

/** Distinct, known capabilities in IA §6.6.2 order. */
export function orderCapabilities(
	capabilities: Iterable<Capability>,
): Capability[] {
	const held = new Set(capabilities);
	return CAPABILITIES.filter((capability) => held.has(capability));
}

/** R10: a preset name plus "N permissions"; anything else is "custom". */
export function presetOf(capabilities: readonly Capability[]): {
	preset: PermissionPreset | "custom";
	count: number;
} {
	const held = orderCapabilities(capabilities);
	const preset = PERMISSION_PRESET_IDS.find((id) => {
		const caps = PERMISSION_PRESETS[id];
		return (
			caps.length === held.length &&
			caps.every((capability) => held.includes(capability))
		);
	});
	return { preset: preset ?? "custom", count: held.length };
}

/** Device-only capabilities are refused for project and service scopes. */
export function capabilitiesAllowedFor(
	scope: InventoryScope | ScopeKind,
): readonly Capability[] {
	return kindOf(scope) === "device"
		? CAPABILITIES
		: CAPABILITIES.filter(
				(capability) => !CAPABILITY_INFO[capability].deviceOnly,
			);
}

/** The capabilities in `capabilities` that `scope` cannot hold. */
export function refusedFor(
	scope: InventoryScope | ScopeKind,
	capabilities: readonly Capability[],
): Capability[] {
	const allowed = capabilitiesAllowedFor(scope);
	return orderCapabilities(
		capabilities.filter((capability) => !allowed.includes(capability)),
	);
}

/** Presets that fit `scope`: Device admin needs the whole device. */
export function presetsFor(
	scope: InventoryScope | ScopeKind,
): readonly PermissionPreset[] {
	const allowed = capabilitiesAllowedFor(scope);
	return PERMISSION_PRESET_IDS.filter((id) =>
		PERMISSION_PRESETS[id].every((capability) => allowed.includes(capability)),
	);
}

export function runsCode(capabilities: readonly Capability[]): boolean {
	return capabilities.some(
		(capability) => CAPABILITY_INFO[capability]?.runsCode,
	);
}

export function isDeviceOnly(capability: Capability): boolean {
	return CAPABILITY_INFO[capability].deviceOnly;
}
