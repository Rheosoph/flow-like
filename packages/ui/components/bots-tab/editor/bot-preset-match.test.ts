import { describe, expect, test } from "bun:test";
import {
	normalizeBotPresetText,
	rankBotPresets,
	tokenizeBotPresetText,
} from "./bot-preset-match";
import { BOT_PRESETS, type BotPreset } from "./bot-presets";

const topId = (text: string) => rankBotPresets(text)[0]?.preset.id;

describe("rankBotPresets", () => {
	test("returns nothing for empty, blank or unrelated text", () => {
		expect(rankBotPresets("")).toEqual([]);
		expect(rankBotPresets("   \n ")).toEqual([]);
		expect(rankBotPresets("purple elephants dancing")).toEqual([]);
		expect(rankBotPresets("the and of my bot")).toEqual([]);
	});

	test.each([
		["book meetings in my calendar", "secretary"],
		["Recherche jeden Morgen zu Wettbewerbern", "research_digest"],
		["Sort my inbox and answer emails", "inbox_triage"],
		["Beantworte meine E-Mails und sortiere den Posteingang", "inbox_triage"],
		["analyse our sales data in spreadsheets", "data_analyst"],
		["Werte unsere Umsatzdaten aus und baue Diagramme", "data_analyst"],
		["test our website in the browser and report bugs", "web_tester"],
		["Teste den Checkout auf unserer Webseite", "web_tester"],
		["watch my app runs and alert me when something fails", "ops_watcher"],
		["Überwache meine Apps und warne mich bei Ausfällen", "ops_watcher"],
		["write scripts and run shell commands in the terminal", "coder"],
		["Programmiere Skripte für die Kommandozeile", "coder"],
		["a helper for everyday questions", "everyday_helper"],
		["Hilf mir im Alltag mit Fragen", "everyday_helper"],
		["Kalendereinträge verwalten und Termine verschieben", "secretary"],
		["Marktrecherche", "research_digest"],
	])("%s → %s", (text, expected) => {
		expect(topId(text)).toBe(expected);
	});

	test("ignores case and diacritics", () => {
		expect(topId("ÜBERWACHUNG")).toBe("ops_watcher");
		expect(topId("uberwachung")).toBe("ops_watcher");
		expect(topId("SEKRETÄRIN")).toBe("secretary");
		expect(topId("sekretarin")).toBe("secretary");
	});

	test("matches a word while it is still being typed", () => {
		expect(topId("calen")).toBe("secretary");
		expect(topId("I need a sekret")).toBe("secretary");
	});

	test("ranks best first and leaves out presets without a match", () => {
		const ranked = rankBotPresets("book meetings in my calendar");
		expect(ranked.map((match) => match.preset.id)).toEqual(["secretary"]);
		const scores = rankBotPresets("research news and analyse data").map(
			(match) => match.score,
		);
		expect(scores).toEqual([...scores].sort((a, b) => b - a));
		expect(scores.length).toBeGreaterThan(1);
	});

	test("keeps the preset order on ties", () => {
		const preset = (id: string): BotPreset => ({
			...BOT_PRESETS[0],
			id,
			keywords: { en: { report: 2 }, de: {} },
		});
		const presets = [preset("first"), preset("second"), preset("third")];
		expect(
			rankBotPresets("weekly report", presets).map((match) => match.preset.id),
		).toEqual(["first", "second", "third"]);
	});

	test("counts each word once per preset", () => {
		expect(rankBotPresets("email email email")[0]?.score).toBe(
			rankBotPresets("email")[0]?.score,
		);
	});
});

describe("text helpers", () => {
	test("normalizes case, accents and ß", () => {
		expect(normalizeBotPresetText("Überprüfen Straße CAFÉ")).toBe(
			"uberprufen strasse cafe",
		);
	});

	test("drops stopwords, short tokens and duplicates", () => {
		expect(tokenizeBotPresetText("Book a meeting, book it for me!")).toEqual([
			"book",
			"meeting",
		]);
		expect(tokenizeBotPresetText("Für die E-Mails")).toEqual(["mails"]);
	});
});
