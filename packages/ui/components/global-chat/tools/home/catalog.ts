import { HOME_WIDGET_PRESETS } from "../../../home/catalog";
import { HOME_ACCENTS } from "../../../home/home-appearance";
import {
	HOME_APP_RENDERINGS,
	HOME_MODEL_RENDERINGS,
	HOME_PACKAGE_RENDERINGS,
} from "../../../home/home-content/config";
import {
	HOME_DATA_AGGREGATIONS,
	HOME_DATA_FILTER_OPERATORS,
	HOME_DATA_VISUALIZATIONS,
} from "../../../home/home-data-query";
import {
	MAX_HOME_LAYOUT_BYTES,
	MAX_HOME_WIDGETS,
	MAX_HOME_WIDGET_CLASS_NAME_BYTES,
	MAX_HOME_WIDGET_CSS_BYTES,
	responsiveHomeColumns,
} from "../../../home/home-layout";
import { DATA_SOURCE_KINDS, HOME_VARIANTS, stringArg } from "./shared";
import { HOME_WIDGET_CONFIG_CONTRACTS } from "./widget-contracts/definitions";

const HOME_CATEGORIES = new Set(
	HOME_WIDGET_PRESETS.map((preset) => preset.category),
);

const HOME_DATA_MODES = ["aggregate", "records"] as const;

const HOME_DATA_SCOPES = ["project", "personal"] as const;

const HOME_DATA_TIME_BUCKETS = [
	"none",
	"day",
	"week",
	"month",
	"quarter",
	"year",
] as const;

const HOME_DATA_DATE_RANGES = ["all", "7d", "30d", "90d", "year"] as const;

const HOME_DATA_SORT_DIRECTIONS = ["asc", "desc"] as const;

const HOME_DATA_FORMATS = ["number", "currency", "percent"] as const;

const HOME_DATA_VALUE_TYPES = ["text", "number", "boolean", "viewer"] as const;

/** Return compact creation templates for the exact widget implementations in this client. */
export function getHomeWidgetCatalog(args: Record<string, unknown>) {
	const category = stringArg(args, "category").toLowerCase();
	const type = stringArg(args, "type").toLowerCase();
	const query = stringArg(args, "query").toLowerCase();
	if (category && !HOME_CATEGORIES.has(category as never)) {
		return {
			status: "error",
			code: "home_widget_category_invalid",
			message: `Unknown category '${category}'.`,
			categories: [...HOME_CATEGORIES],
		};
	}
	const matches = HOME_WIDGET_PRESETS.filter((preset) => {
		if (category && preset.category !== category) return false;
		if (type && preset.type.toLowerCase() !== type) return false;
		if (!query) return true;
		return [preset.id, preset.name, preset.description, preset.type]
			.join(" ")
			.toLowerCase()
			.includes(query);
	});
	const matchedTypes = [...new Set(matches.map((preset) => preset.type))];
	return {
		status: "ok",
		total: matches.length,
		widget_type_total: matchedTypes.length,
		widget_types: Object.fromEntries(
			matchedTypes.map((widgetType) => [
				widgetType,
				{
					type: widgetType,
					...structuredClone(
						HOME_WIDGET_CONFIG_CONTRACTS[
							widgetType as keyof typeof HOME_WIDGET_CONFIG_CONTRACTS
						],
					),
				},
			]),
		),
		layout_contract: {
			version: 1,
			max_widgets: MAX_HOME_WIDGETS,
			max_bytes: MAX_HOME_LAYOUT_BYTES,
			grid_columns: {
				mobile: responsiveHomeColumns(0),
				tablet: responsiveHomeColumns(600),
				desktop: responsiveHomeColumns(1050),
			},
			breakpoints: {
				mobile_max_exclusive: 600,
				desktop_min: 1050,
			},
			height_modes: ["auto", "content", "fixed"],
			class_name: {
				field: "appearance.className",
				optional: true,
				max_bytes: MAX_HOME_WIDGET_CLASS_NAME_BYTES,
				applies_to:
					"The widget surface: the element inside the widget's grid item that draws its fill, border, radius and shadow. Utilities for the surface's own background, border, radius, text color and shadow replace the built-in ones from variant and accent. Editor controls (toolbar, selection overlay, resize handle) sit outside the surface, so classes and descendant variants never reach them.",
				syntax:
					"Space-separated Tailwind CSS v4 utilities compiled at runtime; v4 names such as bg-linear-to-br and arbitrary values such as bg-[#0f172a] or [--home-accent:#0ea5e9] work. Style descendants with variants such as [&_h2]:text-lg and pseudo-elements with before: and after:.",
				pseudo_elements:
					"The surface is position:relative and clips overflow, so before: and after: make decoration layers, e.g. before:absolute before:inset-x-0 before:top-0 before:h-1 before:bg-(--home-accent).",
				shadows:
					"The theme's shadow scale (shadow-2xs … shadow-2xl) is transparent in this theme. For elevation use an arbitrary shadow that includes its color, e.g. shadow-[0_12px_32px_-16px_rgb(0_0_0/0.35)], or box-shadow in css.",
				contrast:
					"A surface text color reaches only inheriting text; descriptions, dates and metadata use text-muted-foreground, which reads var(--muted-foreground). On a dark or saturated fill, redefine it on the surface too, e.g. [--muted-foreground:color-mix(in_oklab,var(--primary-foreground)_75%,transparent)].",
				motion:
					"tw-animate classes (animate-in, fade-in-*, slide-in-from-*) are not compiled at runtime. Define custom motion with @keyframes in appearance.css inside @media (prefers-reduced-motion: no-preference).",
				avoid:
					"The layout owns the surface's position, grid span, height and self-alignment, so absolute, relative, sticky, static, z-*, col-*/row-* span/start/end and self-* do nothing on the surface; they work behind before:, after:, *: or [&_…]: variants that target other elements. Never fixed (it escapes the widget, with or without a variant) or sibling variants (~, +), which reach neighboring widgets.",
				prefer:
					"Theme tokens (bg-card, text-primary, border-primary/30) and the surface_variables (text-(--home-accent), border-(--home-accent)/40, from-(--home-accent)/15) so light/dark mode and the accent setting keep working. When the user asks for a neat, polished, modern or branded look, style deliberately: surface fill or gradient, border, radius, elevation, typography and a decorative pseudo-element; colors the user supplies are fine. Keep styling coherent across widgets rather than decorating every card differently.",
			},
			css: {
				field: "appearance.css",
				optional: true,
				max_bytes: MAX_HOME_WIDGET_CSS_BYTES,
				applies_to:
					"A stylesheet scoped to this widget. :root is the widget surface; :root:is(.dark *) targets dark mode; every other selector matches inside the widget. Rules override className utilities.",
				syntax:
					"Plain CSS, not Tailwind: @apply, @tailwind, @theme, @variant, @utility, theme() and --alpha() do nothing here, so put utilities in className. @keyframes names are local to the widget.",
				pseudo_elements:
					"The surface is position:relative and clips overflow, so :root::before and :root::after may use content, position:absolute, inset, z-index, width and height for decoration. Set isolation:isolate on :root when a layer uses z-index:-1.",
				shadows:
					"Give elevation an explicit color, e.g. box-shadow: 0 12px 32px -16px rgb(0 0 0 / 0.35); the theme's shadow scale is transparent.",
				contrast:
					"On a dark or saturated fill, redefine :root { --muted-foreground: … } too, because descriptions, dates and metadata read var(--muted-foreground) instead of inheriting the surface text color.",
				motion:
					"Define custom motion with @keyframes and wrap it in @media (prefers-reduced-motion: no-preference).",
				avoid:
					"+ or ~ after :root (reaches neighboring widgets); @property, @font-face, @counter-style and @page (document-wide); position, inset, z-index, grid placement, order, width or height on :root itself (the layout owns them); position:fixed anywhere; hsl(), rgb() or oklch() around color variables. @import is removed.",
				prefer:
					"className for anything utilities express; css for @keyframes, :root::before/::after decoration and several descendant rules. Use theme_colors and surface_variables so light/dark mode and the accent setting keep working.",
			},
			surface_variables: {
				names: [
					"--home-accent",
					"--home-accent-foreground",
					"--home-surface-background",
					"--home-surface-foreground",
					"--home-surface-muted",
					"--home-surface-accent",
					"--home-surface-border",
					"--home-surface-item",
					"--home-surface-item-hover",
				],
				usage:
					"Set from the widget's variant and accent and inherited by the surface. Use them so custom styling follows the accent setting: className text-(--home-accent), border-(--home-accent)/40, from-(--home-accent)/15; css color-mix(in oklab, var(--home-accent) 20%, transparent). Redefine one on the surface (css :root { --home-accent: … } or className [--home-accent:…]) to retint the widget's own content.",
			},
			theme_colors: {
				names: [
					"--background",
					"--foreground",
					"--card",
					"--card-foreground",
					"--primary",
					"--primary-foreground",
					"--secondary",
					"--muted",
					"--muted-foreground",
					"--accent",
					"--border",
					"--chart-1",
					"--chart-2",
					"--chart-3",
					"--chart-4",
					"--chart-5",
				],
				usage:
					"These CSS variables hold complete colors. In css use var(--primary) directly or color-mix(in oklab, var(--primary) 30%, transparent); never wrap them in hsl(), rgb() or oklch().",
			},
			targets: {
				surface:
					"The styled element of one widget. Select it with :root in css; unprefixed utilities in className apply to it.",
				generic_header:
					"Widget types without their own header (all except section-heading, greeting, flowpilot, app-embed, quick-actions, app-spotlight, app-ranking, app-collection-feature, model-spotlight and workspace-pulse) render one only when title or description is set. className: [&>div>header], [&>div>header>h2] (starts with an svg icon, [&>div>header>h2>svg]) and [&>div>header>p] (description); css: :root > div > header > h2. Prefer these child paths over a bare header or h2, which also match headings inside app-ranking and section-heading bodies.",
				hooks:
					"[data-home-section-heading] h2/p/a; [data-home-greeting] h1/p; [data-home-discovery=app-spotlight|app-ranking|app-collection-feature|model-spotlight] h2/p/a with [data-home-discovery-mode=hero|compact]; [data-workspace-pulse=starter|loading|unavailable|activity] with [data-workspace-mode=attention|strip]; [data-home-collection-rendering]; [data-home-package-rendering] > [data-package-card]; [data-home-app-updated]; [data-home-data-state]; [data-home-data-presentation=<visualization>]; [data-home-data-calendar]. Reach them with e.g. [&_[data-home-greeting]_h1]:text-4xl.",
				source_note:
					"run-activity, executions-by-app, ai-usage, run-stats, recent-runs, schedules and workspace-pulse show their data source as details > summary with details > p.",
				unhooked:
					"Body wrappers and empty, loading and error states are plain divs without hooks; editor chrome is outside the surface and cannot be styled.",
			},
		},
		categories: [...HOME_CATEGORIES],
		accents: Object.keys(HOME_ACCENTS),
		surface_variants: [...HOME_VARIANTS],
		visualizations: HOME_DATA_VISUALIZATIONS.map(([id, name]) => ({
			id,
			name,
		})),
		data_options: {
			source_kinds: DATA_SOURCE_KINDS,
			scopes: [...HOME_DATA_SCOPES],
			modes: [...HOME_DATA_MODES],
			aggregations: [...HOME_DATA_AGGREGATIONS],
			filter_operators: [...HOME_DATA_FILTER_OPERATORS],
			time_buckets: [...HOME_DATA_TIME_BUCKETS],
			date_ranges: [...HOME_DATA_DATE_RANGES],
			sort_directions: [...HOME_DATA_SORT_DIRECTIONS],
			formats: [...HOME_DATA_FORMATS],
			filter_value_types: [...HOME_DATA_VALUE_TYPES],
		},
		renderings: {
			apps: HOME_APP_RENDERINGS.map(([id, name]) => ({ id, name })),
			models: HOME_MODEL_RENDERINGS.map(([id, name]) => ({ id, name })),
			packages: HOME_PACKAGE_RENDERINGS.map(([id, name]) => ({ id, name })),
			quick_links: ["grid", "list"],
		},
		presets: matches.map((preset) => ({
			preset_id: preset.id,
			name: preset.name,
			description: preset.description,
			category: preset.category,
			widget: {
				type: preset.type,
				title: preset.name,
				description: "",
				size: { columns: preset.columns, rows: preset.rows },
				appearance: {
					variant: preset.variant ?? "card",
					accent: preset.accent ?? "neutral",
				},
				config: structuredClone(preset.config),
			},
		})),
		note: "Give every widget a unique id when placing it in a layout.",
	};
}
