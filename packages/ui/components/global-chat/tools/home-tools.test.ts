import { describe, expect, test } from "vitest";
import type { IBackendState } from "../../../state/backend-state";
import type { IHomeLayout } from "../../home/types";
import {
	getHomeWidgetCatalog,
	homeLayoutComparisonFields,
	listHomeDataSources,
	publicHomeLayoutValidation,
	validateHomeLayoutCandidate,
	validateHomeLayoutReferences,
	validateUnknownHomeWidgetConfigPreservation,
	validateUnknownHomeWidgetPreservation,
} from "./home-tools";

const layout = (widgets: IHomeLayout["widgets"]): IHomeLayout => ({
	version: 1,
	widgets,
});

const widget = (
	id: string,
	type = "information",
	config: Record<string, unknown> = {},
): IHomeLayout["widgets"][number] => ({
	id,
	type,
	title: id,
	size: { columns: 6, rows: 3 },
	appearance: { variant: "card", accent: "neutral" },
	config,
});

describe("Home FlowPilot tools", () => {
	test("catalog filtering returns exact creation templates", () => {
		const result = getHomeWidgetCatalog({
			category: "assistant",
			query: "hero",
		});
		expect(result).toMatchObject({
			status: "ok",
			total: 1,
			widget_type_total: 1,
			widget_types: {
				flowpilot: {
					type: "flowpilot",
					additional_properties: false,
					fields: {
						mode: { type: "string", enum: ["orb", "bar", "card", "hero"] },
					},
				},
			},
			layout_contract: {
				version: 1,
				max_widgets: 80,
				max_bytes: 131_072,
				grid_columns: { mobile: 1, tablet: 6, desktop: 12 },
				height_modes: ["auto", "content", "fixed"],
				class_name: {
					field: "appearance.className",
					optional: true,
					max_bytes: 1024,
				},
				css: { field: "appearance.css", optional: true, max_bytes: 8192 },
				surface_variables: {
					names: expect.arrayContaining([
						"--home-accent",
						"--home-surface-item",
					]),
				},
				theme_colors: {
					names: expect.arrayContaining(["--primary", "--chart-5"]),
				},
			},
			data_options: {
				aggregations: expect.arrayContaining(["count", "sum", "median"]),
				filter_operators: expect.arrayContaining(["eq", "contains", "empty"]),
			},
			renderings: {
				apps: expect.arrayContaining([
					expect.objectContaining({ id: "carousel" }),
				]),
				models: expect.arrayContaining([
					expect.objectContaining({ id: "list" }),
				]),
				packages: expect.arrayContaining([
					expect.objectContaining({ id: "featured" }),
				]),
			},
		});
		expect(result.presets?.[0]).toMatchObject({
			preset_id: "flowpilot-hero",
			widget: {
				type: "flowpilot",
				size: { columns: 12, rows: 4 },
				config: { mode: "hero" },
			},
		});
	});

	test("every catalog template satisfies local presentation constraints", () => {
		const catalog = getHomeWidgetCatalog({});
		if (!catalog.presets) throw new Error("Catalog was unavailable");
		const bindingErrors = new Set([
			"app_embed_missing_app",
			"app_embed_missing_event",
			"data_boxplot_field_missing",
			"data_calendar_fields_invalid",
			"data_date_field_missing",
			"data_group_series_missing",
			"data_histogram_field_missing",
			"data_kanban_group_missing",
			"data_record_date_missing",
			"data_source_missing_app",
			"data_table_missing",
			"data_target_missing",
			"data_xy_fields_missing",
		]);
		for (const preset of catalog.presets) {
			const validation = validateHomeLayoutCandidate(
				layout([{ id: preset.preset_id, ...preset.widget }]),
			);
			const presentationErrors = validation.issues.filter(
				(entry) => entry.severity === "error" && !bindingErrors.has(entry.code),
			);
			expect(presentationErrors, preset.preset_id).toEqual([]);
		}
	});

	test("candidate validation canonicalizes optional fields and rejects bad presentation values", () => {
		const normalized = validateHomeLayoutCandidate({
			version: 1,
			widgets: [{ id: "welcome", type: "greeting" }],
		});
		expect(normalized).toMatchObject({
			status: "ok",
			valid: true,
			canonical_layout: {
				widgets: [
					{
						id: "welcome",
						size: { columns: 6, rows: 3 },
						appearance: { variant: "card", accent: "neutral" },
					},
				],
			},
		});
		expect(normalized.fingerprint).toMatch(/^fp1:[0-9a-f]{32}$/);

		const invalid = validateHomeLayoutCandidate(
			layout([
				{
					...widget("bad"),
					appearance: { variant: "glass", accent: "magenta" },
				},
			]),
		);
		expect(invalid.valid).toBe(false);
		expect(invalid.issues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"unknown_widget_variant",
				"unknown_widget_accent",
			]),
		);

		for (const [type, variant] of [
			["app-collection", "icons"],
			["app-collection", "grid"],
			["packages", "grid"],
			["models", "list"],
			["quick-links", "list"],
		] as const) {
			const legacy = widget(`legacy-${type}-${variant}`, type);
			legacy.appearance.variant = variant;
			expect(validateHomeLayoutCandidate(layout([legacy])).valid).toBe(true);
		}
	});

	test("candidate validation keeps widget classes and flags ones the layout owns", () => {
		const styled = (className: unknown) =>
			validateHomeLayoutCandidate(
				layout([
					{
						...widget("styled"),
						appearance: {
							variant: "card",
							accent: "neutral",
							className,
						} as IHomeLayout["widgets"][number]["appearance"],
					},
				]),
			);
		const codes = (result: ReturnType<typeof validateHomeLayoutCandidate>) =>
			result.issues.map((entry) => entry.code);

		const clean = styled(
			"bg-linear-to-br from-card to-primary/10 md:hover:shadow-lg [&_h2]:text-lg [&:hover_p]:text-primary hidden md:block bg-[url(https://x.test/a.png)]",
		);
		expect(clean.valid).toBe(true);
		expect(clean.issues).toEqual([]);

		const normalized = styled("  bg-card\n\ttext-primary  ");
		expect(normalized.valid).toBe(true);
		expect(normalized.canonical_layout?.widgets[0].appearance).toEqual({
			variant: "card",
			accent: "neutral",
			className: "bg-card text-primary",
		});
		expect(codes(normalized)).toEqual(["home_layout_normalized"]);
		expect(codes(styled("bg-card text-primary"))).toEqual([]);
		expect(
			styled("   ").canonical_layout?.widgets[0].appearance,
		).not.toHaveProperty("className");

		for (const invalid of [42, ["bg-card"], { base: "bg-card" }, null]) {
			const result = styled(invalid);
			expect(result.valid).toBe(false);
			expect(result.issues).toContainEqual(
				expect.objectContaining({
					severity: "error",
					code: "home_widget_class_name_type_invalid",
					path: "$.widgets[0].appearance.className",
				}),
			);
		}

		const outOfScope = styled(
			"bg-card absolute md:z-10 !-z-10 z-20! lg:col-span-full row-start-2 self-end [&~div]:hidden [&+section]:mt-2 relative",
		);
		expect(outOfScope.valid).toBe(true);
		expect(outOfScope.issues).toEqual([
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_class_name_out_of_scope",
				path: "$.widgets[0].appearance.className",
			}),
		]);
		expect(outOfScope.issues[0].message).toContain(
			": absolute, md:z-10, !-z-10, z-20!, lg:col-span-full, row-start-2, self-end, [&~div]:hidden, [&+section]:mt-2, relative.",
		);

		expect(
			codes(
				styled(
					"before:absolute before:inset-0 md:after:z-10 [&_span]:absolute [&>div]:relative [&::before]:absolute *:self-end **:col-span-2 first-letter:float-left",
				),
			),
		).toEqual([]);
		const escaping = styled(
			"fixed before:fixed [&_span]:fixed [&:hover]:absolute hover:z-10",
		);
		expect(codes(escaping)).toEqual(["home_widget_class_name_out_of_scope"]);
		expect(escaping.issues[0].message).toContain(
			": fixed, before:fixed, [&_span]:fixed, [&:hover]:absolute, hover:z-10.",
		);

		expect(codes(styled("x".repeat(1024)))).toEqual([]);
		for (const oversized of ["x".repeat(1025), "é".repeat(513)]) {
			const result = styled(oversized);
			expect(result.valid).toBe(false);
			expect(result.canonical_layout).toBeUndefined();
			expect(result.issues).toEqual([
				expect.objectContaining({
					severity: "error",
					code: "home_layout_invalid",
					message: "Widget 1 appearance className must not exceed 1024 bytes.",
				}),
			]);
		}
	});

	test("candidate validation keeps widget CSS and flags parts that leave the widget", () => {
		const styled = (css: unknown) =>
			validateHomeLayoutCandidate(
				layout([
					{
						...widget("styled"),
						appearance: {
							variant: "card",
							accent: "neutral",
							css,
						} as IHomeLayout["widgets"][number]["appearance"],
					},
				]),
			);
		const codes = (result: ReturnType<typeof validateHomeLayoutCandidate>) =>
			result.issues.map((entry) => entry.code);

		const clean = styled(
			"@keyframes glow { to { opacity: 0.6; } }\n:root { background: var(--card); animation: glow 2s; }\n:root:is(.dark *) { color: white; }\n:root:hover > h2 ~ p, h2 + p { width: 50%; }\n:root { h2 { height: 2rem; } & > p ~ p { margin: 0; } }",
		);
		expect(clean.valid).toBe(true);
		expect(clean.issues).toEqual([]);
		expect(clean.canonical_layout?.widgets[0].appearance.css).toBe(
			"@keyframes glow { to { opacity: 0.6; } }\n:root { background: var(--card); animation: glow 2s; }\n:root:is(.dark *) { color: white; }\n:root:hover > h2 ~ p, h2 + p { width: 50%; }\n:root { h2 { height: 2rem; } & > p ~ p { margin: 0; } }",
		);
		expect(codes(styled(" \n "))).toEqual(["home_layout_normalized"]);

		for (const invalid of [42, [":root {}"], null]) {
			const result = styled(invalid);
			expect(result.valid).toBe(false);
			expect(result.issues).toContainEqual(
				expect.objectContaining({
					severity: "error",
					code: "home_widget_css_type_invalid",
					path: "$.widgets[0].appearance.css",
				}),
			);
		}

		const outOfScope = styled(
			"@property --angle { syntax: '<angle>'; inherits: false; initial-value: 0deg; }\n:root ~ section { opacity: 0; }\nbody.dark + * { color: red; }\n:root { position: fixed; z-index: 50; & + div { display: none; } }\n@media (min-width: 600px) { :root:hover { height: 900px; } }",
		);
		expect(outOfScope.valid).toBe(true);
		expect(outOfScope.issues).toEqual([
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_css_out_of_scope",
				path: "$.widgets[0].appearance.css",
			}),
		]);
		expect(outOfScope.issues[0].message).toContain(
			": @property, :root ~ section, body.dark + *, position on :root, z-index on :root, & + div, height on :root, position: fixed.",
		);

		const decoration = styled(
			':root { isolation: isolate; }\n:root::before { content: ""; position: absolute; inset: 0; z-index: -1; width: 100%; height: 4px; }\n:root:is(.dark *)::after, body:after { position: absolute; top: 0; }\n:root:first-letter { width: 1em; }',
		);
		expect(decoration.issues).toEqual([]);
		const fixedInside = styled("p { position: fixed; }");
		expect(codes(fixedInside)).toEqual(["home_widget_css_out_of_scope"]);
		expect(fixedInside.issues[0].message).toContain(": position: fixed.");

		const tailwind = styled(
			":root { @apply bg-card; color: theme(--color-primary); background: --alpha(var(--primary) / 50%); padding: --spacing(4); }\n@theme { --color-brand: red; }\n@tailwind utilities;",
		);
		expect(tailwind.valid).toBe(true);
		expect(tailwind.issues).toEqual([
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_css_tailwind_syntax",
				path: "$.widgets[0].appearance.css",
			}),
		]);
		expect(tailwind.issues[0].message).toContain(
			": @apply, @theme, @tailwind, theme(), --alpha(), --spacing().",
		);

		const wrapped = styled(
			":root { background: hsl(var(--primary)); color: rgb( var(--foreground) / 0.8); border-color: oklch(var(--home-accent)); --ring: HSL(var(--card)); outline-color: color-mix(in oklab, var(--border) 50%, transparent); }",
		);
		expect(wrapped.valid).toBe(true);
		expect(wrapped.issues).toEqual([
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_css_wrapped_color_variable",
				path: "$.widgets[0].appearance.css",
			}),
		]);
		expect(wrapped.issues[0].message).toContain(
			": hsl(var(--primary)), rgb(var(--foreground)), oklch(var(--home-accent)), hsl(var(--card)).",
		);

		const broken = styled(":root { color: red;");
		expect(broken.valid).toBe(true);
		expect(codes(broken)).toEqual(["home_widget_css_invalid"]);

		expect(codes(styled(`:root{}${" ".repeat(8185)}`))).toEqual([]);
		const oversized = styled(`:root{}${" ".repeat(8186)}`);
		expect(oversized.valid).toBe(false);
		expect(oversized.issues).toContainEqual(
			expect.objectContaining({
				severity: "error",
				code: "home_layout_invalid",
				message: "Widget 1 appearance css must not exceed 8192 bytes.",
			}),
		);

		const withStyling = (
			id: string,
			styling: { className?: string; css?: string } = {},
		) => ({
			...widget(id),
			appearance: { variant: "card", accent: "neutral", ...styling },
		});
		const current = layout([
			withStyling("styled", {
				className: "bg-card",
				css: ":root { color: red; }",
			}),
			withStyling("classes", { className: "bg-card" }),
			withStyling("plain"),
		]);
		const dropped = validateHomeLayoutCandidate(
			layout([
				withStyling("styled"),
				withStyling("classes", {
					className: "bg-primary",
					css: "p { margin: 0; }",
				}),
				withStyling("plain"),
				withStyling("new"),
			]),
			current,
		);
		expect(dropped.valid).toBe(true);
		expect(dropped.issues).toEqual([
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_styling_removed",
				path: "$.widgets[0].appearance.className",
			}),
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_styling_removed",
				path: "$.widgets[0].appearance.css",
			}),
		]);
		expect(validateHomeLayoutCandidate(current, current).issues).toEqual([]);
	});

	test("public validation and context comparisons avoid duplicate layout payloads", () => {
		const current = layout([
			widget("large", "information", {
				mode: "markdown",
				body: "x".repeat(100_000),
			}),
		]);
		const validation = validateHomeLayoutCandidate(current);
		const publicResult = publicHomeLayoutValidation(validation);
		expect(Object.hasOwn(publicResult, "layout")).toBe(false);
		expect(publicResult.canonical_layout).toEqual(current);
		expect(
			JSON.stringify(validation).length - JSON.stringify(publicResult).length,
		).toBeGreaterThan(99_000);

		const base = layout([widget("base")]);
		const defaults = layout([widget("default")]);
		const compact = homeLayoutComparisonFields(current, base, defaults, false);
		expect(compact).not.toHaveProperty("base_layout");
		expect(compact).not.toHaveProperty("default_layout");
		expect(compact).toMatchObject({
			base_fingerprint: expect.stringMatching(/^fp1:/),
			default_fingerprint: expect.stringMatching(/^fp1:/),
		});
		expect(
			homeLayoutComparisonFields(current, base, defaults, true),
		).toMatchObject({
			base_layout: base,
			default_layout: defaults,
		});
		expect(
			homeLayoutComparisonFields(current, base, undefined, true),
		).not.toHaveProperty("default_fingerprint");
	});

	test("unsupported widgets are only allowed when preserved exactly", () => {
		const existing = layout([
			widget("known"),
			widget("future", "future-widget", { value: 1 }),
		]);
		const reordered = layout([existing.widgets[1], existing.widgets[0]]);
		const local = validateHomeLayoutCandidate(
			layout([
				{
					...existing.widgets[1],
					size: { columns: 1, rows: 1 },
					appearance: { variant: "future-surface", accent: "future-accent" },
				},
			]),
		);
		expect(local.valid).toBe(true);
		expect(local.issues.map((entry) => entry.code)).toEqual([
			"unknown_widget_type",
		]);
		expect(validateUnknownHomeWidgetPreservation(reordered, existing)).toEqual(
			[],
		);

		const changed = structuredClone(reordered);
		changed.widgets[0].config.value = 2;
		expect(
			validateUnknownHomeWidgetPreservation(changed, existing),
		).toMatchObject([{ code: "unknown_widget_changed" }]);

		const removed = layout([existing.widgets[0]]);
		expect(
			validateUnknownHomeWidgetPreservation(removed, existing),
		).toMatchObject([{ code: "unknown_widget_removed" }]);
	});

	test("unadvertised config on known widgets must be preserved exactly", () => {
		const existing = layout([
			widget("known", "information", {
				mode: "faq",
				body: "Keep me",
				futureConfig: { revision: 1 },
				items: [
					{ id: "first", title: "First", futureItem: { revision: 1 } },
					{ id: "second", title: "Second", futureItem: "keep" },
				],
			}),
		]);
		const local = validateHomeLayoutCandidate(existing);
		expect(local.valid).toBe(true);
		expect(
			local.issues.filter(
				(entry) => entry.code === "home_widget_config_key_unknown",
			),
		).toHaveLength(3);
		const reordered = structuredClone(existing);
		(reordered.widgets[0].config.items as unknown[]).reverse();
		expect(
			validateUnknownHomeWidgetConfigPreservation(reordered, existing),
		).toEqual([]);

		const changed = structuredClone(existing);
		changed.widgets[0].config.futureConfig = { revision: 2 };
		expect(
			validateUnknownHomeWidgetConfigPreservation(changed, existing),
		).toMatchObject([{ code: "unknown_widget_config_changed" }]);

		const dropped = structuredClone(existing);
		dropped.widgets[0].config = Object.fromEntries(
			Object.entries(dropped.widgets[0].config).filter(
				([key]) => key !== "futureConfig",
			),
		);
		expect(
			validateUnknownHomeWidgetConfigPreservation(dropped, existing),
		).toMatchObject([{ code: "unknown_widget_config_changed" }]);

		const nestedChanged = structuredClone(existing);
		(
			(
				nestedChanged.widgets[0].config.items as Array<Record<string, unknown>>
			)[0].futureItem as Record<string, unknown>
		).revision = 2;
		expect(
			validateUnknownHomeWidgetConfigPreservation(nestedChanged, existing),
		).toMatchObject([{ code: "unknown_widget_config_changed" }]);

		const nestedDropped = structuredClone(existing);
		const firstItem = (
			nestedDropped.widgets[0].config.items as Array<Record<string, unknown>>
		)[0];
		nestedDropped.widgets[0].config.items = [
			Object.fromEntries(
				Object.entries(firstItem).filter(([key]) => key !== "futureItem"),
			),
			...(nestedDropped.widgets[0].config.items as unknown[]).slice(1),
		];
		expect(
			validateUnknownHomeWidgetConfigPreservation(nestedDropped, existing),
		).toMatchObject([{ code: "unknown_widget_config_changed" }]);

		const introduced = layout([
			widget("new", "information", { futureConfig: true }),
		]);
		expect(
			validateUnknownHomeWidgetConfigPreservation(introduced, layout([])),
		).toMatchObject([{ code: "unknown_widget_config_introduced" }]);
		expect(
			validateUnknownHomeWidgetConfigPreservation(layout([]), existing),
		).toMatchObject([{ code: "unknown_widget_config_removed" }]);
	});

	test("data config validation rejects values the renderer would silently replace", () => {
		const result = validateHomeLayoutCandidate(
			layout([
				widget("chart", "data", {
					appId: "app",
					sourceKind: "table",
					table: "orders",
					visualization: "bar",
					mode: "bogus",
					measures: [
						{ aggregation: "bogus", field: "revenue", label: "Revenue" },
					],
					filters: [
						{
							field: "region",
							operator: "bogus",
							value: "EMEA",
							valueType: "bogus",
						},
						{
							field: "amount",
							operator: "gt",
							value: "not-a-number",
							valueType: "number",
						},
						{
							field: "active",
							operator: "eq",
							value: "yes",
							valueType: "boolean",
						},
					],
					queryParams: [],
					timeBucket: "hour",
					dateRange: "forever",
					sortDirection: "sideways",
					format: "scientific",
					limit: 0,
					decimals: 7,
					refreshSeconds: 1,
					binWidth: 0,
					currency: "usd",
				}),
			]),
		);
		expect(result.valid).toBe(false);
		expect(result.issues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"home_widget_config_type_invalid",
				"home_widget_config_option_invalid",
				"data_filter_number_invalid",
				"data_filter_boolean_invalid",
				"home_widget_config_number_too_small",
				"home_widget_config_number_out_of_range",
				"home_widget_config_number_too_large",
				"home_widget_config_pattern_invalid",
			]),
		);

		const conditional = validateHomeLayoutCandidate(
			layout([
				widget("percentage", "data", {
					appId: "app",
					sourceKind: "query",
					queryId: "query",
					queryParams: { __home_secret: "value" },
					visualization: "percentstacked",
					dateRange: "7d",
				}),
			]),
		);
		expect(conditional.issues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"data_query_parameter_reserved",
				"data_date_field_missing",
				"data_group_series_missing",
			]),
		);

		for (const [visualization, config] of [
			["scatter", {}],
			["histogram", {}],
			["boxplot", {}],
			["calendar", { groupBy: "day" }],
		] as const) {
			const presentation = validateHomeLayoutCandidate(
				layout([
					widget(`missing-${visualization}`, "data", {
						appId: "app",
						sourceKind: "table",
						table: "records",
						visualization,
						...config,
					}),
				]),
			);
			expect(
				presentation.issues.some((entry) =>
					[
						"data_xy_fields_missing",
						"data_histogram_field_missing",
						"data_boxplot_field_missing",
						"data_calendar_fields_invalid",
					].includes(entry.code),
				),
				visualization,
			).toBe(true);
		}
	});

	test("data-source listing is profile-bound and returns compact schemas", async () => {
		let reads = 0;
		let schemaReads = 0;
		const backend = {
			dbState: {
				listTables: async () => {
					reads++;
					return ["orders"];
				},
				getSchema: async () => {
					schemaReads++;
					return {
						fields: [
							{ name: "region", data_type: "Utf8" },
							{ name: "revenue", data_type: "Float64" },
						],
					};
				},
			},
			graphState: {
				listOverlays: async () => [
					{
						id: "ontology",
						name: "Ontology",
						description: "",
						nodes: [
							{
								label: "Thing",
								table: "things",
								id_column: "id",
								display_column: "name",
								property_columns: [],
							},
						],
					},
				],
			},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "graphState" | "queryState"
		>;

		expect(
			await listHomeDataSources(
				backend,
				{ app_id: "private", kinds: ["table"] },
				async () => false,
			),
		).toMatchObject({ code: "home_data_app_not_in_profile" });
		expect(reads).toBe(0);
		expect(schemaReads).toBe(0);

		const broad = await listHomeDataSources(
			backend,
			{ app_id: "visible", kinds: ["table", "ontology"] },
			async () => true,
		);
		expect(reads).toBe(1);
		expect(schemaReads).toBe(0);
		expect(broad).toMatchObject({
			detail_level: "summaries",
			details_omitted: true,
			sources: {
				tables: [{ name: "orders", columns_omitted: true }],
				ontologies: [{ ontology_id: "ontology", object_types_omitted: true }],
			},
		});

		const result = await listHomeDataSources(
			backend,
			{ app_id: "visible", kinds: ["table"], source_id: "orders" },
			async () => true,
		);
		expect(reads).toBe(2);
		expect(schemaReads).toBe(1);
		expect(result).toMatchObject({
			status: "ok",
			app_id: "visible",
			sources: {
				tables: [
					{
						name: "orders",
						columns: [
							{ name: "region", type_name: "Utf8" },
							{ name: "revenue", type_name: "Float64" },
						],
					},
				],
			},
		});
	});

	test("data-source listing marks every nested truncation as incomplete", async () => {
		const fields = Array.from({ length: 81 }, (_, index) => ({
			name: `field_${index}`,
			data_type: "Utf8",
		}));
		const backend = {
			dbState: {
				listTables: async () => ["ontology"],
				getSchema: async () => ({ fields }),
			},
			graphState: {
				listOverlays: async () => [
					{
						id: "ontology",
						name: "Ontology",
						description: "",
						nodes: [
							{
								label: "Thing",
								table: "things",
								id_column: "field_0",
								display_column: "field_1",
								property_columns: fields,
							},
						],
					},
				],
			},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "graphState" | "queryState"
		>;

		const result = await listHomeDataSources(
			backend,
			{
				app_id: "visible",
				kinds: ["table", "ontology"],
				source_id: "ontology",
				object_type_query: "thing",
			},
			async () => true,
		);
		expect(result).toMatchObject({
			status: "partial",
			complete: false,
			truncated: true,
			details_omitted: true,
			detail_hint: expect.stringContaining("column_query"),
			sources: {
				tables: [{ columns_truncated: true }],
				ontologies: [{ object_types: [{ columns_truncated: true }] }],
			},
		});
	});

	test("data-source query filters before inventory caps", async () => {
		const tables = Array.from({ length: 70 }, (_, index) => `table_${index}`);
		tables.push("needle_table");
		const ontologies = Array.from({ length: 70 }, (_, index) => ({
			id: `ontology_${index}`,
			name: `Ontology ${index}`,
			description: "",
			nodes: [],
		}));
		ontologies.push({
			id: "needle_ontology",
			name: "Needle ontology",
			description: "",
			nodes: [],
		});
		const queries = Array.from({ length: 110 }, (_, index) => ({
			id: `query_${index}`,
			name: `Query ${index}`,
			description: "",
			kind: "sql",
			surface: "table",
			sql: "select 1",
		}));
		queries.push({
			id: "needle_query",
			name: "Needle query",
			description: "",
			kind: "sql",
			surface: "table",
			sql: "select 1",
		});
		const backend = {
			dbState: {
				listTables: async () => tables,
				getSchema: async () => ({ fields: [] }),
			},
			graphState: { listOverlays: async () => ontologies },
			queryState: { listSavedQueries: async () => queries },
		} as unknown as Pick<
			IBackendState,
			"dbState" | "graphState" | "queryState"
		>;

		const result = await listHomeDataSources(
			backend,
			{
				app_id: "visible",
				kinds: ["table", "ontology", "query"],
				query: "needle",
			},
			async () => true,
		);
		expect(result).toMatchObject({
			status: "ok",
			complete: true,
			query: "needle",
			matched_totals: { tables: 1, ontologies: 1, queries: 1 },
			sources: {
				tables: [{ name: "needle_table" }],
				ontologies: [{ ontology_id: "needle_ontology" }],
				queries: [{ query_id: "needle_query" }],
			},
		});
	});

	test("nested data-source filters recover matches beyond detail caps", async () => {
		const fields = Array.from({ length: 90 }, (_, index) => ({
			name: index === 89 ? "needle_field" : `field_${index}`,
			data_type: index === 89 ? "NeedleType" : "Utf8",
		}));
		const nodes = Array.from({ length: 90 }, (_, index) => ({
			label: index === 89 ? "Needle object" : `Object ${index}`,
			table: index === 89 ? "needle_objects" : `objects_${index}`,
			id_column: "field_0",
			display_column: "field_1",
			property_columns: fields,
		}));
		const backend = {
			dbState: {
				listTables: async () => ["large_table"],
				getSchema: async () => ({ fields }),
			},
			graphState: {
				listOverlays: async () => [
					{
						id: "large_ontology",
						name: "Large ontology",
						description: "",
						nodes,
					},
				],
			},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "graphState" | "queryState"
		>;

		const tableResult = await listHomeDataSources(
			backend,
			{
				app_id: "visible",
				kinds: ["table"],
				source_id: "large_table",
				column_query: "needle",
			},
			async () => true,
		);
		expect(tableResult).toMatchObject({
			status: "ok",
			complete: true,
			source_id: "large_table",
			column_query: "needle",
			sources: {
				tables: [
					{
						name: "large_table",
						columns_matched_total: 1,
						columns: [{ name: "needle_field" }],
					},
				],
			},
		});

		const ontologyResult = await listHomeDataSources(
			backend,
			{
				app_id: "visible",
				kinds: ["ontology"],
				source_id: "large_ontology",
				object_type_query: "needle",
				column_query: "needle",
			},
			async () => true,
		);
		expect(ontologyResult).toMatchObject({
			status: "ok",
			complete: true,
			object_type_query: "needle",
			column_query: "needle",
			sources: {
				ontologies: [
					{
						object_types_matched_total: 1,
						object_types: [
							{
								name: "Needle object",
								columns_matched_total: 1,
								columns: [{ name: "needle_field" }],
							},
						],
					},
				],
			},
		});
	});

	test("ontology detail expansion shares one global column budget", async () => {
		const fields = Array.from({ length: 90 }, (_, index) => ({
			name: `field_${index}`,
			data_type: "Utf8",
		}));
		const backend = {
			dbState: {},
			graphState: {
				listOverlays: async () => [
					{
						id: "ontology",
						name: "Ontology",
						description: "",
						nodes: Array.from({ length: 4 }, (_, index) => ({
							label: `Object ${index}`,
							table: `objects_${index}`,
							id_column: "field_0",
							display_column: "field_1",
							property_columns: fields,
						})),
					},
				],
			},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "graphState" | "queryState"
		>;

		const result = await listHomeDataSources(
			backend,
			{
				app_id: "visible",
				kinds: ["ontology"],
				source_id: "ontology",
				object_type_query: "object",
			},
			async () => true,
		);
		expect(result).toMatchObject({
			status: "partial",
			complete: false,
			truncated: true,
			column_records_returned: 240,
			column_record_limit: 240,
			response_byte_limit: 131_072,
		});
		const source = (
			result.sources as { ontologies: Array<Record<string, unknown>> }
		).ontologies[0].object_types as Array<{
			columns: unknown[];
		}>;
		expect(source.reduce((total, node) => total + node.columns.length, 0)).toBe(
			240,
		);
	});

	test("reference validation rejects missing app interfaces and table fields", async () => {
		const backend = {
			eventState: { getEvents: async () => [] },
			dbState: {
				listTables: async () => ["orders"],
				getSchema: async () => ({
					fields: [{ name: "revenue", data_type: "Float64" }],
				}),
			},
			graphState: {},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "eventState" | "graphState" | "queryState"
		>;
		const candidate = layout([
			widget("chat", "app-embed", {
				appId: "app",
				target: "event",
				eventId: "missing",
			}),
			widget("chart", "data", {
				appId: "app",
				sourceKind: "table",
				scope: "project",
				table: "orders",
				visualization: "bar",
				groupBy: "region",
			}),
		]);
		const issues = await validateHomeLayoutReferences(backend, candidate, {
			profileAppIds: new Set(["app"]),
		});
		expect(issues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"home_app_event_missing",
				"home_data_field_missing",
			]),
		);

		const pseudoSortIssues = await validateHomeLayoutReferences(
			backend,
			layout([
				widget("line", "data", {
					appId: "app",
					sourceKind: "table",
					scope: "project",
					table: "orders",
					visualization: "line",
					groupBy: "revenue",
					sortBy: "group",
				}),
			]),
			{ profileAppIds: new Set(["app"]) },
		);
		expect(
			pseudoSortIssues.some(
				(entry) => entry.code === "home_data_field_missing",
			),
		).toBe(false);
	});

	test("reference validation rejects saved queries with reserved parameters", async () => {
		const backend = {
			eventState: {},
			dbState: {},
			graphState: {},
			queryState: {
				listSavedQueries: async () => [
					{
						id: "query",
						name: "Unsafe query",
						description: "",
						kind: "sql",
						surface: "native",
						sql: "select * from records where owner = $__home_viewer",
					},
				],
			},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "eventState" | "graphState" | "queryState"
		>;
		const issues = await validateHomeLayoutReferences(
			backend,
			layout([
				widget("query", "data", {
					appId: "app",
					sourceKind: "query",
					queryId: "query",
					queryParams: {},
				}),
			]),
			{ profileAppIds: new Set(["app"]) },
		);
		expect(issues).toMatchObject([
			{
				code: "home_query_parameter_reserved",
				path: "$.widgets[0].config.queryId",
			},
		]);
	});

	test("reference validation checks saved-query parameter types and result-field limits", async () => {
		const backend = {
			eventState: {},
			dbState: {},
			graphState: {},
			queryState: {
				listSavedQueries: async () => [
					{
						id: "typed-query",
						name: "Typed query",
						kind: "query",
						surface: "native",
						sql: "select $amount, $enabled, $viewer",
						param_schema: {
							properties: {
								amount: { type: "number" },
								enabled: { type: "boolean" },
								viewer: { type: "string" },
							},
						},
					},
				],
			},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "eventState" | "graphState" | "queryState"
		>;
		const issues = await validateHomeLayoutReferences(
			backend,
			layout([
				widget("typed", "data", {
					appId: "app",
					sourceKind: "query",
					queryId: "typed-query",
					queryParams: {
						amount: "12",
						enabled: "true",
						viewer: "$viewer.id",
					},
					groupBy: "unverified-result-field",
				}),
			]),
			{ profileAppIds: new Set(["app"]) },
		);
		expect(
			issues.filter(
				(entry) => entry.code === "home_query_parameter_type_invalid",
			),
		).toHaveLength(2);
		expect(issues).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					code: "home_query_result_fields_unverified",
					severity: "warning",
				}),
			]),
		);
		expect(
			issues.some(
				(entry) =>
					entry.path.endsWith(".viewer") &&
					entry.code === "home_query_parameter_type_invalid",
			),
		).toBe(false);
	});

	test("app embeds require active Home-compatible Events and canonical routes", async () => {
		const backend = {
			eventState: {
				getEvents: async () => [
					{
						id: "headless",
						event_type: "cron",
						active: true,
						default_page_id: null,
						route: null,
					},
					{
						id: "inactive",
						event_type: "quick_action",
						active: false,
						default_page_id: null,
						route: "/old",
					},
					{
						id: "reports",
						event_type: "generic_form",
						active: true,
						default_page_id: null,
						route: "reports",
					},
				],
			},
			dbState: {},
			graphState: {},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "eventState" | "graphState" | "queryState"
		>;
		const issues = await validateHomeLayoutReferences(
			backend,
			layout([
				widget("headless", "app-embed", {
					appId: "app",
					target: "event",
					eventId: "headless",
				}),
				widget("inactive", "app-embed", {
					appId: "app",
					target: "event",
					eventId: "inactive",
				}),
				widget("canonical-route", "app-embed", {
					appId: "app",
					target: "route",
					route: "/reports?tab=summary",
				}),
				widget("inactive-route", "app-embed", {
					appId: "app",
					target: "route",
					route: "/old",
				}),
			]),
			{ profileAppIds: new Set(["app"]) },
		);
		expect(issues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"home_app_event_not_embeddable",
				"home_app_event_inactive",
				"home_app_route_missing",
			]),
		);
		expect(
			issues.some(
				(entry) =>
					entry.path === "$.widgets[2].config.route" &&
					entry.code === "home_app_route_missing",
			),
		).toBe(false);
	});

	test("reference validation ignores unsupported and inactive app fields", async () => {
		const backend = {
			eventState: {},
			dbState: {},
			graphState: {},
			queryState: {},
		} as unknown as Pick<
			IBackendState,
			"dbState" | "eventState" | "graphState" | "queryState"
		>;
		const preservedFuture = widget("future", "future-widget", {
			appId: "not-in-profile",
		});
		const inactive = layout([
			preservedFuture,
			widget("discovery", "app-collection", {
				source: "new",
				appIds: ["not-in-profile"],
			}),
			widget("usage", "run-stats", {
				metric: "ai",
				appId: "not-in-profile",
			}),
		]);
		expect(
			await validateHomeLayoutReferences(backend, inactive, {
				profileAppIds: new Set(),
			}),
		).toEqual([]);
		expect(
			validateUnknownHomeWidgetPreservation(
				structuredClone(inactive),
				inactive,
			),
		).toEqual([]);

		const active = layout([
			widget("manual", "app-collection", {
				source: "manual",
				appIds: ["not-in-profile"],
			}),
		]);
		expect(
			await validateHomeLayoutReferences(backend, active, {
				profileAppIds: new Set(),
			}),
		).toMatchObject([{ code: "home_app_not_in_profile" }]);
	});
});
