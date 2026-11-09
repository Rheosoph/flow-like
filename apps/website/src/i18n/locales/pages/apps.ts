import { enApps } from "./apps-en";

export const translationsApps: Record<string, Record<string, string>> = {
	en: enApps,
};

export function tApps(lang: string, key: string): string {
	return translationsApps[lang]?.[key] ?? translationsApps.en[key] ?? key;
}
