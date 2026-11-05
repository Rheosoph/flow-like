import { describe, expect, it } from "vitest";
import {
	draftCue,
	explicitDraftCue,
	hasReactionProse,
	hasSarcasmCue,
} from "./draft-cues";

describe("local draft reactions", () => {
	it.each([
		["", "neutral"],
		["   ", "neutral"],
		["Help me plan tomorrow", "curious"],
		["I love you", "love"],
		["Ich freue mich", "happy"],
		["I'm feeling sad", "sad"],
		["I am worried", "concerned"],
		["Wow!", "surprised"],
		["🥰", "love"],
		["😭", "sad"],
		["Why?", "curious"],
		["I was sad, now I am happy", "happy"],
		["I'm not happy", "curious"],
		["Ich bin nicht traurig", "curious"],
		["I'm not happy, but now I'm excited", "happy"],
		["Ich bin nicht traurig, aber wütend", "angry"],
		["Ich bin überrascht", "surprised"],
		["That’s my bad", "oops"],
		["just kidding", "wink"],
		["I'm furious", "angry"],
		["I'm frustrated", "concerned"],
		["😄 Actually, I'm not happy", "curious"],
	])("reacts to %j with %s", (draft, expected) => {
		expect(draftCue(draft)).toBe(expected);
	});

	it.each([
		["😉", "wink"],
		["You got me ;-) ", "wink"],
		["Can you help? 😉", "wink"],
		["Just playing :P", "wink"],
		["I mixed those up 😅", "oops"],
		["😡", "angry"],
		["I am worried 😟", "concerned"],
		["That means a lot 🩷", "love"],
		["Thank you <3", "love"],
		["😐", "neutral"],
		["Oh! 😮", "surprised"],
		["😢 I am still sad", "sad"],
		["😢 then 🥳", "happy"],
		["I'm not sad anymore. 🥳", "happy"],
		["Love it 🥰", "love"],
		["Thanks 😉", "wink"],
		["🥰 thanks", "love"],
		["I was 😢, now 🥳", "happy"],
		["I was sad, but I'm happy 😊", "happy"],
		["I don't know why I feel happy 😊", "happy"],
		["❤️😊", "happy"],
		['She said "I\'m sad 😢", but I feel happy 😊', "happy"],
	])("recognizes an explicit reaction in %j", (draft, expression) => {
		expect(explicitDraftCue(draft)).toBe(expression);
		expect(draftCue(draft)).toBe(expression);
	});

	it.each([
		"I am sad 😊",
		"I am worried 😂",
		"I'm happy 😢",
		"Ich bin traurig 😊",
		"😢😊",
		"😡😢",
		"I am sad now 😊",
		"Great, another failure /s",
		"Great, another failure 😊 /s",
		"Yeah right 😊",
		"As if I am happy 😊",
		'She said "I love you ❤️"',
		"She said 'I'm happy ❤️'",
		"She said ‘I’m happy ❤️’",
		"Sie sagte „Ich liebe dich ❤️“",
		"Elle dit « je t'aime ❤️ »",
		'She said "I love you ❤️',
		"> I love you ❤️",
	])("defers conflicting, sarcastic, or reported emotion in %j", (draft) => {
		expect(explicitDraftCue(draft)).toBeUndefined();
		expect(draftCue(draft)).toBe("curious");
	});

	it.each([
		["😉", false],
		["😉🥰!", false],
		["<3", false],
		[":P", false],
		[";-)", false],
		["`some code` 😉", false],
		['"reported ❤️" 😉', false],
		["https://example.test/text 😉", false],
		["I’m happy 😊", true],
		["I'm happy 😊", true],
		["Ich bin glücklich 😊", true],
		["我很开心 😊", true],
		["100 😉", true],
	])("detects surrounding prose in %j", (draft, expected) => {
		expect(hasReactionProse(draft)).toBe(expected);
	});

	it.each([
		["Great /s", true],
		["Great /s!", true],
		["Yeah, right", true],
		["as if", true],
		["https://example.test/s", false],
		['She wrote "yeah right /s"', false],
		["Use `/s`", false],
		["Thanks for your help!", false],
	])("recognizes explicit sarcasm markers in %j", (draft, expected) => {
		expect(hasSarcasmCue(draft)).toBe(expected);
	});

	it.each([
		"These words are happy",
		"🦄",
		"🧑🏽‍💻",
		"Look at `;-)`",
		"```js\nconst face = '😉';\n```",
		"~~~\n😡\n~~~",
		"Here is unfinished code `😅",
		"See https://example.test/;-) or www.example.test/😡",
		"The condition is x<3",
		"I was 😢, now I'm happy",
		"😉 can you help?",
		"I'm not happy 😄",
		"Ich bin nicht 😡",
		"😄 Actually, I’m not happy",
	])(
		"leaves ambiguous wording and source snippets to the model: %j",
		(draft) => {
			expect(explicitDraftCue(draft)).toBeUndefined();
		},
	);

	it("ignores technical examples for the local fallback as well", () => {
		expect(
			draftCue("Compare `happy` with `sad` at https://example.test/love"),
		).toBe("curious");
		expect(draftCue("The code is `;-)`, but thank you 🥰")).toBe("love");
	});
});
