import { enData } from "./data-en";

export const translationsData: Record<string, Record<string, string>> = {
	en: enData,
};

export function tData(lang: string, key: string): string {
	return translationsData[lang]?.[key] ?? translationsData.en[key] ?? key;
}
