import type { DevicesT } from "../../primitives/area-context";
import { compactNumber, durationMs } from "../models-copy";
import type { Consumer } from "../models-view";
import type { StatsRange } from "./stats-view";

/*
 * Words and numbers of the statistics view. Literal `devices:`-prefixed keys
 * so the extractor files them under `devices`.
 */

export function rangeLabel(t: DevicesT, range: StatsRange) {
	const labels = {
		day: t("devices:models.stats.range.day", "24 h"),
		week: t("devices:models.stats.range.week", "7 d"),
		quarter: t("devices:models.stats.range.quarter", "90 d"),
	} satisfies Record<StatsRange, string>;
	return labels[range];
}

/** "No requests to Qwen3-8B Q4_K_M in the last 7 days." */
export function emptyText(t: DevicesT, range: StatsRange, model?: string) {
	const texts = {
		day: model
			? t(
					"devices:models.stats.empty.dayModel",
					"No requests to {{model}} in the last 24 hours.",
					{ model },
				)
			: t(
					"devices:models.stats.empty.day",
					"No model answered a request in the last 24 hours.",
				),
		week: model
			? t(
					"devices:models.stats.empty.weekModel",
					"No requests to {{model}} in the last 7 days.",
					{ model },
				)
			: t(
					"devices:models.stats.empty.week",
					"No model answered a request in the last 7 days.",
				),
		quarter: model
			? t(
					"devices:models.stats.empty.quarterModel",
					"No requests to {{model}} in the last 90 days.",
					{ model },
				)
			: t(
					"devices:models.stats.empty.quarter",
					"No model answered a request in the last 90 days.",
				),
	} satisfies Record<StatsRange, string>;
	return texts[range];
}

export interface StatsFormats {
	count(value: number): string;
	ms(value: number): string;
	speed(value: number): string;
}

const decimals = new Map<string, Intl.NumberFormat>();

function decimal(locale: string) {
	let format = decimals.get(locale);
	if (!format) {
		format = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
		decimals.set(locale, format);
	}
	return format;
}

/** "1.2K", "640 ms", "86 tok/s". */
export function statsFormats(t: DevicesT, locale: string): StatsFormats {
	return {
		count: (value) => compactNumber(locale, value),
		ms: (value) => durationMs(t, value),
		speed: (value) =>
			t("devices:models.stats.speedValue", "{{value}} tok/s", {
				value: decimal(locale).format(value),
			}),
	};
}

/** Who called: "You", "The owner", a person's name or "One person", or the service's placement. */
export function consumerLabel(
	t: DevicesT,
	consumer: Consumer,
	name: (userId: string) => string | undefined,
) {
	if (consumer.kind === "you")
		return t("devices:models.stats.consumers.you", "You");
	if (consumer.kind === "owner")
		return t("devices:models.stats.consumers.owner", "The owner");
	if (consumer.kind === "service") return consumer.serviceId;
	const known = consumer.userId ? name(consumer.userId) : undefined;
	return known ?? t("devices:models.stats.consumers.person", "One person");
}
