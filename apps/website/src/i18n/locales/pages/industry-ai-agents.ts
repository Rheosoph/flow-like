import { deAiAgents } from "./industry-ai-agents-de";
import { enAiAgents } from "./industry-ai-agents-en";
import { esAiAgents } from "./industry-ai-agents-es";
import { frAiAgents } from "./industry-ai-agents-fr";
import { itAiAgents } from "./industry-ai-agents-it";
import { jaAiAgents } from "./industry-ai-agents-ja";
import { koAiAgents } from "./industry-ai-agents-ko";
import { nlAiAgents } from "./industry-ai-agents-nl";
import { ptAiAgents } from "./industry-ai-agents-pt";
import { svAiAgents } from "./industry-ai-agents-sv";
import { zhAiAgents } from "./industry-ai-agents-zh";

export const translationsAiAgents: Record<string, Record<string, string>> = {
	en: enAiAgents,
	de: deAiAgents,
	fr: frAiAgents,
	es: esAiAgents,
	zh: zhAiAgents,
	ja: jaAiAgents,
	ko: koAiAgents,
	pt: ptAiAgents,
	it: itAiAgents,
	nl: nlAiAgents,
	sv: svAiAgents,
};

export function tAiAgents(lang: string, key: string): string {
	return (
		translationsAiAgents[lang]?.[key] ?? translationsAiAgents.en[key] ?? key
	);
}
