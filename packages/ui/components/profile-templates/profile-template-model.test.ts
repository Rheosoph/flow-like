import { describe, expect, test } from "bun:test";
import { IBitTypes } from "../../lib/schema/bit/bit";
import {
	IConnectionMode,
	type IProfile,
} from "../../lib/schema/profile/profile";
import {
	createProfileTemplate,
	filterProfileTemplates,
	prepareProfileTemplate,
} from "./profile-template-model";

function savedTemplate(): IProfile {
	return {
		id: "research-starter",
		name: "Research",
		created: "2025-01-01T00:00:00.000Z",
		updated: "2025-02-01T00:00:00.000Z",
		hub: "hub.example",
		secure: false,
		bits: ["other-hub:private-reference", "hub.example:model"],
		apps: [
			{
				app_id: "saved-app",
				favorite: true,
				pinned: true,
				favorite_order: 7,
				pinned_order: 3,
				saved_setting: { view: "board" },
			},
		],
		settings: {
			connection_mode: IConnectionMode.Step,
			editor: { snap: false },
			custom_default: "keep",
		},
		theme: { accent: "violet" },
		hubs: ["secondary.example"],
		tags: ["Knowledge"],
		interests: ["Research"],
		custom_bits: [
			{
				id: "secret-model",
				hub: "hub.example",
				type: IBitTypes.Llm,
				authors: [],
				created: "",
				updated: "",
				dependencies: [],
				dependency_tree_hash: "",
				hash: "",
				meta: {},
				parameters: { api_key: "fixture-secret" },
			},
		],
		shortcuts: [
			{
				id: "shortcut",
				profileId: "personal-profile",
				label: "Personal link",
				path: "/private",
				order: 0,
				createdAt: "2025-01-01",
			},
		],
		home_layout: { version: 1, widgets: [] },
		home_default_id: "research-starter",
	};
}

describe("profile template model", () => {
	test("duplicating creates a new identity without private models, personal shortcuts, or home overrides", () => {
		const source = savedTemplate();
		const copy = createProfileTemplate("fallback.example", source);
		expect(copy.id).toBeTruthy();
		expect(copy.id).not.toBe(source.id);
		expect(copy.name).toBe("Research copy");
		expect(copy.created).not.toBe(source.created);
		expect(copy.updated).not.toBe(source.updated);
		expect(copy.custom_bits).toBeUndefined();
		expect(copy.shortcuts).toBeUndefined();
		expect(copy.home_layout).toBeNull();
		expect(copy.home_default_id).toBeNull();
		expect(JSON.stringify(copy)).not.toContain("fixture-secret");
	});

	test("duplicate app settings and nested editor defaults remain independent of the original", () => {
		const source = savedTemplate();
		const original = structuredClone(source);
		const copy = createProfileTemplate("fallback.example", source);
		expect(copy.apps).toEqual(source.apps);
		expect(copy.settings).toEqual(source.settings);
		expect(copy.bits).toEqual(source.bits);
		expect(copy.hub).toBe(source.hub);
		expect(copy.secure).toBe(false);
		if (!copy.apps || !copy.settings)
			throw new Error("Missing copied configuration");
		copy.apps[0].favorite_order = 9;
		copy.apps[0].saved_setting.view = "table";
		copy.settings.editor.snap = true;
		copy.bits.push("new-hub:model");
		expect(source).toEqual(original);
	});

	test("saving identity changes preserves app flags, app ordering, custom settings, and unknown bit references", () => {
		const source = savedTemplate();
		const original = structuredClone(source);
		const draft = {
			...source,
			name: "  Research team  ",
			description: "  First line\nSecond line  ",
			hub: " hub.example ",
		};
		const payload = prepareProfileTemplate(draft);
		expect(payload.name).toBe("Research team");
		expect(payload.description).toBe("First line\nSecond line");
		expect(payload.hub).toBe("hub.example");
		expect(payload.id).toBe(source.id);
		expect(payload.created).toBe(source.created);
		expect(payload.apps).toEqual(source.apps);
		expect(payload.settings).toEqual(source.settings);
		expect(payload.bits).toEqual(source.bits);
		expect(payload.theme).toEqual(source.theme);
		expect(payload.secure).toBe(false);
		expect(source).toEqual(original);
	});

	test("blank optional app configuration is preserved rather than resetting saved server defaults", () => {
		const source = savedTemplate();
		source.apps = null;
		expect(prepareProfileTemplate(source).apps).toBeNull();
		const payload = prepareProfileTemplate({ ...source, apps: [] });
		expect(payload.apps).toEqual([]);
	});

	test("filtering and sorting do not reorder or mutate the cached source templates", () => {
		const first = savedTemplate();
		const second = {
			...savedTemplate(),
			id: "operations",
			name: "Operations",
			tags: ["Monitoring"],
			updated: "2026-01-01T00:00:00.000Z",
		};
		const source = Object.freeze([first, second]);
		expect(
			filterProfileTemplates([...source], " monitoring ", "name").map(
				(item) => item.id,
			),
		).toEqual(["operations"]);
		expect(
			filterProfileTemplates([...source], "", "updated").map((item) => item.id),
		).toEqual(["operations", "research-starter"]);
		expect(source.map((item) => item.id)).toEqual([
			"research-starter",
			"operations",
		]);
	});
});
