import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotLocation } from "../../../../state/backend-state/bot-state";

const HOSTED = new Set([
	"hosted",
	"premium",
	"internal",
	"hosted:openrouter",
	"hosted:typesafe",
	"hosted:cloudflare",
	"hosted:systemone_compatible",
]);
const LOCAL = new Set([
	"local",
	"device",
	"custom:systemone",
	"custom:typesafe",
	"custom:openrouter",
	"openrouter",
]);

/** Server and managed-device bots use hosted decisions; desktop bots can use their own providers. */
export function isBotDecisionModel(bit: IBit, location: BotLocation): boolean {
	if (bit.type !== IBitTypes.SystemOne) return false;
	const provider = String(bit.parameters?.provider?.provider_name ?? "")
		.trim()
		.toLowerCase();
	return (
		HOSTED.has(provider) || (location === "desktop" && LOCAL.has(provider))
	);
}
