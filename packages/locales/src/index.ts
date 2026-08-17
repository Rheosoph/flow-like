export {
	DEFAULT_NAMESPACE,
	LANGUAGES,
	LOCALE_CONFIG,
	NAMESPACES,
	SOURCE_LANGUAGE,
} from "./config";
export {
	createI18n,
	getI18n,
	i18n,
	LANGUAGE_STORAGE_KEY,
	type CreateI18nOptions,
} from "./create-i18n";
export {
	describeLanguage,
	isRtl,
	listLanguages,
	type LanguageInfo,
} from "./languages";
export { I18nProvider, useLanguage, type UseLanguageResult } from "./provider";
export { SOURCE_RESOURCES, type SourceResources } from "./resources";
export { Trans, useTranslation } from "react-i18next";
import "./types";
