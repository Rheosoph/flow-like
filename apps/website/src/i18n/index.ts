import {
	de,
	en as enBase,
	es,
	fr,
	it,
	ja,
	ko,
	nl,
	pt,
	sv,
	zh,
} from "./locales";
import { v4en } from "./locales/v4";

const en = { ...enBase, ...v4en };

export const languages = {
	en: "English",
	de: "Deutsch",
	es: "Español",
	fr: "Français",
	zh: "中文",
	ja: "日本語",
	ko: "한국어",
	pt: "Português",
	it: "Italiano",
	nl: "Nederlands",
	sv: "Svenska",
};

export const defaultLang = "en";

export type Lang = keyof typeof languages;

export function getLangFromUrl(url: URL): Lang {
	const [, lang] = url.pathname.split("/");
	if (lang in languages) return lang as Lang;
	return defaultLang;
}

export function detectBrowserLang(): Lang {
	if (typeof navigator === "undefined") return defaultLang;
	const browserLang = navigator.language?.slice(0, 2);
	if (browserLang && browserLang in languages) return browserLang as Lang;
	return defaultLang;
}

export function useTranslations(lang: Lang) {
	return function t(key: TranslationKey): string {
		const langTranslations = translations[lang] as Record<string, string>;
		const enTranslations = translations.en as Record<string, string>;
		return langTranslations?.[key] ?? enTranslations[key] ?? key;
	};
}

export function getTranslation(lang: Lang, key: TranslationKey): string {
	const langTranslations = translations[lang] as Record<string, string>;
	const enTranslations = translations.en as Record<string, string>;
	return langTranslations?.[key] ?? enTranslations[key] ?? key;
}

export const translations = {
	en,
	de,
	es,
	fr,
	zh,
	ja,
	ko,
	pt,
	it,
	nl,
	sv,
} as const;

export type TranslationKey = string;
