import {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	type BotCapabilityGroupId,
} from "@flow-like/flow-like-ui/components/bots-tab/editor/bot-capability-groups";
import { BOT_PRESETS } from "@flow-like/flow-like-ui/components/bots-tab/editor/bot-presets";
import {
	BOT_LOCATIONS,
	type BotCapability,
	type BotLocation,
	isBotCapabilityAvailable,
	LOCAL_ONLY_BOT_CAPABILITIES,
	SYSTEM_BOT_APPEARANCES,
	SYSTEM_BOT_CAPABILITIES,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import {
	DATA_CLASSIFICATIONS,
	type DataClassification,
} from "@flow-like/flow-like-ui/lib/data-classification";
import {
	capabilities as capabilityCopy,
	editor as editorCopy,
	locations as locationCopy,
	presets as presetCopy,
	system as systemCopy,
} from "../../../../../packages/locales/locales/en/bots.json";

export type { BotCapability, BotLocation, DataClassification };
export {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	BOT_LOCATIONS,
	BOT_PRESETS,
	DATA_CLASSIFICATIONS,
	isBotCapabilityAvailable,
	LOCAL_ONLY_BOT_CAPABILITIES,
	SYSTEM_BOT_APPEARANCES,
	SYSTEM_BOT_CAPABILITIES,
};

export function capabilityLabel(id: BotCapability): string {
	return capabilityCopy[id]?.label ?? id;
}

export function capabilityDescription(id: BotCapability): string {
	return capabilityCopy[id]?.description ?? "";
}

export function groupLabel(id: BotCapabilityGroupId): string {
	return editorCopy.abilities.groups[id];
}

export function locationLabel(id: BotLocation): string {
	return locationCopy[id].label;
}

export function locationDescription(id: BotLocation): string {
	return locationCopy[id].description;
}

export function presetCopyOf(id: string): {
	name: string;
	description: string;
	instructions: string;
} {
	const copy = (presetCopy as Record<string, { name: string; description: string; instructions: string }>)[id];
	return copy ?? { name: id, description: "", instructions: "" };
}

export const BUILT_IN_BOTS = [
	{
		key: "general",
		name: systemCopy.general.name,
		description: systemCopy.general.description,
		appearance: SYSTEM_BOT_APPEARANCES.general,
		capabilities: SYSTEM_BOT_CAPABILITIES.general,
	},
	{
		key: "flux",
		name: systemCopy.flux.name,
		description: systemCopy.flux.description,
		appearance: SYSTEM_BOT_APPEARANCES.flux,
		capabilities: SYSTEM_BOT_CAPABILITIES.flux,
	},
] as const;

export const CLASSIFICATION_LABELS: Readonly<Record<DataClassification, string>> = {
	public: "Public",
	internal: "Internal",
	confidential: "Confidential",
	restricted: "Restricted",
};

export function classificationRank(level: DataClassification): number {
	return DATA_CLASSIFICATIONS.indexOf(level);
}
