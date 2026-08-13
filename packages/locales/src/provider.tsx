"use client";

import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { SOURCE_LANGUAGE } from "./config";
import { createI18n } from "./create-i18n";
import { describeLanguage, isRtl, listLanguages } from "./languages";

/**
 * Mount once, as high in the tree as the theme provider. Both apps and
 * `packages/ui` then read translations through `useTranslation()`.
 */
export function I18nProvider({
	children,
	language,
}: Readonly<{ children: ReactNode; language?: string }>) {
	const i18n = useMemo(() => createI18n({ language }), [language]);

	// The document element is rendered by the static shell, so the active
	// language has to be written onto it from the client after every switch.
	useEffect(() => {
		if (typeof document === "undefined") return;
		const apply = () => {
			const code = i18n.resolvedLanguage ?? i18n.language ?? SOURCE_LANGUAGE;
			document.documentElement.lang = code;
			document.documentElement.dir = isRtl(code) ? "rtl" : "ltr";
		};
		apply();
		i18n.on("languageChanged", apply);
		return () => i18n.off("languageChanged", apply);
	}, [i18n]);

	return <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
}

export interface UseLanguageResult {
	current: ReturnType<typeof describeLanguage>;
	available: ReturnType<typeof listLanguages>;
	setLanguage: (code: string) => Promise<unknown>;
}

/** Current language plus everything a language switcher needs. */
export function useLanguage(): UseLanguageResult {
	const { i18n } = useTranslation();

	const resolved = useSyncExternalStore(
		useCallback(
			(onChange: () => void) => {
				i18n.on("languageChanged", onChange);
				return () => i18n.off("languageChanged", onChange);
			},
			[i18n],
		),
		() => i18n.resolvedLanguage ?? i18n.language ?? SOURCE_LANGUAGE,
		() => SOURCE_LANGUAGE,
	);

	return {
		current: describeLanguage(resolved),
		available: listLanguages(resolved),
		setLanguage: useCallback(
			(code: string) => i18n.changeLanguage(code),
			[i18n],
		),
	};
}
