import { deIntegrations } from "./integrations-de";
import { enIntegrations } from "./integrations-en";
import { esIntegrations } from "./integrations-es";
import { frIntegrations } from "./integrations-fr";
import { itIntegrations } from "./integrations-it";
import { jaIntegrations } from "./integrations-ja";
import { koIntegrations } from "./integrations-ko";
import { nlIntegrations } from "./integrations-nl";
import { ptIntegrations } from "./integrations-pt";
import { svIntegrations } from "./integrations-sv";
import { zhIntegrations } from "./integrations-zh";

export const translationsIntegrations: Record<
	string,
	Record<string, string>
> = {
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
	return (
		translationsIntegrations[lang]?.[key] ??
		translationsIntegrations.en[key] ??
		key
	);
}
