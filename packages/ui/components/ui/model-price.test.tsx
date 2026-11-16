import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { renderToStaticMarkup } from "react-dom/server";
import type { ModelPrice } from "../../lib/bit/model-pricing";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { ModelPricePanel } from "./model-price";

createI18n({ language: "en" });

describe("provider price reference display", () => {
	test("shows representative configuration and provider examples without promising the plan bill", () => {
		const price: ModelPrice = {
			kind: "image",
			perImage: 0.03,
			reference: {
				basis: "OpenRouter listed price via Publisher.",
				configuration: "1 output megapixel, no reference images",
				caveats: ["Provider prices exclude platform fees."],
				sources: ["https://openrouter.ai/publisher/image"],
				checkedAt: "2026-11-16T12:00:00Z",
				routingVerified: true,
				providers: [],
			},
		};
		const html = renderToStaticMarkup(
			<ModelPricePanel price={price} type={IBitTypes.ImageGeneration} />,
		);
		expect(html).toContain("Provider price reference");
		expect(html).toContain("Example provider costs");
		expect(html).toContain("1 output megapixel, no reference images");
		expect(html).toContain("$0.30");
		expect(html).toContain("Provider prices exclude platform fees.");
		expect(html).not.toContain("metered against your plan");
		expect(html).toContain("Prices checked 2026-11-16.");
	});

	test("shows the token and request assumptions behind provider examples", () => {
		const reference = {
			basis: "Listed price",
			caveats: [],
			sources: ["https://openrouter.ai/publisher/model"],
			checkedAt: "2026-11-16T12:00:00Z",
			routingVerified: true,
			providers: [],
		};
		const tokens = renderToStaticMarkup(
			<ModelPricePanel
				price={{
					kind: "tokens",
					input: 1,
					output: 3,
					request: 0,
					fromBenchmark: false,
					reference,
				}}
				type={IBitTypes.Llm}
			/>,
		);
		expect(tokens).toContain("1,500 input and 500 output tokens");
		expect(tokens).toContain("35,000 input and 1,500 output tokens");
		const embedding = renderToStaticMarkup(
			<ModelPricePanel
				price={{
					kind: "embedding",
					perMillion: 1,
					unit: "tokens",
					request: 0.01,
					reference,
				}}
				type={IBitTypes.Embedding}
			/>,
		);
		expect(embedding).toContain("600 tokens in one request");
	});

	test("makes the byte assumption explicit for speech examples", () => {
		const price: ModelPrice = {
			kind: "speech",
			perMillion: 15,
			unit: "bytes",
			reference: {
				basis: "Fish Audio input byte pricing.",
				caveats: [],
				sources: ["https://openrouter.ai/fish-audio/s2.1-pro"],
				checkedAt: "2026-11-16T12:00:00Z",
				routingVerified: true,
				providers: [],
			},
		};
		const html = renderToStaticMarkup(
			<ModelPricePanel price={price} type={IBitTypes.Tts} />,
		);
		expect(html).toContain("900 ASCII text bytes per minute");
		expect(html).toContain("Other characters can use more bytes");
		expect(html).toContain("/ 1M bytes");
	});
});
