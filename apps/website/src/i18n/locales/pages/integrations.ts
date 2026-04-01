import { enIntegrations } from "./integrations-en";
import { deIntegrations } from "./integrations-de";
import { esIntegrations } from "./integrations-es";
import { frIntegrations } from "./integrations-fr";
import { zhIntegrations } from "./integrations-zh";
import { jaIntegrations } from "./integrations-ja";
import { koIntegrations } from "./integrations-ko";
import { ptIntegrations } from "./integrations-pt";
import { itIntegrations } from "./integrations-it";
import { nlIntegrations } from "./integrations-nl";
import { svIntegrations } from "./integrations-sv";

export const translationsIntegrations: Record<string, Record<string, string>> = {
  en: enIntegrations,
  de: deIntegrations,
  es: esIntegrations,
  fr: frIntegrations,
  zh: zhIntegrations,
  ja: jaIntegrations,
  ko: koIntegrations,
  pt: ptIntegrations,
  it: itIntegrations,
  nl: nlIntegrations,
  sv: svIntegrations,
};

export function tIntegrations(lang: string, key: string): string {
  return translationsIntegrations[lang]?.[key] ?? translationsIntegrations.en[key] ?? key;
}
