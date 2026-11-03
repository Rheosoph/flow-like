import type { BotExpression } from "../../components/bots/types";

type Cue = readonly [BotExpression, RegExp];
interface CueMatch {
	expression: BotExpression;
	index: number;
	negated: boolean;
	explicit: boolean;
}

const EXPLICIT_CUES: readonly Cue[] = [
	["neutral", /[😐😑]/gu],
	[
		"love",
		/[❤💕💖💗💘💝😍🥰💙💚💛💜🧡🤍🤎🩷🩵]|(?<![\p{L}\p{N}_])<3(?![\p{L}\p{N}_])/gu,
	],
	[
		"happy",
		/[🙂😊😀😃😄😁😆😂🤣🥳🎉🙌]|(?<![\p{L}\p{N}_]):-?\)(?![\p{L}\p{N}_])/gu,
	],
	["sad", /[😢😭😞😔💔]|(?<![\p{L}\p{N}_]):-?\((?![\p{L}\p{N}_])/gu],
	["concerned", /[😟😰😨😥😧😬]/gu],
	["angry", /[😠😡🤬]/gu],
	["surprised", /[😲😮😱🤯]/gu],
	["curious", /[🤔🧐]/gu],
	["wink", /[😉😜😝😛🤪]|(?<![\p{L}\p{N}_])(?:;-?\)|:-?p)(?![\p{L}\p{N}_])/giu],
	["oops", /[😅😳🫣🤭🙈]/gu],
];

function words(pattern: string): RegExp {
	return new RegExp(
		`(?<![\\p{L}\\p{N}_])(?:${pattern})(?![\\p{L}\\p{N}_])`,
		"giu",
	);
}

const WORD_CUES: readonly Cue[] = [
	["love", words("love|adore|liebe|lieb")],
	[
		"happy",
		words("happy|excited|wonderful|great|thanks|danke|glücklich|freue|super"),
	],
	["sad", words("sad|lonely|heartbroken|miss you|traurig|einsam|vermisse")],
	[
		"concerned",
		words("worried|scared|afraid|anxious|frustrated|besorgt|angst|frustriert"),
	],
	["angry", words("angry|furious|wütend")],
	["surprised", words("wow|surprised|unbelievable|überrascht|unglaublich")],
	["curious", words("why|how|wonder|confused|warum|wieso|verwirrt")],
	["wink", words("just kidding|nur spaß|nur spass")],
	["oops", words("oops|whoops|my bad|embarrassed|peinlich|ups|hoppla")],
	["curious", /[?？]/gu],
];

/** Source snippets, links, and quoted speech do not describe the writer's own feeling. */
export function reactionText(draft: string): string {
	return draft
		.replace(
			/```[\s\S]*?(?:```|(?![\s\S]))|~~~[\s\S]*?(?:~~~|(?![\s\S]))|`[^`\n]*(?:`|$)|(?:https?:\/\/|www\.)\S+|^\s{0,3}>[^\n]*/gimu,
			(match) => match.replace(/[^\n]/g, " "),
		)
		.replace(
			/"[^"\n]*(?:"|$)|“[^”\n]*(?:”|$)|„[^“\n]*(?:“|$)|«[^»\n]*(?:»|$)|(?<![\p{L}\p{N}_])'(?:[^'\n]|(?<=\p{L})'(?=\p{L}))*(?:'(?![\p{L}\p{N}_])|$)|‘(?:[^’\n]|(?<=\p{L})’(?=\p{L}))*(?:’(?![\p{L}\p{N}_])|$)/gmu,
			(match) => " ".repeat(match.length),
		);
}

/** Emoji and emoticons alone do not need the model to interpret surrounding wording. */
export function hasReactionProse(draft: string): boolean {
	let text = reactionText(draft);
	for (const [, pattern] of EXPLICIT_CUES) text = text.replace(pattern, " ");
	return /[\p{L}\p{N}]/u.test(text);
}

/** These explicit markers are enough to defer a literal positive reaction. */
export function hasSarcasmCue(draft: string): boolean {
	return /(?:^|\s)\/s(?=$|[\s.!?,])|\byeah[,\s]+right\b|\bas if\b/iu.test(
		reactionText(draft),
	);
}

function negated(text: string, index: number): boolean {
	// A new sentence or contrast can introduce a different feeling.
	const clause =
		text
			.slice(0, index)
			.split(/[.!?;\n,]|\b(?:but|however|aber|sondern)\b/iu)
			.at(-1) ?? "";
	return /\b(?:not|never|no|don't|doesn't|isn't|aren't|wasn't|can't|cannot|nicht|kein|keine|keinen|nie)\b(?:\s+[\p{L}\p{N}'-]+){0,2}\s*$/iu.test(
		clause.replaceAll("’", "'"),
	);
}

function findCues(
	text: string,
	cues: readonly Cue[],
	explicit: boolean,
): CueMatch[] {
	const matches: CueMatch[] = [];
	for (const [expression, pattern] of cues) {
		for (const match of text.matchAll(pattern)) {
			matches.push({
				expression,
				index: match.index,
				negated: negated(text, match.index),
				explicit,
			});
		}
	}
	return matches;
}

function compatible(a: BotExpression, b: BotExpression): boolean {
	const warm = (expression: BotExpression) =>
		expression === "happy" || expression === "love" || expression === "wink";
	return a === b || (warm(a) && warm(b));
}

function reactionCue(
	text: string,
): { latest: CueMatch; explicit?: CueMatch } | undefined {
	if (hasSarcasmCue(text)) return undefined;
	let start = 0;
	// A stated change in feeling supersedes earlier cues; punctuation alone does not.
	for (const change of text.matchAll(
		/\b(?:but|however|then|aber|sondern|dann|and now|und jetzt)\b|(?:^|[.!?;,\n])\s*(?:now|jetzt|nun)\b/giu,
	))
		start = change.index + change[0].length;
	const matches = [
		...findCues(text, EXPLICIT_CUES, true),
		...findCues(text, WORD_CUES, false),
	]
		.filter((cue) => cue.index >= start)
		.sort((a, b) => a.index - b.index);
	const latest = matches.at(-1);
	if (!latest || latest.negated || negated(text, text.length)) return undefined;
	for (const cue of matches.slice(0, -1)) {
		if (cue.negated) continue;
		if (
			!cue.explicit &&
			(cue.expression === "curious" || cue.expression === "surprised")
		)
			continue;
		if (!compatible(cue.expression, latest.expression)) return undefined;
	}
	return {
		latest,
		explicit: matches.filter((cue) => cue.explicit && !cue.negated).at(-1),
	};
}

/** Unambiguous explicit faces can react while the model interprets surrounding prose. */
export function explicitDraftCue(draft: string): BotExpression | undefined {
	return reactionCue(reactionText(draft))?.explicit?.expression;
}

/** Immediate local cues while the multilingual emotion model is loading or unavailable. */
export function draftCue(draft: string): BotExpression {
	if (!draft.trim()) return "neutral";
	const cue = reactionCue(reactionText(draft));
	return cue?.explicit?.expression ?? cue?.latest.expression ?? "curious";
}
