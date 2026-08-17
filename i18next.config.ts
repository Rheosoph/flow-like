import { defineConfig } from "i18next-cli";
import localeConfig from "./packages/locales/locales/config.json";

/**
 * i18next-cli configuration for the whole monorepo.
 *
 * Languages and namespaces come from `packages/locales/locales/config.json` so
 * the CLI, the runtime and the translation studio never disagree about what
 * exists. Run it through `mise run i18n:extract` — the CLI is installed in
 * `tools/i18n` to keep it clear of the root dependency overrides.
 */
const { sourceLanguage, languages, defaultNamespace } = localeConfig;

export default defineConfig({
	locales: [...languages],
	extract: {
		input: [
			"apps/desktop/{app,components,hooks,lib}/**/*.{ts,tsx}",
			"apps/web/{app,components,lib}/**/*.{ts,tsx}",
			"packages/ui/{components,hooks,lib,state}/**/*.{ts,tsx}",
		],
		ignore: [
			"**/node_modules/**",
			"**/.next/**",
			"**/out/**",
			"**/dist/**",
			"**/*.d.ts",
			"**/*.test.{ts,tsx}",
			"**/__tests__/**",
		],
		output: "packages/locales/locales/{{language}}/{{namespace}}.json",
		defaultNS: defaultNamespace,
		primaryLanguage: sourceLanguage,
		// Everything but English starts empty, which is what the studio renders
		// as "missing" — an English string sitting in a German file would read
		// as done and never get looked at again.
		secondaryLanguages: languages.filter((code) => code !== sourceLanguage),
		defaultValue: "",
		// `nav.*` was migrated from the legacy `public/locales/*/translation.json`
		// and is already translated. The `store:*` entries below are looked up
		// through a lookup table (category enum → key, sort option → key), so the
		// scanner never sees the literal and would delete translated work.
		preservePatterns: [
			"nav:*",
			"store:category*",
			"store:bestRated",
			"store:mostPopular",
			"store:newestFirst",
		],
		sort: true,
		indentation: "\t",
	},
});
