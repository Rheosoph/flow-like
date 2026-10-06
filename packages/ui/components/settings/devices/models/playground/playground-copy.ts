import { DeviceTunnelError } from "../../../../../lib/device-management/tunnel";
import { classifyDeviceError } from "../../../../../lib/device-management/workspace/errors";
import { errorCopy } from "../../copy/error-copy";
import type { DevicesT } from "../../primitives/area-context";
import { durationMs } from "../models-copy";
import {
	AnswerBrokeOffError,
	type ChatMeasure,
	type EmbedMeasure,
	GatewayError,
	gatewayStalled,
} from "./gateway-client";

/*
 * Words of the playground. Literal `devices:`-prefixed keys so the extractor
 * files them under `devices`.
 */

const decimals = new Map<string, Intl.NumberFormat>();

function decimal(locale: string, value: number) {
	let format = decimals.get(locale);
	if (!format) {
		format = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
		decimals.set(locale, format);
	}
	return format.format(value);
}

function speedText(t: DevicesT, locale: string, value: number) {
	return t("devices:models.playground.measure.speed", "{{value}} tok/s", {
		value: decimal(locale, value),
	});
}

/** "620 tokens in, 390 out", or "390 tokens out" when the gateway sent no prompt count. */
function tokensText(
	t: DevicesT,
	locale: string,
	completion: number,
	prompt: number | undefined,
) {
	return prompt === undefined
		? t("devices:models.playground.measure.out", {
				count: completion,
				defaultValue_one: "{{count, number}} token out",
				defaultValue_other: "{{count, number}} tokens out",
			})
		: t("devices:models.playground.measure.inOut", {
				count: prompt,
				out: decimal(locale, completion),
				defaultValue_one: "{{count, number}} token in, {{out}} out",
				defaultValue_other: "{{count, number}} tokens in, {{out}} out",
			});
}

/** "First token 190 ms · 86 tok/s · 620 tokens in, 390 out · 4.8 s in all". */
export function chatMeasureText(
	t: DevicesT,
	locale: string,
	measure: ChatMeasure,
) {
	const parts: string[] = [];
	if (measure.ttftMs !== undefined)
		parts.push(
			t("devices:models.playground.measure.ttft", "First token {{value}}", {
				value: durationMs(t, measure.ttftMs),
			}),
		);
	if (measure.tokensPerSecond !== undefined)
		parts.push(speedText(t, locale, measure.tokensPerSecond));
	if (measure.completionTokens !== undefined)
		parts.push(
			tokensText(t, locale, measure.completionTokens, measure.promptTokens),
		);
	parts.push(
		t("devices:models.playground.measure.total", "{{value}} in all", {
			value: durationMs(t, measure.totalMs),
		}),
	);
	return parts.join(" · ");
}

/** "768 dimensions · 12 tokens · 45 ms · 266 tok/s". */
export function embedMeasureText(
	t: DevicesT,
	locale: string,
	measure: EmbedMeasure,
) {
	const parts = [
		t("devices:models.playground.measure.dimensions", {
			count: measure.dimensions,
			defaultValue_one: "{{count, number}} dimension",
			defaultValue_other: "{{count, number}} dimensions",
		}),
	];
	if (measure.promptTokens !== undefined)
		parts.push(
			t("devices:models.playground.measure.tokens", {
				count: measure.promptTokens,
				defaultValue_one: "{{count, number}} token",
				defaultValue_other: "{{count, number}} tokens",
			}),
		);
	parts.push(durationMs(t, measure.totalMs));
	if (measure.tokensPerSecond !== undefined)
		parts.push(speedText(t, locale, measure.tokensPerSecond));
	return parts.join(" · ");
}

/** "[0.0123, -0.0456, …]": the first values of a vector. */
export function vectorPreview(t: DevicesT, measure: EmbedMeasure) {
	return t("devices:models.playground.embed.preview", "[{{values}}, …]", {
		values: measure.preview.map((value) => value.toFixed(4)).join(", "),
	});
}

function refusalText(t: DevicesT, error: GatewayError, device: string) {
	if (error.status === 401 || error.status === 403)
		return t(
			"devices:models.playground.refused.notAllowed",
			"Your access to {{device}} doesn't include using its models.",
			{ device },
		);
	if (error.status === 404)
		return t(
			"devices:models.playground.refused.notFound",
			"{{device}} doesn't host this model anymore.",
			{ device },
		);
	if (error.status === 429)
		return t(
			"devices:models.playground.refused.busy",
			"Too many requests are waiting for {{device}}. Try again in a moment.",
			{ device },
		);
	if (error.status === 503)
		return t(
			"devices:models.playground.refused.unavailable",
			"The model can't answer right now.",
		);
	return t(
		"devices:models.playground.refused.other",
		"The model gateway on {{device}} answered with error {{status}}.",
		{ device, status: error.status },
	);
}

/** The device refused the gateway stream itself: no use of its models, or no model host running. */
function streamRefusal(t: DevicesT, error: DeviceTunnelError, device: string) {
	if (error.code === "unauthorized")
		return t(
			"devices:models.playground.refused.notAllowed",
			"Your access to {{device}} doesn't include using its models.",
			{ device },
		);
	if (error.code === "unsupported")
		return t(
			"devices:models.playground.refused.noHost",
			"{{device}} isn't running its model host right now.",
			{ device },
		);
	return undefined;
}

/** Why a request failed: the gateway's refusal by status with the device's own sentence, a cut-off or stalled answer, else the connection's. */
export function failureText(t: DevicesT, error: unknown, device: string) {
	if (error instanceof GatewayError) {
		const text = refusalText(t, error, device);
		return error.reason ? `${text} “${error.reason}”` : text;
	}
	if (error instanceof AnswerBrokeOffError)
		return t(
			"devices:models.playground.brokeOff",
			"The answer from {{device}} broke off before the model finished. Try again.",
			{ device },
		);
	if (gatewayStalled(error))
		return t(
			"devices:models.playground.stalled",
			"{{device}} didn't start answering. It may still be loading the model; try again in a few minutes.",
			{ device },
		);
	const refused =
		error instanceof DeviceTunnelError
			? streamRefusal(t, error, device)
			: undefined;
	return refused ?? errorCopy(t, classifyDeviceError(error).code, { device });
}
