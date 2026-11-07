import type { EmotionScores } from "./protocol";

// Per-label thresholds from the pinned lettuce revision. A passing emotion is
// kept even when neutral also scores high.
// https://huggingface.co/Zeolit/lettuce-thymos-26m-v1/blob/57219928da2daf26201012d0aafcc3d58dea74d0/labels.json
export const EMOTION_CALIBRATION_REVISION =
	"57219928da2daf26201012d0aafcc3d58dea74d0";

const THRESHOLDS: Readonly<Record<string, number>> = {
	admiration: 0.33003830909729004,
	amusement: 0.23647058010101318,
	anger: 0.17629921436309814,
	annoyance: 0.15685714781284332,
	approval: 0.15521739423274994,
	caring: 0.20999999344348907,
	confusion: 0.24936169385910034,
	curiosity: 0.17615385353565216,
	desire: 0.2545454502105713,
	disappointment: 0.15333333611488342,
	disapproval: 0.18365854024887085,
	disgust: 0.20999999344348907,
	embarrassment: 0.2849999964237213,
	excitement: 0.19615384936332703,
	fear: 0.2664705812931061,
	gratitude: 0.4717073142528534,
	grief: 0.19956521689891815,
	joy: 0.2738383710384369,
	love: 0.43809816241264343,
	nervousness: 0.2549999952316284,
	optimism: 0.2776922881603241,
	pride: 0.23800000548362732,
	realization: 0.17249999940395355,
	relief: 0.18407407402992249,
	remorse: 0.2834782600402832,
	sadness: 0.2023809552192688,
	surprise: 0.21782608330249786,
};

/** Normalize accepted model labels before combining them into the bot's faces. */
export function calibrateEmotionScores(raw: EmotionScores): EmotionScores {
	const scores: EmotionScores = {};
	let matched = false;
	for (const [label, threshold] of Object.entries(THRESHOLDS)) {
		const probability = raw[label];
		if (!Number.isFinite(probability) || probability < threshold) {
			scores[label] = 0;
			continue;
		}
		matched = true;
		// A label at its calibrated threshold has 0.5 support; values above it
		// retain their relative strength for choosing between supported faces.
		scores[label] =
			0.5 + (0.5 * (Math.min(1, probability) - threshold)) / (1 - threshold);
	}
	scores.neutral = matched ? 0 : 1;
	return scores;
}
