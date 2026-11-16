import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { renderToStaticMarkup } from "react-dom/server";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import { BitRatingBadge } from "./model-rating";

createI18n({ language: "en" });

function bit(type: IBitTypes, evaluations: Record<string, unknown>): IBit {
	return {
		type,
		model_evaluation: { evaluations },
	} as unknown as IBit;
}

describe("BitRatingBadge in mixed asset lists", () => {
	test("shows the category quality estimate for speech models", () => {
		const markup = renderToStaticMarkup(
			<BitRatingBadge
				bit={bit(IBitTypes.Tts, {
					quality_estimate: {
						category: IBitTypes.Tts,
						score: 84,
						low: 78,
						high: 89,
						confidence: "medium",
						criteria: "Naturalness and pronunciation.",
						reasoning: "Estimated from published model comparisons.",
						sources: ["https://example.com/model-card"],
						updated_at: "2026-11-16",
					},
				})}
			/>,
		);
		expect(markup).toContain("Quality");
		expect(markup).toContain("≈84");
		expect(markup).not.toContain("Not rated");
	});

	test("leaves supporting files without a score badge", () => {
		expect(
			renderToStaticMarkup(<BitRatingBadge bit={bit(IBitTypes.File, {})} />),
		).toBe("");
	});

	test("shows the current version for chat scores and hides older scores", () => {
		const evaluations = {
			artificial_analysis_intelligence_index: 42.89,
			artificial_analysis_intelligence_index_version: "5.0.1",
			benchmark_status: "measured",
		};
		const markup = renderToStaticMarkup(
			<BitRatingBadge bit={bit(IBitTypes.Llm, evaluations)} />,
		);
		expect(markup).toContain("AA v5.0.1");
		expect(markup).toContain(">43<");
		expect(markup).not.toContain("≈");
		expect(
			renderToStaticMarkup(
				<BitRatingBadge
					bit={bit(IBitTypes.Llm, {
						...evaluations,
						artificial_analysis_intelligence_index_version: "4.3",
					})}
				/>,
			),
		).toBe("");
	});
});
