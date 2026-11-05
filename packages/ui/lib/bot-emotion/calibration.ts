import type { EmotionScores } from "./protocol";

// thresholds.json from the pinned model revision. These labels are independent,
// so a neutral score does not veto an emotion that passes its own threshold.
// https://huggingface.co/Horizon-Labs/multilingual-emotions-small/blob/ac26dadbdafc26276e01093790104679bf33aec5/thresholds.json
export const EMOTION_CALIBRATION_REVISION =
	"ac26dadbdafc26276e01093790104679bf33aec5";

const THRESHOLDS: Readonly<Record<string, number>> = {
	admiration: 0.4,
	amusement: 0.2,
	anger: 0.28,
	annoyance: 0.16,
	approval: 0.16,
	caring: 0.28,
	confusion: 0.16,
	curiosity: 0.26,
	desire: 0.3,
	disappointment: 0.16,
	disapproval: 0.2,
	disgust: 0.32,
	embarrassment: 0.2,
	excitement: 0.18,
	fear: 0.2,
	gratitude: 0.6,
	grief: 0.1,
	joy: 0.26,
	love: 0.5,
	nervousness: 0.62,
	optimism: 0.4,
	pride: 0.28,
	realization: 0.18,
	relief: 0.32,
	remorse: 0.14,
	sadness: 0.34,
	surprise: 0.2,
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
