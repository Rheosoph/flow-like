import { enBots } from "./bots-en";

export const translationsBots: Record<string, Record<string, string>> = {
	en: enBots,
};

export function tBots(lang: string, key: string): string {
	return translationsBots[lang]?.[key] ?? translationsBots.en[key] ?? key;
}
