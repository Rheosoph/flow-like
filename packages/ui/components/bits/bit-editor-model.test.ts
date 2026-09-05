import { describe, expect, test } from "bun:test";
import type { IBit } from "../../lib/schema/bit/bit";
import { IBitTypes } from "../../lib/schema/bit/bit";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	clone,
	coreChanged,
	emptyMetadata,
	saveAdminBit,
	splitBitSecrets,
	validateBitDraft,
} from "./bit-editor-model";
function fixture(): IBit {
	return {
		id: "bit-one",
		authors: [],
		dependencies: [],
		dependency_tree_hash: "tree",
		hash: "hash",
		hub: "hub",
		type: IBitTypes.Llm,
		created: "now",
		updated: "now",
		parameters: {
			context_length: 2048,
			provider: {
				provider_name: "Hosted",
				api_surface: null,
				params: { tier: "team", custom: true },
			},
			unknown: { preserve: [1, 2] },
		},
		meta: {
			en: { ...emptyMetadata(), name: "One" },
			de: {
				...emptyMetadata(),
				name: "Eins",
				preview_media: ["https://example.invalid/old.png"],
			},
		},
	};
}
const profile = { id: "profile" } as IProfile;
function apiFixture(source: IBit) {
	const calls: string[] = [];
	const state = { failLanguage: "", noResult: false, newId: "" };
	const api = {
		stream: async (
			_profile: unknown,
			route: string,
			options: RequestInit,
			callback: (data: unknown) => void,
		) => {
			calls.push(route);
			callback({
				stage: "download",
				message: "Downloading artifact",
				percent: 50,
			});
			if (!state.noResult)
				callback({
					...JSON.parse(options.body as string),
					id: state.newId || source.id,
					meta: {},
				});
		},
		put: async (_profile: unknown, route: string) => {
			calls.push(route);
			if (route.endsWith(state.failLanguage) && state.failLanguage)
				throw new Error("Metadata failed");
		},
	} as IApiState;
	return { api, calls, state };
}
describe("bit editor persistence", () => {
	test("metadata-only changes save the changed locale without upserting the artifact", async () => {
		const original = fixture();
		const draft = clone(original);
		draft.meta.de.name = "Geändert";
		draft.meta.de.preview_media = [];
		const { api, calls } = apiFixture(original);
		const saved = await saveAdminBit(api, profile, original, draft);
		expect(calls).toEqual(["admin/bit/bit-one/de"]);
		expect(saved.meta.de.preview_media).toEqual([]);
		expect(saved.parameters).toEqual(original.parameters);
		expect(saved.meta.en).toEqual(original.meta.en);
	});
	test("retry skips a completed core save and successful locales", async () => {
		let persisted = fixture();
		const draft = clone(persisted);
		draft.version = "2";
		draft.meta.en.name = "New";
		draft.meta.de.name = "Neu";
		const { api, calls, state } = apiFixture(persisted);
		state.failLanguage = "/de";
		await expect(
			saveAdminBit(api, profile, persisted, draft, (bit) => {
				persisted = bit;
			}),
		).rejects.toThrow("Metadata failed");
		expect(persisted.version).toBe("2");
		expect(persisted.meta.en.name).toBe("New");
		state.failLanguage = "";
		await saveAdminBit(api, profile, persisted, draft);
		expect(calls).toEqual([
			"admin/bit/bit-one",
			"admin/bit/bit-one/en",
			"admin/bit/bit-one/de",
			"admin/bit/bit-one/de",
		]);
	});
	test("new artifact identities receive all locales and checkpoints keep pending locales on retry", async () => {
		let persisted = fixture();
		const draft = clone(persisted);
		draft.download_link = "https://example.invalid/new.gguf";
		const { api, state } = apiFixture(persisted);
		state.newId = "bit-two";
		state.failLanguage = "/de";
		await expect(
			saveAdminBit(api, profile, persisted, draft, (bit) => {
				persisted = bit;
			}),
		).rejects.toThrow();
		expect(persisted.id).toBe("bit-two");
		expect(persisted.meta.en.name).toBe("One");
		expect(persisted.meta.de).toBeUndefined();
	});
	test("incomplete streams never report saved or start metadata writes", async () => {
		const original = fixture();
		const draft = { ...original, version: "2" };
		const { api, calls, state } = apiFixture(original);
		state.noResult = true;
		await expect(saveAdminBit(api, profile, original, draft)).rejects.toThrow(
			"did not complete",
		);
		expect(calls).toHaveLength(1);
	});
	test("custom credentials are separated without modifying nested configuration or the source", () => {
		const original = fixture();
		original.parameters.provider.params.api_key = "fixture-key";
		original.parameters.provider.params.headers = {
			authorization: "fixture-value",
		};
		const { bit, secrets } = splitBitSecrets(original);
		expect(secrets).toEqual({
			api_key: "fixture-key",
			headers: { authorization: "fixture-value" },
		});
		expect(bit.parameters.provider.params).toEqual({
			tier: "team",
			custom: true,
		});
		expect(bit.parameters.unknown).toEqual({ preserve: [1, 2] });
		expect(bit.parameters.provider.api_surface).toBeNull();
		expect(original.parameters.provider.params.api_key).toBe("fixture-key");
	});
	test("legacy metadata can be edited without normalizing untouched model parameters", () => {
		const original = fixture();
		const { context_length: _context, ...legacyParameters } =
			original.parameters;
		original.parameters = legacyParameters;
		const draft = clone(original);
		draft.meta.en.name = "Renamed";
		expect(validateBitDraft(draft, "admin", original)).toBeNull();
		expect(coreChanged(original, draft)).toBe(false);
		draft.parameters.context_length = -1;
		expect(validateBitDraft(draft, "admin", original)).toContain(
			"positive whole number",
		);
	});
	test("MLX dependencies differ for user manifests and registry bits", () => {
		const draft = fixture();
		draft.parameters.provider.provider_name = "MLX";
		expect(validateBitDraft(draft, "admin")).toContain("dependency");
		expect(validateBitDraft(draft, "custom")).toBeNull();
	});
});
