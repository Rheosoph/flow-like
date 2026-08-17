import i18next, { type i18n as I18nInstance } from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import resourcesToBackend from "i18next-resources-to-backend";
import { initReactI18next } from "react-i18next";
import {
	DEFAULT_NAMESPACE,
	LANGUAGES,
	NAMESPACES,
	SOURCE_LANGUAGE,
} from "./config";
import { SOURCE_RESOURCES } from "./resources";
import "./types";

export const LANGUAGE_STORAGE_KEY = "flow-like:language";

export interface CreateI18nOptions {
	/** Skip detection and pin the instance to one language (tests, previews). */
	language?: string;
	debug?: boolean;
}

let configured = false;

function configure(options: CreateI18nOptions): I18nInstance {
	if (configured || i18next.isInitialized) {
		configured = true;
		return i18next;
	}

	const isBrowser = typeof window !== "undefined";

	i18next
		// Non-source languages are code-split: the bundler turns this template
		// import into one chunk per file under `locales/`, so a language costs
		// nothing until someone selects it.
		.use(
			resourcesToBackend(
				(language: string, namespace: string) =>
					import(`../locales/${language}/${namespace}.json`),
			),
		)
		.use(initReactI18next);

	if (isBrowser && !options.language) i18next.use(LanguageDetector);

	void i18next.init({
		debug: options.debug ?? false,
		lng: options.language,
		fallbackLng: SOURCE_LANGUAGE,
		supportedLngs: [...LANGUAGES],
		// Treat `de-AT` as `de` instead of falling straight back to English.
		nonExplicitSupportedLngs: true,
		ns: [...NAMESPACES],
		defaultNS: DEFAULT_NAMESPACE,
		// The source language is bundled; `partialBundledLanguages` keeps the
		// backend active for everything that is not.
		resources: { [SOURCE_LANGUAGE]: SOURCE_RESOURCES },
		partialBundledLanguages: true,
		detection: {
			order: ["querystring", "localStorage", "navigator"],
			lookupQuerystring: "lng",
			lookupLocalStorage: LANGUAGE_STORAGE_KEY,
			caches: ["localStorage"],
		},
		interpolation: {
			// React escapes for us.
			escapeValue: false,
		},
		// Secondary catalogs deliberately use an empty string for work that has
		// not been translated yet. Empty must therefore fall back to English,
		// rather than rendering a blank label.
		returnEmptyString: false,
		// Bundled resources can be registered synchronously. This matters for the
		// legacy module-scope `i18next.t()` calls that run while modules evaluate.
		initAsync: false,
		// Static exports have no Suspense boundary around the shell, and the
		// source language is already bundled, so there is nothing to suspend on.
		react: { useSuspense: false },
	});

	configured = true;
	return i18next;
}

/**
 * Builds the shared i18next instance. Safe to call from every entry point —
 * the second call returns the instance the first one created.
 */
export function createI18n(options: CreateI18nOptions = {}): I18nInstance {
	const client = configure(options);
	if (options.debug !== undefined) client.options.debug = options.debug;
	if (options.language && client.resolvedLanguage !== options.language) {
		void client.changeLanguage(options.language);
	}
	return client;
}

/** The configured singleton shared with direct `import i18next` call sites. */
export function getI18n(): I18nInstance {
	return i18next;
}

// A number of non-component helpers still call the package's default
// `i18next.t()` export directly. Configure that exact singleton during module
// evaluation so those calls have bundled English resources even when they run
// before React mounts the provider.
export const i18n = configure({});
