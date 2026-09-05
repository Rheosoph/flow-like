import { describe, expect, test } from "bun:test";
import {
	clearProfileTemplateDraft,
	readProfileTemplateDraft,
	writeProfileTemplateDraft,
} from "./profile-template-drafts";
import { createProfileTemplate } from "./profile-template-model";

describe("profile template session drafts", () => {
	test("restores unsaved fields and their baseline without mixing hubs, viewers, or templates", () => {
		const key = JSON.stringify([
			"https://first.example",
			"viewer-a",
			"template-a",
		]);
		const baseline = createProfileTemplate("first.example");
		const draft = { ...baseline, name: "Unsaved name" };
		writeProfileTemplateDraft(key, { draft, baseline });
		expect(readProfileTemplateDraft(key)).toEqual({ draft, baseline });
		expect(
			readProfileTemplateDraft(
				JSON.stringify(["https://second.example", "viewer-a", "template-a"]),
			),
		).toBeUndefined();
		expect(
			readProfileTemplateDraft(
				JSON.stringify(["https://first.example", "viewer-b", "template-a"]),
			),
		).toBeUndefined();
		expect(
			readProfileTemplateDraft(
				JSON.stringify(["https://first.example", "viewer-a", "template-b"]),
			),
		).toBeUndefined();
		clearProfileTemplateDraft(key);
	});
	test("cache snapshots remain independent and explicit discard removes the draft", () => {
		const key = "independent-draft";
		const baseline = createProfileTemplate("hub.example");
		const draft = { ...baseline, bits: ["hub.example:model"] };
		writeProfileTemplateDraft(key, { draft, baseline });
		draft.bits.push("later:model");
		const restored = readProfileTemplateDraft(key);
		expect(restored?.draft.bits).toEqual(["hub.example:model"]);
		restored?.draft.bits.push("changed-restoration:model");
		expect(readProfileTemplateDraft(key)?.draft.bits).toEqual([
			"hub.example:model",
		]);
		clearProfileTemplateDraft(key);
		expect(readProfileTemplateDraft(key)).toBeUndefined();
	});
	test("bounds session memory and keeps the most recently edited drafts", () => {
		const baseline = createProfileTemplate();
		for (let index = 0; index < 21; index++)
			writeProfileTemplateDraft(`bounded-${index}`, {
				draft: { ...baseline, name: `Draft ${index}` },
				baseline,
			});
		expect(readProfileTemplateDraft("bounded-0")).toBeUndefined();
		expect(readProfileTemplateDraft("bounded-20")?.draft.name).toBe("Draft 20");
		for (let index = 0; index < 21; index++)
			clearProfileTemplateDraft(`bounded-${index}`);
	});
});
