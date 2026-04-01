export const svDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "För Utvecklare | Flow-Like",
	"dev.meta.description":
		"Två vägar att bygga med Flow-Like: skapa arbetsflöden visuellt med befintliga noder, eller skriv egna noder i 15+ språk som kompileras till WebAssembly.",

	// Hero
	"dev.hero.badge": "Öppen Källkod",
	"dev.hero.headline.prefix": "För",
	"dev.hero.headline.highlight": "Utvecklare",
	"dev.hero.description":
		"Oavsett om du bygger arbetsflöden visuellt eller skriver egna noder i valfritt språk — Flow-Like ger dig två tydliga vägar att leverera produktionsklar automation.",
	"dev.hero.cio.prefix": "Letar du efter den övergripande sammanfattningen?",
	"dev.hero.cio.link": "För CIO:er →",
	"dev.hero.card.workflows.title": "Skapa Arbetsflöden",
	"dev.hero.card.workflows.description":
		"Dra, koppla, driftsätt — ingen kod krävs",
	"dev.hero.card.nodes.title": "Skriv Egna Noder",
	"dev.hero.card.nodes.description": "15+ språk, kompilerade till WebAssembly",
	"dev.hero.converge": "Driftsätt i produktion",

	// Path Picker
	"dev.pathpicker.label": "Välj din väg",

	// Workflow Path
	"dev.workflow.badge": "Väg 2",
	"dev.workflow.headline": "Skapa Arbetsflöden Visuellt",
	"dev.workflow.description":
		"Använd den visuella editorn för att dra och koppla färdiga noder till kraftfulla automationsflöden. Ingen kod behövs — koppla, konfigurera och driftsätt.",
	"dev.workflow.feature.dragdrop.title": "Dra & Släpp-byggare",
	"dev.workflow.feature.dragdrop.description":
		"Den visuella arbetsytan låter dig bygga komplexa data- och automationspipelines genom att koppla samman noder. Varje koppling typkontrolleras i realtid.",
	"dev.workflow.feature.catalog.title": "Rik Nodkatalog",
	"dev.workflow.feature.catalog.description":
		"Hundratals färdiga noder för databaser, API:er, AI/ML, filoperationer, meddelanden och mer — redo att använda i ditt arbetsflöde.",
	"dev.workflow.feature.templates.title": "Mallar & Delning",
	"dev.workflow.feature.templates.description":
		"Börja från mallar eller publicera egna. Dela flöden mellan team med versionshantering, så alla kan dra nytta av beprövade mönster.",
	"dev.workflow.feature.interfaces.title": "Bygg Gränssnitt",
	"dev.workflow.feature.interfaces.description":
		"Skapa anpassade användargränssnitt med den inbyggda gränssnittsredigeraren. Förvandla valfritt arbetsflöde till en interaktiv app som ditt team kan använda direkt.",
	"dev.workflow.feature.vcs.title": "Versionshantering",
	"dev.workflow.feature.vcs.description":
		"Varje flöde kan serialiseras och jämföras. Lagra arbetsflöden i Git, granska ändringar i PR:ar och rulla tillbaka med säkerhet.",
	"dev.workflow.feature.typesafe.title": "Typsäker Exekvering",
	"dev.workflow.feature.typesafe.description":
		"In- och utdata valideras vid kompilering. Fånga felmatchningar före driftsättning, inte i produktion.",
	"dev.workflow.howitworks": "Så fungerar det",
	"dev.workflow.step1.title": "Öppna den visuella arbetsytan",
	"dev.workflow.step1.description": "I skrivbordsappen eller webbstudion",
	"dev.workflow.step2.title": "Dra noder från katalogen",
	"dev.workflow.step2.description":
		"Sök, filtrera eller bläddra bland kategorier",
	"dev.workflow.step3.title": "Koppla & konfigurera",
	"dev.workflow.step3.description": "Typsäker koppling med realtidsvalidering",
	"dev.workflow.step4.title": "Kör eller driftsätt",
	"dev.workflow.step4.description": "Lokalt, egenhostat eller i molnet",
	"dev.workflow.cta.docs": "Läs Dokumentationen",
	"dev.workflow.cta.download": "Ladda ner Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "eller",
	"dev.nodes.badge": "Väg 1",
	"dev.nodes.headline": "Skriv Egna Noder",
	"dev.nodes.description":
		"Utöka motorn med din egen logik. Skriv en nod i valfritt språk som stöds — den kompileras till WebAssembly och körs i en sandlåda med full tillgång till värd-SDK:t (loggning, lagring, HTTP, AI-modeller och mer).",
	"dev.nodes.languages.title": "Språk som Stöds",
	"dev.nodes.languages.description":
		"Varje språk levereras med en projektmall och SDK. Välj ett och börja bygga.",
	"dev.nodes.languages.sdk": "Fullständig SDK",
	"dev.nodes.sdk.title": "Värd-SDK-funktioner",
	"dev.nodes.sdk.description":
		"Varje nod får sandlådeskyddad tillgång till dessa plattforms-API:er genom SDK:t:",
	"dev.nodes.sdk.logging": "Loggning",
	"dev.nodes.sdk.logging.desc": "Strukturerad loggutdata",
	"dev.nodes.sdk.pins": "Pins",
	"dev.nodes.sdk.pins.desc": "Läs & skriv nod-I/O",
	"dev.nodes.sdk.variables": "Variabler",
	"dev.nodes.sdk.variables.desc": "Flödesomfattande tillstånd",
	"dev.nodes.sdk.cache": "Cache",
	"dev.nodes.sdk.cache.desc": "Persistent KV cache",
	"dev.nodes.sdk.metadata": "Metadata",
	"dev.nodes.sdk.metadata.desc": "Nod- & flödesinfo",
	"dev.nodes.sdk.streaming": "Streaming",
	"dev.nodes.sdk.streaming.desc": "Strömma stora datamängder",
	"dev.nodes.sdk.storage": "Lagring",
	"dev.nodes.sdk.storage.desc": "Fil- & bloblagring",
	"dev.nodes.sdk.ai": "AI-modeller",
	"dev.nodes.sdk.ai.desc": "LLM- & embedding-anrop",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "Utgående HTTP-förfrågningar",
	"dev.nodes.sdk.auth": "Autentisering",
	"dev.nodes.sdk.auth.desc": "Hemligheter & autentiseringsuppgifter",
	"dev.nodes.feature.sandbox.title": "Sandlåda & Säkerhet",
	"dev.nodes.feature.sandbox.description":
		"Noder körs i en WebAssembly-sandlåda med kapabilitetsbaserade behörigheter. Ingen filsystems- eller nätverksåtkomst om det inte uttryckligen tillåts.",
	"dev.nodes.feature.test.title": "Testa & Iterera",
	"dev.nodes.feature.test.description":
		"Varje mall levereras med ett testramverk. Skriv testfall, kör lokalt och validera innan du publicerar till din katalog.",
	"dev.nodes.feature.publish.title": "Publicera & Versionera",
	"dev.nodes.feature.publish.description":
		"Paketera din nod med metadata, versionera semantiskt och dela den privat eller till den publika katalogen.",
	"dev.nodes.quickstart.title": "Kom Igång Snabbt",
	"dev.nodes.quickstart.description":
		"Öppna Flow-Like Studio, välj en språkmall och börja bygga din anpassade nod:",
	"dev.nodes.quickstart.step1":
		"1. Öppna Flow-Like Studio och navigera till Nodutvecklarsektionen",
	"dev.nodes.quickstart.step2": "2. Välj en mall bland över 15 stödda språk",
	"dev.nodes.quickstart.step3":
		"3. Implementera din nodlogik och bygg WASM-binären",
	"dev.nodes.quickstart.step4":
		"4. Publicera direkt till din katalog från Studio",
	"dev.nodes.cta.guide": "Guide för Nodutveckling",
	"dev.nodes.cta.templates": "Bläddra Bland Alla Mallar",

	// CTA
	"dev.cta.headline": "Redo att bygga?",
	"dev.cta.description":
		"Oavsett om du skapar visuellt eller kodar egna noder — Flow-Like är öppen källkod, lokalt-först och byggt för utvecklare som vill ha full kontroll över sin automationsstack.",
	"dev.cta.download": "Ladda ner Flow-Like",
	"dev.cta.github": "Visa på GitHub",
};
