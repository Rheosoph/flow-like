import {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	type BotCapabilityGroupId,
} from "@flow-like/flow-like-ui/components/bots-tab/editor/bot-capability-groups";
import { BOT_PRESETS } from "@flow-like/flow-like-ui/components/bots-tab/editor/bot-presets";
import type { BotAppearance } from "@flow-like/flow-like-ui/components/bots/types";
import {
	type BotCapability,
	type BotLocation,
	LOCAL_ONLY_BOT_CAPABILITIES,
	isBotCapabilityAvailable,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import {
	capabilities as capabilityCopy,
	editor as editorCopy,
	presets as presetCopy,
} from "../../../../../packages/locales/locales/en/bots.json";

export type { BotCapability, BotLocation };
export {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	BOT_PRESETS,
	LOCAL_ONLY_BOT_CAPABILITIES,
	isBotCapabilityAvailable,
};

export const ALL_CAPABILITIES: readonly BotCapability[] =
	BOT_CAPABILITY_GROUPS.flatMap((group) => group.capabilities);

export function capabilityLabel(id: BotCapability): string {
	return capabilityCopy[id]?.label ?? id;
}

export function groupLabel(id: BotCapabilityGroupId): string {
	return editorCopy.abilities.groups[id];
}

export function presetName(id: string): string {
	return (presetCopy as Record<string, { name: string }>)[id]?.name ?? id;
}

export function presetDescription(id: string): string {
	return (
		(presetCopy as Record<string, { description: string }>)[id]?.description ??
		""
	);
}

/** The cast the homepage crew showcase introduced. */
export const CREW = {
	scout: { name: "Scout", appearance: { kind: "pip" } },
	byte: { name: "Byte", appearance: { kind: "chip" } },
	flux: { name: "Flux", appearance: { kind: "flux" } },
	post: { name: "Post", appearance: { kind: "mochi" } },
	loop: { name: "Loop", appearance: { kind: "orbi" } },
} as const satisfies Record<
	string,
	{ name: string; appearance: BotAppearance }
>;

export type CrewId = keyof typeof CREW;
