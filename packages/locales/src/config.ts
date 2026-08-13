import config from "../locales/config.json";

/**
 * `locales/config.json` is the single source of truth for which languages and
 * namespaces exist. The runtime reads it, `i18next.config.ts` reads it, and the
 * translation studio in `apps/translation` writes it — so adding a language is
 * a JSON edit, never a code change.
 */
export const LOCALE_CONFIG = config;

export const SOURCE_LANGUAGE = config.sourceLanguage;
export const DEFAULT_NAMESPACE = config.defaultNamespace;
export const NAMESPACES: readonly string[] = config.namespaces;
export const LANGUAGES: readonly string[] = config.languages;
