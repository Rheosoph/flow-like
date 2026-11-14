import { expect, test } from "bun:test";
import { callSpeechText } from "./call-speech-text";

test("speech keeps prose, link labels and inline terms without their formatting", () => {
	expect(
		callSpeechText(
			'# Ergebnis\n> **Fertig.** Lies [die __Anleitung__](https://example.test/a_(b) "Titel mit ) Klammer").\n- [x] Nutze `node_id`.\n    - _Danach_ weiter.\n😀 2 < 3, #123 und C# bleiben erhalten.',
		),
	).toBe(
		"Ergebnis\nFertig. Lies die Anleitung.\nNutze node_id.\nDanach weiter.\n😀 2 < 3, #123 und C# bleiben erhalten.",
	);
});

test("speech omits images, references, destinations and markup while keeping surrounding prose", () => {
	expect(
		callSpeechText(
			'Hier ist [die Anleitung][guide]. ![Bild](image.png)\n[guide]: https://example.test "Titel"\n<https://example.test>\nhttps://example.test/long?q=1\nwww.example.test\n<a href="https://example.test">Weiter</a> &amp; &#x1f600; &#252;berall.\n<script>hidden()</script><a2ui>{"id":"hidden"}</a2ui>',
		),
	).toBe("Hier ist die Anleitung.\nWeiter & 😀 überall.");
});

test("fences close only with the matching marker and sufficient length", () => {
	expect(
		callSpeechText(
			'Vorher.\n> ````a2ui\n> {"id":"hidden"}\n> ```\n> hidden\n> ~~~~\n> hidden\n> ````\nDanach.\n~~~json\n{"never":"closed"}',
		),
	).toBe("Vorher.\nDanach.");
});

test("omitting a parenthesized URL or self-closing widget preserves following prose", () => {
	expect(
		callSpeechText("Mehr Infos (https://example.test/a_(b)) stehen im Chat."),
	).toBe("Mehr Infos stehen im Chat.");
	expect(callSpeechText("<a2ui /> Danach kommt die Zusammenfassung.")).toBe(
		"Danach kommt die Zusammenfassung.",
	);
});

test("indented code, tables and rules never become speech", () => {
	expect(
		callSpeechText(
			"Vorher.\n    const hidden = true;\n\thidden();\n| A | B |\n| --- | --- |\n---\n***\nDanach.",
		),
	).toBe("Vorher.\nDanach.");
});

test("unfinished links, code and tags stay hidden when a reply ends early", () => {
	for (const suffix of [
		"[Anleitung](https://example.test/",
		'```a2ui\n{"type":"text"}',
		"`unfinished code",
		'<a2ui>{"id":"hidden"}',
		'<a href="https://example.test/',
	]) {
		expect(callSpeechText(`Vorher.\n${suffix}`)).toBe("Vorher.");
	}
});

test("every streamed prefix keeps the same speech prefix as the completed reply", () => {
	for (const text of [
		"Die **Antwort** ist __bereit__. Nutze `node_id` weiter.",
		'Hier ist [die Anleitung](https://example.test/a_(b) "Titel mit ) Klammer"). Weiter.',
		"Hier ist [die Anleitung][guide].\n[guide]: https://example.test",
		"Die Anleitung steht hier: https://example.test/guide. Weiter.",
		"Mehr Infos (https://example.test/a_(b)) stehen im Chat.",
		"Hier: <https://example.test>. Weiter.",
		'Hier: <a2ui>{"type":"text"}</a2ui> Weiter.',
		'Fertig.\n~~~a2ui\n{"id":"hidden"}\n~~~\nWeiter.',
		"Hier sind &amp; und &#252; richtig.",
	]) {
		const complete = callSpeechText(text);
		for (let end = 1; end <= text.length; end++) {
			const prefix = callSpeechText(text.slice(0, end), { streaming: true });
			expect(complete.startsWith(prefix)).toBe(true);
		}
	}
});
