import { BOT_PRESETS, type BotPreset } from "./bot-presets";

export interface BotPresetMatch {
	preset: BotPreset;
	score: number;
}

const STOPWORDS = new Set(
	[
		"a an the and or but to of in on at by for with from into about as is are be been it its",
		"this that these those my me mine our us we you your he she they them can could should would",
		"will shall may might must do does did want wants need needs like please bot bots some any all",
		"every each when what which who how then than so if not no also just up out over new",
		"der die das den dem des ein eine einen einem einer eines und oder aber zu zum zur von vom im",
		"am an auf aus bei mit nach für über unter vor ist sind sein war wird werden soll sollte kann",
		"können muss möchte ich du er sie es wir ihr mein meine meinen meinem meiner meines mir mich",
		"dein deine unser unsere unserer unseren unserem bitte jeden jede jeder jedes dass wenn was wer",
		"wie auch nur noch schon",
		"mal alle alles nicht kein keine dann da hier",
	]
		.join(" ")
		.split(" ")
		.map(normalizeBotPresetText),
);

/** Exact word, word built on the term (meetings, Kalendereinträge), compound tail (Marktrecherche), half-typed word. */
const QUALITY = { exact: 1, stem: 0.85, tail: 0.7, partial: 0.6 } as const;
const MIN_SCORE = 1;

const vocabularies = new WeakMap<BotPreset, ReadonlyMap<string, number>>();

/** Lower case, no accents, ß as ss. */
export function normalizeBotPresetText(text: string): string {
	return text
		.toLowerCase()
		.normalize("NFKD")
		.replace(/\p{M}+/gu, "")
		.replace(/ß/g, "ss");
}

export function tokenizeBotPresetText(text: string): string[] {
	const tokens = normalizeBotPresetText(text)
		.split(/[^\p{L}\p{N}]+/u)
		.filter((token) => token.length >= 2 && !STOPWORDS.has(token));
	return [...new Set(tokens)];
}

function vocabularyOf(preset: BotPreset) {
	const cached = vocabularies.get(preset);
	if (cached) return cached;
	const vocabulary = new Map<string, number>();
	for (const terms of [preset.keywords.en, preset.keywords.de]) {
		for (const [term, weight] of Object.entries(terms)) {
			const key = normalizeBotPresetText(term);
			vocabulary.set(key, Math.max(weight, vocabulary.get(key) ?? 0));
		}
	}
	vocabularies.set(preset, vocabulary);
	return vocabulary;
}

/** A short term only takes an ending (meeting → meetings); a long one also heads a compound. */
function isStem(token: string, term: string) {
	const extra = token.length - term.length;
	if (extra <= 0 || !token.startsWith(term)) return false;
	return term.length >= 5 || (term.length >= 3 && extra <= 3);
}

function isTail(token: string, term: string) {
	return token.length > term.length && term.length >= 5 && token.endsWith(term);
}

/** At least four letters and half of the term typed so far. */
function isPartial(token: string, term: string) {
	return (
		token.length < term.length &&
		token.length >= 4 &&
		token.length * 2 >= term.length &&
		term.startsWith(token)
	);
}

function matchQuality(token: string, term: string) {
	if (token === term) return QUALITY.exact;
	if (isStem(token, term)) return QUALITY.stem;
	if (isTail(token, term)) return QUALITY.tail;
	return isPartial(token, term) ? QUALITY.partial : 0;
}

function scorePreset(tokens: readonly string[], preset: BotPreset) {
	let score = 0;
	for (const token of tokens) {
		let best = 0;
		for (const [term, weight] of vocabularyOf(preset))
			best = Math.max(best, weight * matchQuality(token, term));
		score += best;
	}
	return Math.round(score * 100) / 100;
}

/** Presets that fit a free-text description, best first. Ties keep the preset order; unrelated text gives none. */
export function rankBotPresets(
	text: string,
	presets: readonly BotPreset[] = BOT_PRESETS,
): BotPresetMatch[] {
	const tokens = tokenizeBotPresetText(text);
	if (!tokens.length) return [];
	return presets
		.map((preset) => ({ preset, score: scorePreset(tokens, preset) }))
		.filter((match) => match.score >= MIN_SCORE)
		.sort((a, b) => b.score - a.score);
}
