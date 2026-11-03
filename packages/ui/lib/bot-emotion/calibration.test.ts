import { describe, expect, it } from "vitest";
import {
	EMOTION_CALIBRATION_REVISION,
	calibrateEmotionScores,
} from "./calibration";
import { BOT_EMOTION_REVISION, type EmotionScores } from "./protocol";
import { EmotionScorer, chooseExpression, expressionScores } from "./scoring";

// Relevant raw probabilities captured with the pinned q8 model in a browser.
// The original implementation chose neutral for both hostile German drafts.
const EXAMPLES = [
	{
		text: "I am sad 😊",
		expression: "sad",
		raw: { sadness: 0.9849020838737488 },
	},
	{
		text: "Great, another failure /s",
		expression: "neutral",
		raw: { admiration: 0.8194171190261841, disappointment: 0.4613993465900421 },
	},
	{
		text: "I want you to export this report as a PDF.",
		expression: "curious",
		raw: {
			approval: 0.11605680733919144,
			desire: 0.7189970016479492,
			neutral: 0.38545745611190796,
		},
	},
	{
		text: "du kleiner wichser brings wirklich garnichts hin",
		expression: "concerned",
		raw: {
			neutral: 0.5234704613685608,
			annoyance: 0.1919647604227066,
			disapproval: 0.21048662066459656,
			approval: 0.04635210335254669,
			disappointment: 0.04897788539528847,
		},
	},
	{
		text: "du kleiner wichser bringst wirklich gar nichts hin",
		expression: "concerned",
		raw: {
			neutral: 0.5122005939483643,
			annoyance: 0.35933011770248413,
			disapproval: 0.14379452168941498,
			approval: 0.03487924858927727,
			disappointment: 0.05437897518277168,
		},
	},
	{
		text: "Ich liebe dich und bin so froh, dass du mir hilfst!",
		expression: "love",
		raw: {
			neutral: 0.000008081465239229146,
			love: 0.9734739065170288,
			joy: 0.28736361861228943,
			caring: 0.1381550133228302,
		},
	},
	{
		text: "Ich bin traurig und vermisse meine Familie.",
		expression: "sad",
		raw: {
			neutral: 0.002417738549411297,
			sadness: 0.9976567029953003,
			grief: 0.23598121106624603,
			disappointment: 0.021297696977853775,
		},
	},
	{
		text: "such mir bitte was zum porsche panamera s4 raus, welche mängel hat der häufig",
		expression: "curious",
		raw: {
			neutral: 0.34224656224250793,
			curiosity: 0.6020520925521851,
			confusion: 0.030131792649626732,
			annoyance: 0.03440302237868309,
			desire: 0.04429887235164642,
		},
	},
	{
		text: "Der Termin ist am Dienstag um zehn Uhr.",
		expression: "neutral",
		raw: {
			neutral: 0.915891170501709,
			approval: 0.027521496638655663,
			realization: 0.012867672368884087,
			optimism: 0.010196760296821594,
			excitement: 0.005379457492381334,
		},
	},
	{
		text: "The meeting is at 3 pm tomorrow.",
		expression: "neutral",
		raw: {
			neutral: 0.7747272849082947,
			optimism: 0.1697070598602295,
			approval: 0.017247214913368225,
			desire: 0.012828567065298557,
		},
	},
] satisfies { text: string; expression: string; raw: EmotionScores }[];

describe("pinned emotion calibration", () => {
	it("uses thresholds from the same revision as the inference model", () => {
		expect(EMOTION_CALIBRATION_REVISION).toBe(BOT_EMOTION_REVISION);
	});

	it.each(EXAMPLES)("recognizes $expression in $text", async (example) => {
		const scorer = new EmotionScorer(async (text) =>
			calibrateEmotionScores(
				text === example.text ? example.raw : { neutral: 1 },
			),
		);
		const scores = await scorer.score({
			draft: example.text,
			messages: [
				{ role: "assistant", content: "A previous factual response." },
			],
		});
		expect(chooseExpression(scores)).toBe(example.expression);
	});

	it("does not promote weak labels by combining them into a large face group", () => {
		const scores = expressionScores(
			calibrateEmotionScores({
				neutral: 0.8,
				anger: 0.15,
				annoyance: 0.15,
				disapproval: 0.15,
				disgust: 0.15,
				fear: 0.15,
				nervousness: 0.15,
				gratitude: 0.4,
			}),
		);
		expect(scores.concerned).toBe(0);
		expect(scores.happy).toBe(0);
		expect(chooseExpression(scores)).toBe("neutral");
	});

	it("rejects invalid and unknown labels", () => {
		expect(
			chooseExpression(
				expressionScores(
					calibrateEmotionScores({
						anger: Number.NaN,
						joy: Number.POSITIVE_INFINITY,
						love: -1,
						unknown: 1,
					}),
				),
			),
		).toBe("neutral");
	});
});
