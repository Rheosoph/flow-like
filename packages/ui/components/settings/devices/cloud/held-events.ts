import { botProvider } from "../../../../lib/device-management/bot-config";
import { scheduleNameList } from "../copy/schedule-copy";
import type { DevicesT } from "../primitives/area-context";

/*
 * The events a service took off the hub, split by what happens to them when
 * the service loses its cloud access or goes: the hub runs a schedule again,
 * nothing runs a bot, and the device keeps the bot's token. No React, no I/O.
 */

type Provider = NonNullable<ReturnType<typeof botProvider>>;

export interface HeldEvents {
	/** Schedules of either kind, as one list of names; null when there are none. */
	schedules: string | null;
	/** Bots, as one list of names, and the providers that issued their tokens; null when there are none. */
	bots: { names: string; providers: Provider[] } | null;
}

export interface KnownEvent {
	name: string;
	eventType: string;
}

/** An event whose type isn't known counts as a schedule, as before bots could run on a device. */
export function splitHeld(
	ids: readonly string[],
	events: ReadonlyMap<string, KnownEvent>,
	locale: string,
): HeldEvents {
	const providerOf = (id: string) =>
		botProvider(events.get(id)?.eventType ?? "");
	const bots = ids.filter((id) => providerOf(id) !== null);
	const schedules = ids.filter((id) => providerOf(id) === null);
	const names = new Map(
		[...events].map(([id, event]) => [id, event.name] as const),
	);
	const list = (subset: readonly string[]) =>
		subset.length ? scheduleNameList(subset, names, locale) : null;
	const providers = [
		...new Set(bots.flatMap((id) => providerOf(id) ?? [])),
	].sort();
	const botNames = list(bots);
	return {
		schedules: list(schedules),
		bots: botNames ? { names: botNames, providers } : null,
	};
}

/** Where a bot's token is replaced: BotFather, the Discord Developer Portal, or both. */
function providerText(t: DevicesT, providers: readonly Provider[]) {
	if (providers.length > 1)
		return t(
			"devices:cloud.revoke.provider.both",
			"BotFather and the Discord Developer Portal",
		);
	return providers[0] === "discord"
		? t("devices:cloud.revoke.provider.discord", "the Discord Developer Portal")
		: t("devices:cloud.revoke.provider.telegram", "BotFather");
}

/**
 * What ending a service's cloud access, or removing it, does to its bots: the
 * device disconnects them at its next check with the hub (a removal stops the
 * service first, so at once), can stay connected while it doesn't reach the
 * hub, and keeps the token. Only a new token at the provider cuts a bot off
 * for certain (design R2 §5.3).
 */
export function botCutOffText(
	t: DevicesT,
	values: {
		device: string;
		bots: NonNullable<HeldEvents["bots"]>;
		/** When the cloud access ends by itself; undefined when that isn't known. */
		date: string | undefined;
		/** The service is being removed: it stops first. */
		removing?: boolean;
	},
): string {
	const words = {
		device: values.device,
		events: values.bots.names,
		provider: providerText(t, values.bots.providers),
	};
	if (values.removing)
		return t(
			"devices:serviceConfig.remove.bots",
			"{{device}} disconnects {{events}} when it stops the service. The bot token stays on {{device}}: to cut the bot off for certain, replace the token with {{provider}}.",
			words,
		);
	return values.date
		? t(
				"devices:cloud.revoke.bots",
				"{{device}} disconnects {{events}} at its next check with the hub, within 30 minutes. If it can't reach the hub, it can stay connected until its cloud access ends on {{date}}. The bot token stays on {{device}}: to cut the bot off for certain, replace the token with {{provider}}.",
				{ ...words, date: values.date },
			)
		: t(
				"devices:cloud.revoke.botsNoDate",
				"{{device}} disconnects {{events}} at its next check with the hub, within 30 minutes. If it can't reach the hub, it can stay connected until its cloud access ends. The bot token stays on {{device}}: to cut the bot off for certain, replace the token with {{provider}}.",
				words,
			);
}
