import { describe, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import i18next from "i18next";
import { LOCALE_CONFIG } from "./config";
import { createI18n, i18n as exportedI18n } from "./create-i18n";
import { SOURCE_RESOURCES } from "./resources";

const LOCALES_DIR = path.resolve(import.meta.dirname, "../locales");

function flatten(tree: unknown, prefix = ""): string[] {
	if (typeof tree !== "object" || tree === null) return [];
	return Object.entries(tree).flatMap(([key, value]) => {
		const next = prefix ? `${prefix}.${key}` : key;
		return typeof value === "object" && value !== null
			? flatten(value, next)
			: [next];
	});
}

function entries(tree: unknown, prefix = ""): Array<[string, string]> {
	if (typeof tree !== "object" || tree === null) return [];
	return Object.entries(tree).flatMap(([key, value]) => {
		const next = prefix ? `${prefix}.${key}` : key;
		return typeof value === "object" && value !== null
			? entries(value, next)
			: typeof value === "string"
				? [[next, value] as [string, string]]
				: [];
	});
}

describe("locale files", () => {
	test("every configured language has every configured namespace", () => {
		for (const language of LOCALE_CONFIG.languages) {
			const files = readdirSync(path.join(LOCALES_DIR, language));
			for (const namespace of LOCALE_CONFIG.namespaces) {
				expect(files).toContain(`${namespace}.json`);
			}
		}
	});

	test("every file is valid JSON with only string leaves", () => {
		for (const language of LOCALE_CONFIG.languages) {
			for (const namespace of LOCALE_CONFIG.namespaces) {
				const file = path.join(LOCALES_DIR, language, `${namespace}.json`);
				const parsed = JSON.parse(readFileSync(file, "utf8"));
				const walk = (value: unknown): void => {
					if (typeof value === "string") return;
					expect(Array.isArray(value)).toBe(false);
					expect(typeof value).toBe("object");
					for (const child of Object.values(value as object)) walk(child);
				};
				walk(parsed);
			}
		}
	});

	test("the bundled source resources match the namespace list", () => {
		expect(Object.keys(SOURCE_RESOURCES).sort()).toEqual(
			[...LOCALE_CONFIG.namespaces].sort(),
		);
	});

	test("every secondary locale has exactly the source key set", () => {
		for (const namespace of LOCALE_CONFIG.namespaces) {
			const source = JSON.parse(
				readFileSync(
					path.join(
						LOCALES_DIR,
						LOCALE_CONFIG.sourceLanguage,
						`${namespace}.json`,
					),
					"utf8",
				),
			);
			const sourceKeys = flatten(source).sort();

			for (const language of LOCALE_CONFIG.languages) {
				if (language === LOCALE_CONFIG.sourceLanguage) continue;
				const target = JSON.parse(
					readFileSync(
						path.join(LOCALES_DIR, language, `${namespace}.json`),
						"utf8",
					),
				);
				expect({ language, namespace, keys: flatten(target).sort() }).toEqual({
					language,
					namespace,
					keys: sourceKeys,
				});
			}
		}
	});

	/*
	 * A translation that drops an interpolation renders the raw token to the
	 * user. This is the one locale mistake that is always a bug, so it is worth
	 * failing the build over rather than only surfacing it in the studio.
	 */
	test("translations keep the placeholders their source has", () => {
		const tokens = (value: string) => {
			const interpolation = [
				...value.matchAll(/\{\{\s*([^}]+?)\s*\}\}|\$t\(([^)]+)\)/g),
			].map((match) => (match[1] ?? `$t(${match[2]})`).trim());
			const transTags = [
				...value.matchAll(/<\s*(\/?)\s*(\d+)\s*(\/?)\s*>/g),
			].map((match) =>
				match[1]
					? `</${match[2]}>`
					: match[3]
						? `<${match[2]}/>`
						: `<${match[2]}>`,
			);
			return [...interpolation, ...transTags];
		};

		for (const namespace of LOCALE_CONFIG.namespaces) {
			const source = JSON.parse(
				readFileSync(
					path.join(
						LOCALES_DIR,
						LOCALE_CONFIG.sourceLanguage,
						`${namespace}.json`,
					),
					"utf8",
				),
			);

			for (const language of LOCALE_CONFIG.languages) {
				if (language === LOCALE_CONFIG.sourceLanguage) continue;
				const target = JSON.parse(
					readFileSync(
						path.join(LOCALES_DIR, language, `${namespace}.json`),
						"utf8",
					),
				);

				for (const key of flatten(source)) {
					const read = (tree: unknown) =>
						key
							.split(".")
							.reduce<unknown>(
								(node, segment) =>
									typeof node === "object" && node !== null
										? (node as Record<string, unknown>)[segment]
										: undefined,
								tree,
							);
					const sourceValue = read(source);
					const targetValue = read(target);
					if (typeof sourceValue !== "string") continue;
					if (typeof targetValue !== "string" || !targetValue) continue;

					expect({
						language,
						namespace,
						key,
						tokens: tokens(targetValue).sort(),
					}).toEqual({
						language,
						namespace,
						key,
						tokens: tokens(sourceValue).sort(),
					});
				}
			}
		}
	});

	test("German translations are complete and use the informal product voice", () => {
		const failures: string[] = [];
		const literalProductTerms: Array<[RegExp, RegExp]> = [
			[/\bflows?\b/i, /\b(?:Fluss|Flüsse|Strom|Ström)/i],
			[/\bnodes?\b/i, /Knoten/i],
			[/\bboards?\b/i, /\bBrett/i],
			[/\bsinks?\b/i, /(?:Waschbecken|\bSpül|\bAbfluss)/i],
			[/\bbuilder\b/i, /(?:Bauherr|Erbauer)/i],
			[/\bhubs?\b/i, /\bNabe/i],
			[/\btiles?\b/i, /Fliese/i],
			[/\bpins?\b/i, /Stecknadel/i],
		];
		for (const namespace of LOCALE_CONFIG.namespaces) {
			const source = new Map(
				entries(
					JSON.parse(
						readFileSync(
							path.join(
								LOCALES_DIR,
								LOCALE_CONFIG.sourceLanguage,
								`${namespace}.json`,
							),
							"utf8",
						),
					),
				),
			);
			const target = JSON.parse(
				readFileSync(path.join(LOCALES_DIR, "de", `${namespace}.json`), "utf8"),
			);
			for (const [key, value] of entries(target)) {
				const sourceValue = source.get(key) ?? "";
				if (!value.trim()) failures.push(`${namespace}:${key} is empty`);
				if (/\b(?:Sie|Ihnen|Ihr|Ihre|Ihrer|Ihrem|Ihren|Ihres)\b/.test(value)) {
					failures.push(`${namespace}:${key} uses formal address`);
				}
				if (
					literalProductTerms.some(
						([sourceTerm, badTranslation]) =>
							sourceTerm.test(sourceValue) && badTranslation.test(value),
					)
				) {
					failures.push(
						`${namespace}:${key} translates a product term literally`,
					);
				}
				if (
					!/(?:\p{L}\{\{|\}\}\p{L})/u.test(sourceValue) &&
					/(?:\p{L}\{\{|\}\}\p{L})/u.test(value)
				) {
					failures.push(`${namespace}:${key} joins a placeholder to a word`);
				}
				if (
					/(?:Hey friend|Hallo Freund|Hallo Kumpel|ZZFLTOKEN|FLOWLIKETRANSLATIONSEPARATOR)/i.test(
						value,
					)
				) {
					failures.push(`${namespace}:${key} leaks a translation marker`);
				}
			}
		}
		expect(failures).toEqual([]);
	});
});

/*
 * `createI18n` is a singleton by design — every entry point calls it and gets
 * the instance the first caller built. These tests share that instance, so each
 * one sets the language it needs rather than relying on the order they run in.
 */
describe("createI18n", () => {
	test("configures the default i18next export instead of a private instance", () => {
		expect(createI18n()).toBe(i18next);
		expect(exportedI18n).toBe(i18next);
		expect(i18next.isInitialized).toBe(true);
		expect(i18next.t("settings:theme.light")).toBe("Light");
	});

	test("resolves keys in the source language without a backend round-trip", async () => {
		const i18n = createI18n({ language: LOCALE_CONFIG.sourceLanguage });
		await i18n.changeLanguage(LOCALE_CONFIG.sourceLanguage);
		await i18n.loadNamespaces([...LOCALE_CONFIG.namespaces]);
		expect(i18n.t("settings:theme.light")).toBe("Light");
		expect(i18n.t("feedback.trigger")).toBe("Report Bug");
	});

	test("loads a secondary language through the dynamic-import backend", async () => {
		const i18n = createI18n();
		await i18n.changeLanguage("de");
		await i18n.loadNamespaces([...LOCALE_CONFIG.namespaces]);
		expect(i18n.t("settings:theme.light")).toBe("Hell");
	});

	test("falls back to the source language for an unknown key", async () => {
		const i18n = createI18n();
		await i18n.changeLanguage("de");
		expect((i18n.t as (key: string) => string)("common:nope.not.here")).toBe(
			"nope.not.here",
		);
	});
});
