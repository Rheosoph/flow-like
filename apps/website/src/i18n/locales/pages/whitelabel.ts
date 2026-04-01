import { enWhitelabel } from "./whitelabel-en";
import { deWhitelabel } from "./whitelabel-de";
import { esWhitelabel } from "./whitelabel-es";
import { frWhitelabel } from "./whitelabel-fr";
import { zhWhitelabel } from "./whitelabel-zh";
import { jaWhitelabel } from "./whitelabel-ja";
import { koWhitelabel } from "./whitelabel-ko";
import { ptWhitelabel } from "./whitelabel-pt";
import { itWhitelabel } from "./whitelabel-it";
import { nlWhitelabel } from "./whitelabel-nl";
import { svWhitelabel } from "./whitelabel-sv";

export const translationsWhitelabel: Record<string, Record<string, string>> = {
  en: enWhitelabel,
  de: deWhitelabel,
  es: esWhitelabel,
  fr: frWhitelabel,
  zh: zhWhitelabel,
  ja: jaWhitelabel,
  ko: koWhitelabel,
  pt: ptWhitelabel,
  it: itWhitelabel,
  nl: nlWhitelabel,
  sv: svWhitelabel,
};

export function tWhitelabel(lang: string, key: string): string {
  return translationsWhitelabel[lang]?.[key] ?? translationsWhitelabel.en[key] ?? key;
}
