export const esDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "Para Desarrolladores | Flow-Like",
	"dev.meta.description":
		"Dos caminos para crear con Flow-Like: compón flujos de trabajo visualmente con nodos existentes, o escribe nodos personalizados en más de 15 lenguajes compilados a WebAssembly.",

	// Hero
	"dev.hero.badge": "Código Abierto",
	"dev.hero.headline.prefix": "Para",
	"dev.hero.headline.highlight": "Desarrolladores",
	"dev.hero.description":
		"Ya sea que construyas flujos de trabajo visualmente o escribas nodos personalizados en el lenguaje que prefieras — Flow-Like te ofrece dos caminos claros para llevar automatización a producción.",
	"dev.hero.cio.prefix": "¿Busca la visión ejecutiva?",
	"dev.hero.cio.link": "Para CIOs →",
	"dev.hero.card.workflows.title": "Componer Flujos de Trabajo",
	"dev.hero.card.workflows.description":
		"Arrastra, conecta, despliega — sin código",
	"dev.hero.card.nodes.title": "Escribir Nodos Personalizados",
	"dev.hero.card.nodes.description":
		"Más de 15 lenguajes, compilados a WebAssembly",
	"dev.hero.converge": "Desplegar en producción",

	// Path Picker
	"dev.pathpicker.label": "Elige tu camino",

	// Workflow Path
	"dev.workflow.badge": "Camino 2",
	"dev.workflow.headline": "Compón Flujos de Trabajo Visualmente",
	"dev.workflow.description":
		"Usa el editor visual para arrastrar y conectar nodos preconstruidos en potentes flujos de automatización. Sin código — solo conecta, configura y despliega.",
	"dev.workflow.feature.dragdrop.title": "Constructor Drag & Drop",
	"dev.workflow.feature.dragdrop.description":
		"El lienzo visual te permite construir pipelines complejos de datos y automatización conectando nodos entre sí. Cada conexión se verifica en tiempo real con validación de tipos.",
	"dev.workflow.feature.catalog.title": "Catálogo de Nodos Completo",
	"dev.workflow.feature.catalog.description":
		"Cientos de nodos preconstruidos para bases de datos, APIs, IA/ML, operaciones de archivos, mensajería y más — listos para incorporar a tu flujo de trabajo.",
	"dev.workflow.feature.templates.title": "Plantillas y Compartición",
	"dev.workflow.feature.templates.description":
		"Comienza a partir de plantillas o publica las tuyas. Comparte flujos entre equipos con versionado, para que todos se beneficien de patrones probados.",
	"dev.workflow.feature.interfaces.title": "Crea Interfaces",
	"dev.workflow.feature.interfaces.description":
		"Crea interfaces personalizadas con el editor de interfaces integrado. Convierte cualquier flujo de trabajo en una aplicación interactiva que tu equipo puede usar directamente.",
	"dev.workflow.feature.vcs.title": "Control de Versiones",
	"dev.workflow.feature.vcs.description":
		"Cada flujo es serializable y comparable. Almacena flujos de trabajo en Git, revisa cambios en PRs y revierte con confianza.",
	"dev.workflow.feature.typesafe.title": "Ejecución con Tipado Seguro",
	"dev.workflow.feature.typesafe.description":
		"Las entradas y salidas se validan en tiempo de compilación. Detecta incompatibilidades antes del despliegue, no en producción.",
	"dev.workflow.howitworks": "Cómo funciona",
	"dev.workflow.step1.title": "Abre el lienzo visual",
	"dev.workflow.step1.description":
		"En la aplicación de escritorio o el estudio web",
	"dev.workflow.step2.title": "Arrastra nodos del catálogo",
	"dev.workflow.step2.description": "Busca, filtra o navega por categorías",
	"dev.workflow.step3.title": "Conecta y configura",
	"dev.workflow.step3.description":
		"Conexiones con tipado seguro y validación en tiempo real",
	"dev.workflow.step4.title": "Ejecuta o despliega",
	"dev.workflow.step4.description": "Local, autoalojado o en la nube",
	"dev.workflow.cta.docs": "Leer la Documentación",
	"dev.workflow.cta.download": "Descargar Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "o",
	"dev.nodes.badge": "Camino 1",
	"dev.nodes.headline": "Escribe Nodos Personalizados",
	"dev.nodes.description":
		"Extiende el motor con tu propia lógica. Escribe un nodo en cualquier lenguaje compatible — se compila a WebAssembly y se ejecuta en un sandbox con acceso completo al SDK del host (logging, almacenamiento, HTTP, modelos de IA y más).",
	"dev.nodes.languages.title": "Lenguajes Compatibles",
	"dev.nodes.languages.description":
		"Cada lenguaje incluye una plantilla de proyecto y SDK. Elige uno y comienza a construir.",
	"dev.nodes.languages.sdk": "SDK Completo",
	"dev.nodes.sdk.title": "Capacidades del SDK del Host",
	"dev.nodes.sdk.description":
		"Cada nodo obtiene acceso aislado a estas APIs de la plataforma a través del SDK:",
	"dev.nodes.sdk.logging": "Logging",
	"dev.nodes.sdk.logging.desc": "Salida de logs estructurada",
	"dev.nodes.sdk.pins": "Pins",
	"dev.nodes.sdk.pins.desc": "Lectura y escritura de E/S del nodo",
	"dev.nodes.sdk.variables": "Variables",
	"dev.nodes.sdk.variables.desc": "Estado con alcance de flujo",
	"dev.nodes.sdk.cache": "Cache",
	"dev.nodes.sdk.cache.desc": "KV cache persistente",
	"dev.nodes.sdk.metadata": "Metadatos",
	"dev.nodes.sdk.metadata.desc": "Información del nodo y del flujo",
	"dev.nodes.sdk.streaming": "Streaming",
	"dev.nodes.sdk.streaming.desc": "Transmisión de grandes volúmenes de datos",
	"dev.nodes.sdk.storage": "Almacenamiento",
	"dev.nodes.sdk.storage.desc": "Almacenamiento de archivos y blobs",
	"dev.nodes.sdk.ai": "Modelos de IA",
	"dev.nodes.sdk.ai.desc": "Llamadas a LLM y embeddings",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "Peticiones HTTP salientes",
	"dev.nodes.sdk.auth": "Auth",
	"dev.nodes.sdk.auth.desc": "Secretos y credenciales",
	"dev.nodes.feature.sandbox.title": "Aislado y Seguro",
	"dev.nodes.feature.sandbox.description":
		"Los nodos se ejecutan en un sandbox de WebAssembly con permisos basados en capacidades. Sin acceso al sistema de archivos ni a la red a menos que se otorgue explícitamente.",
	"dev.nodes.feature.test.title": "Prueba e Itera",
	"dev.nodes.feature.test.description":
		"Cada plantilla incluye un entorno de pruebas. Escribe fixtures, ejecuta localmente y valida antes de publicar en tu catálogo.",
	"dev.nodes.feature.publish.title": "Publica y Versiona",
	"dev.nodes.feature.publish.description":
		"Empaqueta tu nodo con metadatos, versiónalo semánticamente y compártelo de forma privada o en el catálogo público.",
	"dev.nodes.quickstart.title": "Inicio Rápido",
	"dev.nodes.quickstart.description":
		"Abre Flow-Like Studio, elige una plantilla de lenguaje y comienza a construir tu nodo personalizado:",
	"dev.nodes.quickstart.step1":
		"1. Abre Flow-Like Studio y navega a la sección de Desarrollo de Nodos",
	"dev.nodes.quickstart.step2":
		"2. Elige una plantilla de entre más de 15 lenguajes soportados",
	"dev.nodes.quickstart.step3":
		"3. Implementa la lógica de tu nodo y compila el binario WASM",
	"dev.nodes.quickstart.step4":
		"4. Publica directamente en tu catálogo desde el Studio",
	"dev.nodes.cta.guide": "Guía de Desarrollo de Nodos",
	"dev.nodes.cta.templates": "Explorar Todas las Plantillas",

	// CTA
	"dev.cta.headline": "¿Listo para construir?",
	"dev.cta.description":
		"Ya sea que compongas visualmente o programes nodos personalizados — Flow-Like es de código abierto, local-first y está diseñado para desarrolladores que quieren control total sobre su stack de automatización.",
	"dev.cta.download": "Descargar Flow-Like",
	"dev.cta.github": "Ver en GitHub",
};
