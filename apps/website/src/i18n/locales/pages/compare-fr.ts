export const frCompare = {
	// Meta
	"compare.meta.title": "Flow-Like vs Alternatives | Comparaison Complète",
	"compare.meta.description":
		"Découvrez comment Flow-Like se compare à Zapier, n8n, Retool, Power Apps, Airflow et plus. La seule plateforme qui combine automatisation et création d'apps.",

	// Hero
	"compare.hero.tagline": "Vue d'Ensemble",
	"compare.hero.headline": "Comment Flow-Like",
	"compare.hero.headline.highlight": "se compare?",
	"compare.hero.description":
		"La plupart des outils vous forcent à choisir : automatiser OU créer des apps. Flow-Like est la seule plateforme qui fait les deux—avec la performance et la gouvernance exigées par les entreprises.",
	"compare.hero.cta": "Essayer Flow-Like gratuitement",
	"compare.hero.cta.demo": "Voir en action",

	// Category Headers
	"compare.category.execution": "Moteurs d'Exécution",
	"compare.category.execution.desc":
		"Outils conçus pour exécuter des workflows et automatiser des tâches. Excellents pour connecter des systèmes, mais limités pour créer des applications.",
	"compare.category.lowcode": "Constructeurs d'Apps Low-Code",
	"compare.category.lowcode.desc":
		"Outils pour créer des apps internes et des tableaux de bord. Excellents pour les interfaces, mais capacités d'automatisation limitées.",
	"compare.category.orchestration": "Orchestration de Données & Workflows",
	"compare.category.orchestration.desc":
		"Outils enterprise pour pipelines de données complexes et orchestration. Puissants mais nécessitent une expertise technique.",

	// Legend
	"compare.legend.native": "Support natif",
	"compare.legend.partial": "Partiel / Add-on",
	"compare.legend.none": "Non supporté",

	// Capability Labels
	"compare.cap.visual_workflow": "Constructeur visuel de workflows",
	"compare.cap.replayable": "Exécution Rejouable",
	"compare.cap.high_volume": "Flux de données haute volumétrie",
	"compare.cap.compiled": "Logique métier compilée",
	"compare.cap.ai_agents": "Agents IA intégrés",
	"compare.cap.ui_builder": "Constructeur d'interface utilisateur",
	"compare.cap.full_apps": "Livrer des applications complètes",
	"compare.cap.customer_facing": "Apps orientées client",
	"compare.cap.desktop": "Applications desktop",
	"compare.cap.mobile": "Applications mobiles",
	"compare.cap.offline": "Exécution hors-ligne",
	"compare.cap.local_first": "Architecture local-first",
	"compare.cap.file_native": "État basé sur fichiers",
	"compare.cap.data_science": "Workflows data-science",
	"compare.cap.governance": "Gouvernance & RBAC",
	"compare.cap.self_hosted": "Option self-hosted",
	"compare.cap.lock_in": "Pas de dépendance fournisseur",

	// Flow-Like Values
	"compare.fl.visual_workflow": "IR typé",
	"compare.fl.high_volume": "255k/s",
	"compare.fl.compiled": "Rust",
	"compare.fl.desktop": "Tauri",
	"compare.fl.file_native": "Object store",
	"compare.fl.governance": "Graph + portée données",
	"compare.fl.lock_in": "Aucun",

	// Capability Explanations
	"compare.explain.visual_workflow.title": "Constructeur Visuel de Workflows",
	"compare.explain.visual_workflow.what":
		"Concevez des automatisations en connectant des nœuds sur un canevas au lieu d'écrire du code.",
	"compare.explain.visual_workflow.flow":
		"Flow-Like utilise une représentation intermédiaire typée (IR). Chaque connexion sait quels types de données elle accepte, prévenant les erreurs avant qu'elles ne surviennent.",

	"compare.explain.replayable.title": "Exécution Rejouable",
	"compare.explain.replayable.what":
		"Rejouez n'importe quelle exécution passée avec des résultats identiques pour le débogage et les audits.",
	"compare.explain.replayable.flow":
		"Flow-Like enregistre chaque exécution avec un event sourcing complet. Rejouez n'importe quelle exécution avec exactement les mêmes données et obtenez des résultats identiques—essentiel pour le débogage et les audits de conformité.",

	"compare.explain.high_volume.title": "Flux de Données Haute Volumétrie",
	"compare.explain.high_volume.what":
		"Traitez des milliers d'événements par seconde sans effort.",
	"compare.explain.high_volume.flow":
		"Le moteur Rust de Flow-Like atteint 255 000 événements par seconde sur du matériel standard. C'est 1 000× plus rapide que les moteurs typiques—et proportionnellement moins cher.",

	"compare.explain.compiled.title": "Logique Métier Compilée",
	"compare.explain.compiled.what":
		"Votre logique d'automatisation s'exécute en code natif optimisé—pas en scripts interprétés.",
	"compare.explain.compiled.flow":
		"Flow-Like compile les workflows en code Rust natif. Pas de surcharge d'interpréteur JavaScript, pas de démarrages à froid. Vos automatisations s'exécutent à vitesse quasi-matérielle.",

	"compare.explain.ai_agents.title": "Agents IA Intégrés",
	"compare.explain.ai_agents.what":
		"Orchestrez des LLMs et modèles IA comme participants de première classe dans vos workflows.",
	"compare.explain.ai_agents.flow":
		"Flow-Like dispose de nœuds d'agents IA natifs avec garde-fous, limites de débit et journalisation complète. Construisez des pipelines RAG, chatbots ou agents autonomes avec observabilité totale.",

	"compare.explain.ui_builder.title": "Constructeur d'Interface Utilisateur",
	"compare.explain.ui_builder.what":
		"Créez des interfaces pour vos automatisations sans écrire de code frontend.",
	"compare.explain.ui_builder.flow":
		"L'Interface Builder de Flow-Like vous permet de concevoir visuellement des interfaces modernes et interactives connectées directement à vos workflows.",

	"compare.explain.full_apps.title": "Livrer des Applications Complètes",
	"compare.explain.full_apps.what":
		"Packagez votre automatisation + interface en application autonome et déployable.",
	"compare.explain.full_apps.flow":
		"Exportez votre projet Flow-Like comme application complète. Déployez sur web, desktop ou mobile. Pas de déploiement frontend séparé nécessaire.",

	"compare.explain.customer_facing.title": "Apps Orientées Client",
	"compare.explain.customer_facing.what":
		"Construisez des apps que vos clients peuvent utiliser, pas seulement des outils internes.",
	"compare.explain.customer_facing.flow":
		"Les apps Flow-Like peuvent être entièrement personnalisées et déployées publiquement. Construisez des portails clients, produits SaaS ou outils publics avec la même plateforme.",

	"compare.explain.desktop.title": "Applications Desktop",
	"compare.explain.desktop.what":
		"Exécutez vos automatisations comme logiciels desktop natifs.",
	"compare.explain.desktop.flow":
		"Flow-Like utilise Tauri pour compiler en apps desktop natives pour Windows, macOS et Linux. Vos utilisateurs obtiennent une expérience rapide et native avec capacité hors-ligne complète.",

	"compare.explain.mobile.title": "Applications Mobiles",
	"compare.explain.mobile.what": "Déployez sur appareils iOS et Android.",
	"compare.explain.mobile.flow":
		"Construisez une fois, déployez partout. Flow-Like compile en apps mobiles qui fonctionnent hors-ligne et synchronisent une fois connectées.",

	"compare.explain.offline.title": "Exécution Hors-Ligne",
	"compare.explain.offline.what":
		"Les automatisations continuent de fonctionner même sans accès internet.",
	"compare.explain.offline.flow":
		"Flow-Like stocke tout localement d'abord. Vos workflows s'exécutent hors-ligne, mettent les changements en file d'attente et synchronisent automatiquement au retour de la connectivité.",

	"compare.explain.local_first.title": "Architecture Local-First",
	"compare.explain.local_first.what":
		"Les données vivent sur votre appareil par défaut, pas dans le cloud de quelqu'un d'autre.",
	"compare.explain.local_first.flow":
		"Vos projets, données et exécutions restent sur votre matériel. Choisissez quand et quoi synchroniser. Contrôle total sur la résidence de vos données.",

	"compare.explain.file_native.title": "État Basé sur Fichiers",
	"compare.explain.file_native.what":
		"État du projet stocké comme fichiers normaux que vous pouvez versionner, sauvegarder et inspecter.",
	"compare.explain.file_native.flow":
		"Flow-Like utilise un format object store. Votre projet entier est constitué de fichiers portables—mettez-les dans Git, Dropbox ou tout système de sauvegarde.",

	"compare.explain.data_science.title": "Workflows Data-Science",
	"compare.explain.data_science.what":
		"Traitez des datasets, exécutez des modèles ML et construisez des pipelines d'analytics.",
	"compare.explain.data_science.flow":
		"Support natif pour transformations de données, inférence de modèles ML et visualisation. Connectez-vous aux data lakes, exécutez des opérations style pandas et exportez dans n'importe quel format.",

	"compare.explain.governance.title": "Gouvernance & RBAC",
	"compare.explain.governance.what":
		"Contrôlez qui peut voir, éditer et exécuter quoi—jusqu'aux champs de données individuels.",
	"compare.explain.governance.flow":
		"Le système de permissions de Flow-Like est conscient du graphe et scopé aux données. Accordez l'accès à des nœuds, workflows ou même champs de données spécifiques.",

	"compare.explain.self_hosted.title": "Option Self-Hosted",
	"compare.explain.self_hosted.what":
		"Exécutez tout sur votre propre infrastructure.",
	"compare.explain.self_hosted.flow":
		"Déployez Flow-Like on-prem, dans votre cloud privé ou totalement isolé. Vos données n'ont jamais à quitter votre infrastructure. Zéro dépendances externes.",

	"compare.explain.lock_in.title": "Pas de Dépendance Fournisseur",
	"compare.explain.lock_in.what":
		"À quel point est-il difficile de partir si nécessaire?",
	"compare.explain.lock_in.flow":
		"Flow-Like exporte vers des formats standards. Vos workflows sont des fichiers portables, pas enfermés dans une base de données SaaS. Changez de fournisseur ou auto-hébergez sans perte de données.",

	// Insight Section
	"compare.insight.tagline": "L'Insight Clé",
	"compare.insight.headline": "Les autres outils vous forcent à choisir",
	"compare.insight.description":
		"n8n, Node-RED, Zapier, Airflow, Temporal sont des moteurs d'exécution. Retool, Power Apps, Appsmith sont des shells UI. Flow-Like est le seul système qui est les deux—et peut livrer le résultat comme une vraie application.",

	// CTA Section
	"compare.cta.tagline": "Prêt à voir la différence?",
	"compare.cta.headline": "Essayez Flow-Like aujourd'hui",
	"compare.cta.description":
		"Téléchargez l'app desktop et commencez à construire. Pas de carte bancaire requise. Pas d'inscription cloud nécessaire.",
	"compare.cta.download": "Télécharger gratuitement",
	"compare.cta.enterprise": "Démo enterprise",
	"compare.cta.note": "Fonctionne hors-ligne. S'exécute sur votre matériel.",

	// Competitor Descriptions
	"compare.competitor.zapier":
		"La plateforme d'intégration la plus populaire. Excellente pour les automatisations simples, mais personnalisation limitée et coûts élevés à l'échelle.",
	"compare.competitor.n8n":
		"Automatisation de workflows open-source. Auto-hébergeable et flexible, mais basé sur JavaScript sans création d'apps native.",
	"compare.competitor.nodered":
		"Programmation de flux orientée IoT. Excellente pour les projets matériels, limitée pour les applications métier.",
	"compare.competitor.retool":
		"Constructeur rapide d'outils internes. Excellent pour les panneaux d'admin, mais automatisation limitée et dépendant du SaaS.",
	"compare.competitor.powerapps":
		"Plateforme low-code de Microsoft. Intégration profonde Office 365, mais forte dépendance et limité hors de l'écosystème Microsoft.",
	"compare.competitor.superblocks":
		"Constructeur moderne d'outils internes. Interface propre, mais capacités de workflow limitées et principalement cloud.",
	"compare.competitor.appsmith":
		"Constructeur d'apps open-source. Auto-hébergeable et personnalisable, mais focalisé uniquement sur les outils internes.",
	"compare.competitor.airflow":
		"Standard industriel pour l'orchestration de workflows. Puissant pour les pipelines de données, mais nécessite expertise Python et pas de création d'UI.",
	"compare.competitor.temporal":
		"Plateforme d'exécution durable. Excellente pour les workflows complexes, mais orientée développeurs avec courbe d'apprentissage abrupte.",

	// Capability Deep Dive
	"compare.explain.how_does_it": "Comment Flow-Like le fait",
};
