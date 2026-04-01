export const nlDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "Voor Ontwikkelaars | Flow-Like",
	"dev.meta.description":
		"Twee manieren om te bouwen met Flow-Like: stel workflows visueel samen met bestaande nodes, of schrijf aangepaste nodes in meer dan 15 talen die naar WebAssembly compileren.",

	// Hero
	"dev.hero.badge": "Open Source",
	"dev.hero.headline.prefix": "Voor",
	"dev.hero.headline.highlight": "Ontwikkelaars",
	"dev.hero.description":
		"Of je nu workflows visueel bouwt of aangepaste nodes schrijft in de taal van je keuze — Flow-Like biedt je twee duidelijke paden om productieklare automatisering te leveren.",
	"dev.hero.cio.prefix": "Op zoek naar het executive overzicht?",
	"dev.hero.cio.link": "Voor CIO's →",
	"dev.hero.card.workflows.title": "Workflows Samenstellen",
	"dev.hero.card.workflows.description":
		"Slepen, verbinden, deployen — geen code nodig",
	"dev.hero.card.nodes.title": "Aangepaste Nodes Schrijven",
	"dev.hero.card.nodes.description": "15+ talen, gecompileerd naar WebAssembly",
	"dev.hero.converge": "Naar productie deployen",

	// Path Picker
	"dev.pathpicker.label": "Kies je pad",

	// Workflow Path
	"dev.workflow.badge": "Pad 2",
	"dev.workflow.headline": "Stel Workflows Visueel Samen",
	"dev.workflow.description":
		"Gebruik de visuele editor om kant-en-klare nodes te slepen en te verbinden tot krachtige automatiseringsflows. Geen code nodig — gewoon verbinden, configureren en deployen.",
	"dev.workflow.feature.dragdrop.title": "Drag & Drop Bouwer",
	"dev.workflow.feature.dragdrop.description":
		"Het visuele canvas laat je complexe data- en automatiseringspipelines bouwen door nodes aan elkaar te koppelen. Elke verbinding wordt in realtime gecontroleerd op type.",
	"dev.workflow.feature.catalog.title": "Uitgebreide Node-catalogus",
	"dev.workflow.feature.catalog.description":
		"Honderden kant-en-klare nodes voor databases, API's, AI/ML, bestandsbewerkingen, berichten en meer — klaar om in je workflow te gebruiken.",
	"dev.workflow.feature.templates.title": "Templates & Delen",
	"dev.workflow.feature.templates.description":
		"Begin vanuit templates of publiceer je eigen. Deel flows met teams inclusief versiebeheer, zodat iedereen profiteert van bewezen patronen.",
	"dev.workflow.feature.interfaces.title": "Interfaces Bouwen",
	"dev.workflow.feature.interfaces.description":
		"Maak aangepaste UI's met de ingebouwde interface-editor. Maak van elke workflow een interactieve app die je team direct kan gebruiken.",
	"dev.workflow.feature.vcs.title": "Versiebeheer",
	"dev.workflow.feature.vcs.description":
		"Elke flow is serialiseerbaar en vergelijkbaar. Sla workflows op in Git, review wijzigingen in PR's en draai vol vertrouwen terug.",
	"dev.workflow.feature.typesafe.title": "Typeveilige Uitvoering",
	"dev.workflow.feature.typesafe.description":
		"In- en uitvoer worden gevalideerd tijdens compilatie. Ontdek fouten vóór deployment, niet in productie.",
	"dev.workflow.howitworks": "Hoe het werkt",
	"dev.workflow.step1.title": "Open het visuele canvas",
	"dev.workflow.step1.description": "In de desktopapp of webstudio",
	"dev.workflow.step2.title": "Sleep nodes uit de catalogus",
	"dev.workflow.step2.description": "Zoek, filter of blader door categorieën",
	"dev.workflow.step3.title": "Verbind & configureer",
	"dev.workflow.step3.description":
		"Typeveilige bedrading met realtime validatie",
	"dev.workflow.step4.title": "Uitvoeren of deployen",
	"dev.workflow.step4.description": "Lokaal, self-hosted of in de cloud",
	"dev.workflow.cta.docs": "Lees de Documentatie",
	"dev.workflow.cta.download": "Download Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "of",
	"dev.nodes.badge": "Pad 1",
	"dev.nodes.headline": "Schrijf Aangepaste Nodes",
	"dev.nodes.description":
		"Breid de engine uit met je eigen logica. Schrijf een node in een ondersteunde taal — deze compileert naar WebAssembly en draait in een sandbox met volledige toegang tot de host SDK (logging, opslag, HTTP, AI-modellen en meer).",
	"dev.nodes.languages.title": "Ondersteunde Talen",
	"dev.nodes.languages.description":
		"Elke taal wordt geleverd met een projecttemplate en SDK. Kies er een en begin met bouwen.",
	"dev.nodes.languages.sdk": "Volledige SDK",
	"dev.nodes.sdk.title": "Host SDK-mogelijkheden",
	"dev.nodes.sdk.description":
		"Elke node krijgt via de SDK sandbox-toegang tot deze platform-API's:",
	"dev.nodes.sdk.logging": "Logging",
	"dev.nodes.sdk.logging.desc": "Gestructureerde loguitvoer",
	"dev.nodes.sdk.pins": "Pins",
	"dev.nodes.sdk.pins.desc": "Node I/O lezen & schrijven",
	"dev.nodes.sdk.variables": "Variabelen",
	"dev.nodes.sdk.variables.desc": "Flow-scoped state",
	"dev.nodes.sdk.cache": "Cache",
	"dev.nodes.sdk.cache.desc": "Persistente KV cache",
	"dev.nodes.sdk.metadata": "Metadata",
	"dev.nodes.sdk.metadata.desc": "Node- & flow-info",
	"dev.nodes.sdk.streaming": "Streaming",
	"dev.nodes.sdk.streaming.desc": "Grote data streamen",
	"dev.nodes.sdk.storage": "Opslag",
	"dev.nodes.sdk.storage.desc": "Bestands- & blob-opslag",
	"dev.nodes.sdk.ai": "AI-modellen",
	"dev.nodes.sdk.ai.desc": "LLM- & embedding-aanroepen",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "Uitgaande HTTP-verzoeken",
	"dev.nodes.sdk.auth": "Auth",
	"dev.nodes.sdk.auth.desc": "Secrets & inloggegevens",
	"dev.nodes.feature.sandbox.title": "Sandboxed & Veilig",
	"dev.nodes.feature.sandbox.description":
		"Nodes draaien in een WebAssembly-sandbox met op capabilities gebaseerde rechten. Geen toegang tot bestandssysteem of netwerk tenzij expliciet toegekend.",
	"dev.nodes.feature.test.title": "Testen & Itereren",
	"dev.nodes.feature.test.description":
		"Elke template wordt geleverd met een testharnas. Schrijf fixtures, draai lokaal en valideer voordat je naar je catalogus publiceert.",
	"dev.nodes.feature.publish.title": "Publiceren & Versiebeheer",
	"dev.nodes.feature.publish.description":
		"Verpak je node met metadata, versieer het semantisch en deel het privé of via de publieke catalogus.",
	"dev.nodes.quickstart.title": "Snel Starten",
	"dev.nodes.quickstart.description":
		"Open Flow-Like Studio, kies een taaltemplate en begin met het bouwen van je aangepaste node:",
	"dev.nodes.quickstart.step1":
		"1. Open Flow-Like Studio en navigeer naar de Node-ontwikkelaarssectie",
	"dev.nodes.quickstart.step2":
		"2. Kies een template uit meer dan 15 ondersteunde talen",
	"dev.nodes.quickstart.step3":
		"3. Implementeer je node-logica en bouw de WASM-binary",
	"dev.nodes.quickstart.step4":
		"4. Publiceer direct vanuit de Studio naar je catalogus",
	"dev.nodes.cta.guide": "Node-ontwikkelgids",
	"dev.nodes.cta.templates": "Bekijk Alle Templates",

	// CTA
	"dev.cta.headline": "Klaar om te bouwen?",
	"dev.cta.description":
		"Of je nu visueel samenstelt of aangepaste nodes codeert — Flow-Like is open source, local-first en gebouwd voor ontwikkelaars die volledige controle willen over hun automatiseringsstack.",
	"dev.cta.download": "Download Flow-Like",
	"dev.cta.github": "Bekijk op GitHub",
};
