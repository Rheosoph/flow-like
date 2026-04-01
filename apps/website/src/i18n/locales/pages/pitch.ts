import { enPitch } from "./pitch-en";

export const translationsPitch: Record<string, Record<string, string>> = {
	en: enPitch,
};

export type LangPitch = keyof typeof translationsPitch;

export function tPitch(lang: string, key: string): string {
	return translationsPitch[lang]?.[key] ?? translationsPitch.en[key] ?? key;
}
