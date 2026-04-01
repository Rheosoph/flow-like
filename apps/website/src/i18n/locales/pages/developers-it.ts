export const itDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "Per Sviluppatori | Flow-Like",
	"dev.meta.description":
		"Due percorsi per costruire con Flow-Like: componi workflow visivamente con nodi esistenti, oppure scrivi nodi personalizzati in oltre 15 linguaggi compilati in WebAssembly.",

	// Hero
	"dev.hero.badge": "Open Source",
	"dev.hero.headline.prefix": "Per",
	"dev.hero.headline.highlight": "Sviluppatori",
	"dev.hero.description":
		"Che tu costruisca workflow visivamente o scriva nodi personalizzati nel linguaggio che preferisci — Flow-Like ti offre due percorsi chiari per distribuire automazioni pronte per la produzione.",
	"dev.hero.cio.prefix": "Cerchi la panoramica per dirigenti?",
	"dev.hero.cio.link": "Per CIO →",
	"dev.hero.card.workflows.title": "Componi Workflow",
	"dev.hero.card.workflows.description":
		"Trascina, collega, distribuisci — nessun codice necessario",
	"dev.hero.card.nodes.title": "Scrivi Nodi Personalizzati",
	"dev.hero.card.nodes.description":
		"Oltre 15 linguaggi, compilati in WebAssembly",
	"dev.hero.converge": "Distribuisci in produzione",

	// Path Picker
	"dev.pathpicker.label": "Scegli il tuo percorso",

	// Workflow Path
	"dev.workflow.badge": "Percorso 2",
	"dev.workflow.headline": "Componi Workflow Visivamente",
	"dev.workflow.description":
		"Usa l'editor visuale per trascinare e collegare nodi predefiniti in potenti flussi di automazione. Nessun codice necessario — collega, configura e distribuisci.",
	"dev.workflow.feature.dragdrop.title": "Costruttore Drag & Drop",
	"dev.workflow.feature.dragdrop.description":
		"Il canvas visuale ti permette di costruire pipeline complesse di dati e automazione collegando i nodi tra loro. Ogni connessione è verificata sui tipi in tempo reale.",
	"dev.workflow.feature.catalog.title": "Catalogo Nodi Ricco",
	"dev.workflow.feature.catalog.description":
		"Centinaia di nodi predefiniti per database, API, AI/ML, operazioni su file, messaggistica e altro — pronti da inserire nel tuo workflow.",
	"dev.workflow.feature.templates.title": "Template e Condivisione",
	"dev.workflow.feature.templates.description":
		"Parti dai template o pubblica i tuoi. Condividi flussi tra i team con versionamento, così tutti beneficiano di pattern collaudati.",
	"dev.workflow.feature.interfaces.title": "Costruisci Interfacce",
	"dev.workflow.feature.interfaces.description":
		"Crea interfacce personalizzate con l'editor integrato. Trasforma qualsiasi workflow in un'app interattiva utilizzabile direttamente dal tuo team.",
	"dev.workflow.feature.vcs.title": "Controllo di Versione",
	"dev.workflow.feature.vcs.description":
		"Ogni flusso è serializzabile e confrontabile. Salva i workflow in Git, revisiona le modifiche nelle PR e ripristina con sicurezza.",
	"dev.workflow.feature.typesafe.title": "Esecuzione Type-Safe",
	"dev.workflow.feature.typesafe.description":
		"Input e output sono validati in fase di compilazione. Individua le incompatibilità prima del deploy, non in produzione.",
	"dev.workflow.howitworks": "Come funziona",
	"dev.workflow.step1.title": "Apri il canvas visuale",
	"dev.workflow.step1.description": "Nell'app desktop o nello studio web",
	"dev.workflow.step2.title": "Trascina i nodi dal catalogo",
	"dev.workflow.step2.description": "Cerca, filtra o esplora le categorie",
	"dev.workflow.step3.title": "Collega e configura",
	"dev.workflow.step3.description":
		"Connessioni type-safe con validazione in tempo reale",
	"dev.workflow.step4.title": "Esegui o distribuisci",
	"dev.workflow.step4.description": "Locale, self-hosted o cloud",
	"dev.workflow.cta.docs": "Leggi la Documentazione",
	"dev.workflow.cta.download": "Scarica Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "oppure",
	"dev.nodes.badge": "Percorso 1",
	"dev.nodes.headline": "Scrivi Nodi Personalizzati",
	"dev.nodes.description":
		"Estendi il motore con la tua logica. Scrivi un nodo in qualsiasi linguaggio supportato — viene compilato in WebAssembly e viene eseguito in sandbox con accesso completo all'SDK dell'host (logging, storage, HTTP, modelli AI e altro).",
	"dev.nodes.languages.title": "Linguaggi Supportati",
	"dev.nodes.languages.description":
		"Ogni linguaggio viene fornito con un template di progetto e un SDK. Scegline uno e inizia a costruire.",
	"dev.nodes.languages.sdk": "SDK Completo",
	"dev.nodes.sdk.title": "Funzionalità dell'SDK Host",
	"dev.nodes.sdk.description":
		"Ogni nodo ottiene accesso in sandbox a queste API della piattaforma tramite l'SDK:",
	"dev.nodes.sdk.logging": "Logging",
	"dev.nodes.sdk.logging.desc": "Output di log strutturato",
	"dev.nodes.sdk.pins": "Pin",
	"dev.nodes.sdk.pins.desc": "Lettura e scrittura I/O del nodo",
	"dev.nodes.sdk.variables": "Variabili",
	"dev.nodes.sdk.variables.desc": "Stato a livello di flusso",
	"dev.nodes.sdk.cache": "Cache",
	"dev.nodes.sdk.cache.desc": "KV cache persistente",
	"dev.nodes.sdk.metadata": "Metadati",
	"dev.nodes.sdk.metadata.desc": "Info su nodo e flusso",
	"dev.nodes.sdk.streaming": "Streaming",
	"dev.nodes.sdk.streaming.desc": "Trasmissione di grandi volumi di dati",
	"dev.nodes.sdk.storage": "Storage",
	"dev.nodes.sdk.storage.desc": "Archiviazione file e blob",
	"dev.nodes.sdk.ai": "Modelli AI",
	"dev.nodes.sdk.ai.desc": "Chiamate LLM ed embedding",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "Richieste HTTP in uscita",
	"dev.nodes.sdk.auth": "Auth",
	"dev.nodes.sdk.auth.desc": "Segreti e credenziali",
	"dev.nodes.feature.sandbox.title": "Sandboxed e Sicuro",
	"dev.nodes.feature.sandbox.description":
		"I nodi vengono eseguiti in una sandbox WebAssembly con permessi basati su capability. Nessun accesso al filesystem o alla rete se non esplicitamente concesso.",
	"dev.nodes.feature.test.title": "Testa e Itera",
	"dev.nodes.feature.test.description":
		"Ogni template include un framework di test. Scrivi fixture, esegui in locale e valida prima di pubblicare nel catalogo.",
	"dev.nodes.feature.publish.title": "Pubblica e Versiona",
	"dev.nodes.feature.publish.description":
		"Impacchetta il tuo nodo con i metadati, versionalo semanticamente e condividilo privatamente o nel catalogo pubblico.",
	"dev.nodes.quickstart.title": "Avvio Rapido",
	"dev.nodes.quickstart.description":
		"Apri Flow-Like Studio, scegli un template di linguaggio e inizia a costruire il tuo nodo personalizzato:",
	"dev.nodes.quickstart.step1":
		"1. Apri Flow-Like Studio e vai alla sezione Sviluppo Nodi",
	"dev.nodes.quickstart.step2":
		"2. Scegli un template tra oltre 15 linguaggi supportati",
	"dev.nodes.quickstart.step3":
		"3. Implementa la logica del tuo nodo e compila il binario WASM",
	"dev.nodes.quickstart.step4":
		"4. Pubblica direttamente nel tuo catalogo dallo Studio",
	"dev.nodes.cta.guide": "Guida allo Sviluppo dei Nodi",
	"dev.nodes.cta.templates": "Esplora Tutti i Template",

	// CTA
	"dev.cta.headline": "Pronto a costruire?",
	"dev.cta.description":
		"Che tu componga visivamente o scriva nodi personalizzati — Flow-Like è open source, local-first e pensato per sviluppatori che vogliono il pieno controllo sul proprio stack di automazione.",
	"dev.cta.download": "Scarica Flow-Like",
	"dev.cta.github": "Visualizza su GitHub",
};
