import { describe, expect, test, vi } from "vitest";
import type { IMetadata, ITemplateSearchHit } from "../../lib";
import type { IBackendState } from "../../state/backend-state";
import {
	type TemplateMetadataEntry,
	TemplateMetadataReadError,
	type TemplateReadOptions,
} from "../../state/backend-state/template-read";
import { scoutForkPreview, scoutSearchTemplates } from "./scout-tools";

function owned(id: string, appId = "private"): TemplateMetadataEntry {
	return [
		appId,
		id,
		{
			name: `Invoice ${id}`,
			description: "Billing",
			tags: ["finance"],
		} as IMetadata,
	];
}

function publicHit(id: string, appId = "public"): ITemplateSearchHit {
	return {
		app_id: appId,
		template_id: id,
		metadata: owned(id)[2],
		app_allow_forking: true,
		app_price: 4,
		rating_sum: 0,
		rating_count: 0,
	};
}

function backend(
	publicHits: ITemplateSearchHit[] = [],
	ownedEntries: TemplateMetadataEntry[] = [],
) {
	const searchTemplates = vi.fn(async (query) =>
		publicHits.slice(query.offset, query.offset + query.limit),
	);
	const getTemplates = vi.fn(
		async (_app, _language, options: TemplateReadOptions) => {
			options.onCoverage?.({
				complete: false,
				scope: "test_window",
				warning: "First 100 memberships only.",
			});
			return ownedEntries;
		},
	);
	const getAppAuthoritative = vi.fn();
	return {
		templateState: { searchTemplates, getTemplates },
		appState: { getAppAuthoritative },
	};
}

async function search(
	state: ReturnType<typeof backend>,
	args: Record<string, unknown> = {},
) {
	return scoutSearchTemplates(state as unknown as IBackendState, {
		query: "invoice",
		...args,
	});
}

describe("Scout template search coverage", () => {
	test("includes private owned matches and merges public duplicates without inventing app metadata", async () => {
		const state = backend(
			[publicHit("shared", "private")],
			[owned("shared"), owned("local")],
		);
		const result = await search(state);
		expect(result.templates).toEqual(
			expect.arrayContaining([
				expect.objectContaining({ template_id: "local", sources: ["owned"] }),
				expect.objectContaining({
					template_id: "shared",
					sources: ["public", "owned"],
					app_price: 4,
				}),
			]),
		);
		expect(result.templates).toHaveLength(2);
		expect(
			(result.templates as Record<string, unknown>[]).find(
				(entry) => entry.template_id === "local",
			),
		).not.toHaveProperty("app_allow_forking");
		expect(state.templateState.searchTemplates).toHaveBeenCalledWith(
			expect.anything(),
			{ strict: true, readOnly: true },
		);
		expect(state.templateState.getTemplates).toHaveBeenCalledWith(
			undefined,
			undefined,
			expect.objectContaining({ strict: true, readOnly: true }),
		);
		expect(state.appState.getAppAuthoritative).not.toHaveBeenCalled();
	});

	test("preserves public failures while returning usable owned metadata", async () => {
		const state = backend([], [owned("local")]);
		state.templateState.searchTemplates.mockRejectedValueOnce(
			new Error("public hub unavailable"),
		);
		const result = await search(state);
		expect(result).toMatchObject({
			status: "ok",
			complete: false,
			coverage: {
				public: {
					complete: false,
					error: "public hub unavailable",
					next_offset: 0,
				},
				owned: { complete: false, next_offset: 1, observed_exhausted: true },
			},
		});
		expect(result.templates).toHaveLength(1);
		expect(result.note).toContain("not exhaustive account or public search");
	});

	test("keeps partial owned results and their failure alongside successful public results", async () => {
		const state = backend([publicHit("store")]);
		state.templateState.getTemplates.mockRejectedValueOnce(
			new TemplateMetadataReadError("owned API unavailable", [owned("cached")]),
		);
		const result = await search(state);
		expect(result.templates).toHaveLength(2);
		expect(result).toMatchObject({
			complete: false,
			coverage: {
				owned: { error: "owned API unavailable", complete: false },
				public: { complete: false, observed_exhausted: true },
			},
		});
	});

	test("reports both failures instead of a successful empty search", async () => {
		const state = backend();
		state.templateState.getTemplates.mockRejectedValueOnce(
			new Error("owned denied"),
		);
		state.templateState.searchTemplates.mockRejectedValueOnce(
			new Error("public denied"),
		);
		expect(await search(state)).toMatchObject({
			status: "error",
			complete: false,
			templates: [],
			coverage: {
				owned: { error: "owned denied" },
				public: { error: "public denied" },
			},
		});
	});

	test("advances independent cursors only through consumed rows under one cap", async () => {
		const state = backend(
			[publicHit("a"), publicHit("b"), publicHit("c")],
			[owned("c"), owned("a"), owned("b")],
		);
		const first = await search(state, { limit: 2 });
		expect(first.templates).toHaveLength(2);
		expect(first).toMatchObject({
			coverage: { public: { next_offset: 1 }, owned: { next_offset: 1 } },
		});
		const second = await search(state, {
			limit: 2,
			public_offset: 1,
			owned_offset: 1,
		});
		expect(second.templates).toEqual(
			expect.arrayContaining([
				expect.objectContaining({ app_id: "private", template_id: "b" }),
				expect.objectContaining({ app_id: "public", template_id: "b" }),
			]),
		);
		expect(second).toMatchObject({
			coverage: { public: { next_offset: 2 }, owned: { next_offset: 2 } },
		});
	});

	test("counts a merged duplicate in both source cursors", async () => {
		const state = backend(
			[publicHit("a", "private"), publicHit("b")],
			[owned("a"), owned("b")],
		);
		const result = await search(state, { limit: 1 });
		expect(result.templates).toHaveLength(1);
		expect(result).toMatchObject({
			coverage: { public: { next_offset: 1 }, owned: { next_offset: 1 } },
		});
	});

	test("bounds a full public page and preserves its numeric cursor when exhausted", async () => {
		const state = backend(
			Array.from({ length: 100 }, (_, index) => publicHit(String(index))),
		);
		const result = await search(state, { limit: 999 });
		expect(result.templates).toHaveLength(100);
		expect(result).toMatchObject({
			coverage: { public: { complete: false, next_offset: 100 } },
		});
		expect(
			await search(state, { limit: 100, public_offset: 100 }),
		).toMatchObject({
			coverage: {
				public: { complete: false, next_offset: 100, observed_exhausted: true },
			},
		});
	});

	test("reusing both returned cursors makes progress after either source is exhausted", async () => {
		const state = backend(
			[publicHit("first"), publicHit("second")],
			[owned("only")],
		);
		let cursors = { public_offset: 0, owned_offset: 0 };
		const seen: string[] = [];
		for (let page = 0; page < 4; page++) {
			const result = await search(state, { limit: 1, ...cursors });
			for (const entry of result.templates as { template_id: string }[])
				seen.push(entry.template_id);
			const coverage = result.coverage as Record<
				string,
				{ next_offset: number }
			>;
			cursors = {
				public_offset: coverage.public.next_offset,
				owned_offset: coverage.owned.next_offset,
			};
		}
		expect(seen).toEqual(["only", "first", "second"]);
		expect(cursors).toEqual({ public_offset: 2, owned_offset: 1 });
	});

	test("bounds template metadata text and tags and identifies truncation", async () => {
		const metadata = {
			name: "invoice".repeat(1000),
			description: "x".repeat(10_000),
			use_case: "x".repeat(10_000),
			tags: Array.from({ length: 100 }, () => "x".repeat(1000)),
		} as IMetadata;
		const hit = { ...publicHit("large"), metadata, app_name: "x".repeat(5000) };
		const result = await search(
			backend([hit], [["private", "large", metadata]]),
		);
		for (const item of result.templates as Record<string, unknown>[]) {
			expect(item.metadata_truncated).toBe(true);
			expect(item.name).toHaveLength(240);
			expect(item.description).toHaveLength(2000);
			expect(item.use_case).toHaveLength(1000);
			expect(item.tags).toHaveLength(12);
			expect((item.tags as string[])[0]).toHaveLength(80);
			if (item.app_name) {
				expect(item.app_name).toHaveLength(240);
				expect(item.app_name_truncated).toBe(true);
			}
		}
	});

	test("a short public page is an observed window, never a certified complete corpus", async () => {
		const result = await search(backend([publicHit("one")]));
		expect(result).toMatchObject({
			complete: false,
			coverage: {
				public: {
					complete: false,
					observed_exhausted: true,
					scope: "public_api_window",
					warning: expect.stringContaining("metadata consolidation"),
				},
			},
		});
	});

	test("applies owned query, tag, category and forkability filters with authoritative reads", async () => {
		const state = backend(
			[],
			[
				owned("a", "yes"),
				owned("b", "no"),
				["other", "c", { name: "Unrelated" } as IMetadata],
			],
		);
		state.appState.getAppAuthoritative.mockImplementation(async (id) => ({
			primary_category: "Finance",
			allow_forking: id === "yes",
		}));
		const result = await search(state, {
			category: "Finance",
			tag: "finance",
			forkable_only: true,
		});
		expect(result.templates).toEqual([
			expect.objectContaining({ app_id: "yes" }),
		]);
		expect(state.appState.getAppAuthoritative).toHaveBeenCalledTimes(2);
	});

	test("caps app filter reads and marks unverifiable matches incomplete", async () => {
		const state = backend(
			[],
			Array.from({ length: 30 }, (_, index) =>
				owned(String(index), `app${index}`),
			),
		);
		state.appState.getAppAuthoritative.mockRejectedValue(
			new Error("forbidden"),
		);
		expect(await search(state, { forkable_only: true })).toMatchObject({
			complete: false,
			templates: [],
			coverage: {
				owned: {
					complete: false,
					warning: expect.stringContaining("25 app reads failed"),
				},
			},
		});
		expect(state.appState.getAppAuthoritative).toHaveBeenCalledTimes(25);
	});

	test.each([-1, 1.5, Number.MAX_SAFE_INTEGER + 1, "1"])(
		"rejects invalid source offsets %s before backend reads",
		async (offset) => {
			const state = backend();
			expect(await search(state, { public_offset: offset })).toMatchObject({
				status: "error",
			});
			expect(state.templateState.searchTemplates).not.toHaveBeenCalled();
			expect(state.templateState.getTemplates).not.toHaveBeenCalled();
		},
	);
});

describe("Scout fork preview", () => {
	const preview = {
		user_can_fork: true,
		blocked_packages: [{ package_id: "chart-kit", block: "paid", price: 499 }],
	};
	const state = () =>
		({
			appState: { getForkPreview: vi.fn(async () => preview) },
		}) as unknown as IBackendState;

	test("relays the hub's preview, left-out packages included", async () => {
		expect(await scoutForkPreview(state(), { app_id: "source" }, true)).toEqual(
			{ status: "ok", preview },
		);
	});

	test("says that a paid package cannot be bought on a build that may not sell", async () => {
		const result = await scoutForkPreview(state(), { app_id: "source" }, false);

		expect(result.preview).toBe(preview);
		expect(result.note).toContain("cannot be bought here");
	});
});
