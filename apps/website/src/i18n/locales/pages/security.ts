import { enSecurity } from "./security-en";
import { deSecurity } from "./security-de";
import { esSecurity } from "./security-es";
import { frSecurity } from "./security-fr";
import { zhSecurity } from "./security-zh";
import { jaSecurity } from "./security-ja";
import { koSecurity } from "./security-ko";
import { ptSecurity } from "./security-pt";
import { itSecurity } from "./security-it";
import { nlSecurity } from "./security-nl";
import { svSecurity } from "./security-sv";

export const translationsSecurity: Record<string, Record<string, string>> = {
  en: enSecurity,
  de: deSecurity,
  es: esSecurity,
  fr: frSecurity,
  zh: zhSecurity,
  ja: jaSecurity,
  ko: koSecurity,
  pt: ptSecurity,
  it: itSecurity,
  nl: nlSecurity,
  sv: svSecurity,
};

export function tSecurity(lang: string, key: string): string {
  return translationsSecurity[lang]?.[key] ?? translationsSecurity.en[key] ?? key;
}
