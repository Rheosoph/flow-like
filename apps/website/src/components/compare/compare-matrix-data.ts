export function createCompareMatrixData(
  t: (key: string) => string,
) {
type Support = "native" | "partial" | "none" | string;

interface SupportNoteSource {
  label: string;
  url: string;
}

interface SupportNote {
  summary: string;
  evidence?: string;
  caveat?: string;
  sources?: SupportNoteSource[];
  checkedAt?: string;
}

interface CompetitorResearch {
  summary: string;
  sources: SupportNoteSource[];
  cells?: Partial<Record<string, SupportNote>>;
}

interface Competitor {
  name: string;
  capabilities: Record<string, Support>;
  notes?: Record<string, SupportNote>;
  warning?: string;
  examples?: string;
  exampleNote?: string;
}

interface Category {
  name: string;
  desc: string;
  competitors: Competitor[];
}

const capabilityGroups: { id: string; capabilities: string[] }[] = [
  {
    id: "execution",
    capabilities: ["visual_workflow", "replayable", "high_volume", "compiled"],
  },
  {
    id: "data",
    capabilities: ["file_size", "file_native", "data_science"],
  },
  {
    id: "ai_ux",
    capabilities: ["ai_agents", "ui_builder", "full_apps", "customer_facing"],
  },
  {
    id: "distribution",
    capabilities: ["desktop", "mobile", "offline", "local_first"],
  },
  {
    id: "trust",
    capabilities: ["governance", "self_hosted", "lock_in", "sandbox_isolation", "concurrent_state"],
  },
];

const flowLikeCapabilities: Record<string, Support> = {
  visual_workflow: "native",
  replayable: "native",
  high_volume: "native",
  compiled: "native",
  file_size: "unlimited",
  ai_agents: "native",
  ui_builder: "native",
  full_apps: "native",
  customer_facing: "native",
  desktop: "native",
  mobile: "native",
  offline: "native",
  local_first: "native",
  file_native: "native",
  data_science: "native",
  governance: "native",
  self_hosted: "native",
  lock_in: "low",
  sandbox_isolation: "native",
  concurrent_state: "native",
};

const source = (label: string, url: string): SupportNoteSource => ({ label, url });

const checkedAt = "2026-05-30";

const note = (
  summary: string,
  sources: SupportNoteSource[],
  caveat?: string,
  evidence?: string,
): SupportNote => ({
  summary,
  ...(evidence ? { evidence } : {}),
  ...(caveat ? { caveat } : {}),
  sources,
  checkedAt,
});

const codeFrameworkCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
  dataScienceSummary: string,
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} remains rated none for visual workflow building because the reviewed docs describe code-defined ${focus}, not a native drag-and-drop workflow canvas.`,
    sources,
  ),
  replayable: note(
    `${name} remains rated by its documented runtime primitives rather than by deterministic replay; durable or replayable behavior depends on the surrounding app architecture unless a separate durable runtime is used.`,
    sources,
  ),
  high_volume: note(
    `${name} remains rated none for built-in high-volume execution because scaling is the responsibility of the application, queue, worker, and hosting stack that uses the framework.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because it is used from interpreted application code and model/tool configuration rather than a compiled workflow runtime.`,
    sources,
  ),
  file_size: note(
    `${name} is rated depends for file limits because documents and payloads are handled by the application, vector store, model provider, or storage backend rather than by one product-level upload limit.`,
    sources,
  ),
  file_native: note(
    `${name} is not rated file-native because files are inputs to loaders, retrieval, or tools, not first-class local project artifacts managed by the runtime itself.`,
    sources,
  ),
  data_science: note(dataScienceSummary, sources),
  ui_builder: note(
    `${name} is rated none for UI building because it does not ship a first-party forms/dashboard/app-screen builder.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because it is a framework used inside applications, not a product for packaging and distributing complete apps.`,
    sources,
  ),
  customer_facing: note(
    `${name} is rated none for customer-facing delivery because exposing it to customers requires a separate application, auth layer, hosting model, and operations stack.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop distribution because the reviewed docs describe a developer framework, not a desktop runtime or packaged desktop app.`,
    sources,
  ),
  mobile: note(
    `${name} is rated none for mobile distribution because mobile apps must be built separately around the framework.`,
    sources,
  ),
  offline: note(
    `${name} is rated partial for offline use because framework code can run in customer-controlled environments, but model calls, hosted tracing, and external retrieval/services often remain network-dependent.`,
    sources,
  ),
  local_first: note(
    `${name} is rated partial for local-first use because the library can run locally, but complete local-first data ownership depends on the chosen models, storage, and deployment architecture.`,
    sources,
  ),
  governance: note(
    `${name} is rated none for governance because the open framework itself does not provide enterprise admin, audit, approval, or policy controls by default.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated native for self-hosting because it is a developer framework that can run in customer-controlled applications and infrastructure.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated low lock-in at the framework level because app code can be moved and model/provider integrations can be changed, although app-specific abstractions may still create migration cost.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated none for sandbox isolation because the framework does not provide a hardened default boundary for arbitrary tool or code execution.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated none for concurrent state because transactional multi-user state must be implemented by the surrounding application and data stores.`,
    sources,
  ),
});

const hostedAutomationCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated native for visual workflow building where the product centers on visual scenarios, recipes, Zaps, or workflows for ${focus}.`,
    sources,
  ),
  replayable: note(
    `${name} is not rated as a replayable runtime because public docs emphasize run history, retries, and job operations rather than deterministic event-history replay of business logic.`,
    sources,
  ),
  high_volume: note(
    `${name} remains limited for high-volume workloads because documented platform limits, job quotas, payload caps, or plan tiers shape throughput.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because automations are hosted platform configuration, connector steps, or scripts rather than compiled portable application logic.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are passed through connectors or storage integrations rather than treated as first-class local project artifacts.`,
    sources,
  ),
  data_science: note(
    `${name} is rated none or partial for data science because it can move data between apps, but it is not a data-science notebook, pipeline engine, or local analytical runtime.`,
    sources,
  ),
  ui_builder: note(
    `${name} is limited for UI building because any forms, interfaces, or pages are secondary to automation workflows and do not equal a full application UI platform.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because it ships automations and integration workflows, not complete desktop/mobile/customer applications.`,
    sources,
  ),
  customer_facing: note(
    `${name} is limited for customer-facing delivery because externally exposed experiences are usually forms, portals, embedded integrations, or API endpoints rather than full customer apps.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop delivery because the product is a hosted automation platform, not a desktop application runtime.`,
    sources,
  ),
  mobile: note(
    `${name} is rated none for mobile app delivery because mobile access, if present, is for operating the platform rather than shipping custom mobile apps.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline execution because hosted automations require the vendor cloud and connected services.`,
    sources,
  ),
  local_first: note(
    `${name} is rated none for local-first architecture because workflow definitions, execution, and operational state are centered on the hosted platform.`,
    sources,
  ),
  governance: note(
    `${name} governance depends on its admin, plan, workspace, and audit controls; this note is tied to the public product docs and platform limits reviewed for the matrix.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated none for self-hosting because the reviewed public docs present execution and control as part of the vendor-hosted automation platform.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because workflow definitions, connectors, job history, and operational behavior are coupled to the vendor automation platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because hosted execution separates user code/connectors from local machines, but public docs do not expose portable sandbox guarantees comparable to a dedicated local runtime.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial for concurrent state because the hosted platform manages job execution state, but transactional business-state semantics depend on connected apps and workflow design.`,
    sources,
  ),
});

const openWorkflowToolCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated native for visual workflow building because its public docs center on a node/flow editor for ${focus}.`,
    sources,
  ),
  replayable: note(
    `${name} is not rated native for replayable execution because reviewed docs emphasize executions, logs, and flow operation rather than deterministic replay from event history.`,
    sources,
  ),
  high_volume: note(
    `${name} is not rated native for big-data throughput because it orchestrates integrations and event flows; heavy data processing needs external workers, queues, databases, or data platforms.`,
    sources,
  ),
  compiled: note(
    `${name} is rated around JavaScript/script execution rather than compiled business logic because custom behavior runs through Node.js or JavaScript-style extension points.`,
    sources,
  ),
  file_size: note(
    `${name} has deployment-specific file and payload limits; practical handling depends on runtime configuration, storage mode, reverse proxies, and connected services.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are payloads moving through flows, not local project artifacts owned by the runtime.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for analytics/data work because it can orchestrate data and AI services, but it is not a notebook, model training platform, or analytical storage engine.`,
    sources,
  ),
  ai_agents: note(
    `${name} can connect to AI/LLM tooling through nodes and integrations, but the rating depends on whether first-party agent memory, planning, and tool orchestration are documented.`,
    sources,
  ),
  ui_builder: note(
    `${name} is not rated as a full UI builder because the visual surface builds flows, not custom application screens.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because it runs automations and flows rather than packaging complete customer-facing apps.`,
    sources,
  ),
  customer_facing: note(
    `${name} is rated none for customer-facing app delivery because exposed endpoints or dashboards do not equal a supported external application distribution model.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop delivery because the runtime/editor is not a custom desktop app packaging system.`,
    sources,
  ),
  mobile: note(
    `${name} is rated none for mobile delivery because public docs do not show native mobile app packaging for workflows built in the tool.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline user experiences because flows depend on the running server and connected services.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first for end-user app data; even when self-hosted, flow state and external integrations are server-centered rather than device-local.`,
    sources,
  ),
  governance: note(
    `${name} governance is rated by its documented auth, role, workspace, or runtime security features rather than by a broad enterprise business governance plane.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated native for self-hosting when its public docs support running the runtime under customer control.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated low lock-in relative to SaaS-only automation because workflows can be run in customer-controlled deployments, though node definitions and workflow JSON still create migration cost.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is not rated native for sandbox isolation because public docs do not establish a hardened, portable isolation boundary for arbitrary workflow code/tool execution.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial or none for concurrent state because the runtime can coordinate executions, but transactional application-state semantics depend on external stores and workflow design.`,
    sources,
  ),
});

const lowCodeAppCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated partial for visual workflow building because its visual surface is centered on ${focus}, not durable process orchestration as the primary runtime model.`,
    sources,
  ),
  replayable: note(
    `${name} is not rated native for replayable execution because public docs emphasize app/workflow runs, logs, and retries rather than deterministic replay from workflow history.`,
    sources,
  ),
  high_volume: note(
    `${name} is not rated native for big-data throughput because it is built for operational apps and integrations; bulk or analytical workloads rely on connected databases, APIs, or external compute.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because app behavior is platform configuration, queries, scripts, or hosted workflows rather than portable compiled logic.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are uploaded through widgets, storage integrations, or backend services rather than managed as local project artifacts.`,
    sources,
  ),
  data_science: note(
    `${name} is rated none for data-science workflows because it can connect to data systems but is not itself an ML, notebook, RAG, or analytical pipeline runtime.`,
    sources,
  ),
  ai_agents: note(
    `${name} AI-agent support is rated from first-party product docs, not from whether an app can call an LLM API or embed an external agent service.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated native for end-user UI building when its public docs show forms, pages, dashboards, widgets, or app screens as first-party app-builder primitives.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated native for shipping full apps when the platform supports deployable business applications, even though those apps remain coupled to the vendor runtime.`,
    sources,
  ),
  customer_facing: note(
    `${name} customer-facing delivery depends on the documented sharing, embedding, portal, or authentication model rather than on generic internal app-building features.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none or partial for desktop because public docs focus on browser/platform apps rather than packaging arbitrary custom desktop applications.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated from first-party mobile app/runtime documentation rather than from responsive browser access alone.`,
    sources,
  ),
  offline: note(
    `${name} offline support is rated from documented offline execution or sync behavior; browser-only availability does not count as offline capability.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because app definitions, platform state, and deployment are centered on the vendor platform or server runtime.`,
    sources,
  ),
  governance: note(
    `${name} governance is rated from its documented admin, permissions, audit, deployment, and enterprise controls rather than from generic low-code capability claims.`,
    sources,
  ),
  self_hosted: note(
    `${name} self-hosting is rated from documented self-hosted, on-prem, VPC, or hybrid deployment options, not from API connectivity to private systems.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because app definitions, components, permissions, and runtime behavior are coupled to the vendor platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because hosted execution separates app code from user machines, but public docs do not expose a portable sandbox model for arbitrary untrusted automation.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial for concurrent state because the platform coordinates app/workflow execution, while transactional business-state safety depends on connected databases and app design.`,
    sources,
  ),
});

const biPlatformCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated partial for visual workflow because its visual surfaces are for ${focus}, not for building durable operational workflows.`,
    sources,
  ),
  replayable: note(
    `${name} is rated none for replayable execution because BI refreshes, schedules, or prep jobs do not provide deterministic business-workflow replay.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated partial for high-volume data because it can work with large analytical models or warehouses, but heavy processing depends on extracts, capacity, connected warehouses, or server configuration.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because calculations, semantic models, and dashboards are not compiled portable workflow/application code.`,
    sources,
  ),
  file_native: note(
    `${name} is rated partial or none for file-native work because files are data sources or extracts, not local project artifacts managed by an application runtime.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for data science because it supports analytics and BI modeling, but ML/notebook-style work usually relies on external data platforms or adjacent services.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated native for dashboard/report UI building, but that UI model is analytics-oriented rather than a general app-screen builder.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because dashboards and embedded analytics do not equal complete operational applications.`,
    sources,
  ),
  customer_facing: note(
    `${name} customer-facing support is rated partial because embedded analytics can be exposed externally, while full app delivery still requires another application layer.`,
    sources,
  ),
  desktop: note(
    `${name} desktop support is rated from first-party authoring tools or local clients, not from browser access to published dashboards.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated from first-party mobile dashboard/report consumption, not from custom mobile app generation.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline execution because interactive BI experiences and refreshes normally depend on the service/server and data connections.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because governed semantic models, reports, schedules, and sharing are centered on the BI platform/server.`,
    sources,
  ),
  governance: note(
    `${name} is rated enterprise for governance where public docs show managed permissions, semantic models, deployment controls, and admin/security features.`,
    sources,
  ),
  self_hosted: note(
    `${name} self-hosting is rated from documented server/report-server/customer-hosted deployment options, not from connecting to on-prem data.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because dashboards, semantic models, permissions, and embedded analytics are coupled to the BI platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because platform/server execution boundaries exist, but BI products do not provide a general sandbox for arbitrary agent tools or code.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial for concurrent state because the platform coordinates shared reports/models, while transactional business workflow state belongs in external applications or databases.`,
    sources,
  ),
});

const orchestrationCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated none for visual workflow building because its public docs describe code-defined ${focus}, with UI surfaces focused on monitoring and operations.`,
    sources,
  ),
  replayable: note(
    `${name} replayability is rated from documented rerun, backfill, event-history, or durable execution behavior rather than from a visual editor.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated native for high-volume orchestration because it is designed to coordinate many long-running jobs or workflows across workers and infrastructure.`,
    sources,
  ),
  compiled: note(
    `${name} is rated native for compiled/code-defined logic where business behavior is authored in application code instead of mutable hosted configuration.`,
    sources,
  ),
  file_size: note(
    `${name} file/payload handling is rated from orchestration metadata and payload limits; large files should generally live in external object storage and be passed by reference.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are external inputs/artifacts referenced by jobs, not first-class local project data managed by the runtime.`,
    sources,
  ),
  data_science: note(
    `${name} is rated native for data/analytics workflows because its core use case is orchestrating data pipelines, jobs, services, or long-running compute.`,
    sources,
  ),
  ai_agents: note(
    `${name} is rated none for built-in AI agents because public docs describe workflow/service orchestration, not first-party agent memory, tools, and planning.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated none for UI building because operational consoles are not app-screen builders for end users.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because it provides workflow infrastructure, not packaged business application delivery.`,
    sources,
  ),
  customer_facing: note(
    `${name} is rated none for customer-facing apps because customers interact with applications built on top of it, not with the orchestration runtime itself.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop delivery because it is backend orchestration infrastructure, not a desktop app runtime.`,
    sources,
  ),
  mobile: note(
    `${name} is rated none for mobile delivery because it does not package custom mobile applications.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline user experiences because workflows depend on backend services, workers, and connected systems.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first for end-user app data because workflow state is held by orchestration services and backend stores.`,
    sources,
  ),
  governance: note(
    `${name} governance is rated from its documented auth, namespace, access-control, or deployment controls rather than from business app governance features.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated native for self-hosting when the runtime can be operated by the customer outside a vendor-only SaaS path.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated low lock-in relative to SaaS platforms because workflow code and infrastructure can be customer-controlled, though runtime APIs and histories still create migration cost.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because worker/process/container isolation is deployer-controlled rather than a universal product-level sandbox for untrusted tools.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated native for concurrent workflow state when the runtime coordinates workflow/task state through its scheduler, service, metadata database, or event history.`,
    sources,
  ),
});

const codingAgentCells = (
  name: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated none for visual workflow building because the reviewed docs describe an agent/CLI/developer workspace rather than a visual process builder.`,
    sources,
  ),
  replayable: note(
    `${name} is rated by local session or task history, not deterministic workflow replay; reproducibility depends on logs, prompts, repository state, and model behavior.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated none for high-volume execution because it is optimized for interactive developer tasks, not fleet-scale business workflow throughput.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because it drives tools and edits code rather than compiling its own portable workflow runtime.`,
    sources,
  ),
  file_size: note(
    `${name} is rated depends for file limits because practical limits come from the local workspace, model context, tool configuration, and host environment.`,
    sources,
  ),
  file_native: note(
    `${name} is rated native for file-native workflows when it operates directly on local repositories and workspace files.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for data science because it can edit or run analytical code, but it is not itself a data-science platform or dataset runtime.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated none for UI building because it can edit UI code but does not provide a first-party no-code app-screen builder.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because it helps build software but does not package, host, or distribute complete business apps by itself.`,
    sources,
  ),
  customer_facing: note(
    `${name} is rated none for customer-facing delivery because customer exposure comes from the app the agent edits, not from the agent product itself.`,
    sources,
  ),
  desktop: note(
    `${name} is rated partial for desktop when it provides a local CLI or desktop-adjacent developer surface, but not a custom desktop-app runtime for end users.`,
    sources,
  ),
  mobile: note(
    `${name} is rated none for mobile delivery because it does not ship mobile applications as a product capability.`,
    sources,
  ),
  offline: note(
    `${name} is rated partial for offline use because workspace operations can be local, but model inference and provider authentication are commonly network-dependent.`,
    sources,
  ),
  local_first: note(
    `${name} is rated native or partial for local-first work when the source files live in the user's workspace, with the caveat that model calls may leave the machine.`,
    sources,
  ),
  governance: note(
    `${name} is rated none for governance because the reviewed public docs do not provide enterprise workflow governance, approvals, or audit controls by default.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated native for local/self-hosted control when it can run from source or as a local tool, though model inference may still use external providers.`,
    sources,
  ),
  lock_in: note(
    `${name} is generally low lock-in at the source-code level because the output is repository changes, but prompts, sessions, and provider-specific behavior may still be hard to migrate.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} sandboxing depends on the tool's documented runtime and local configuration; the matrix distinguishes that from Flow-Like's product-level sandbox model.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated none for concurrent state because coding-agent sessions do not provide transactional application state across multiple users or workflows.`,
    sources,
  ),
});

const enterpriseDataPlatformCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated partial for visual workflow building because the reviewed docs show workflow or automation builders for ${focus}, but not a portable visual app runtime.`,
    sources,
  ),
  replayable: note(
    `${name} is rated by platform lineage, process history, and operational execution controls rather than deterministic event-history replay.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated native or partial for high-volume work because the platform is built around enterprise data/process workloads, with practical throughput still shaped by deployment and tenant limits.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because business behavior is modeled in platform artifacts, automations, functions, or scripts rather than portable compiled workflow code.`,
    sources,
  ),
  file_size: note(
    `${name} file/payload limits depend on the concrete dataset, attachment, API, process, or app surface; the public docs do not expose one universal cap for every path.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native in this matrix because files and datasets are governed platform assets, not local project files owned by an offline-first runtime.`,
    sources,
  ),
  data_science: note(
    `${name} is rated native or partial for data science because the reviewed docs show analytics, AI, data pipelines, or operational data modeling as core platform capabilities.`,
    sources,
  ),
  ai_agents: note(
    `${name} AI-agent support is rated from first-party AI/agent builder docs, not from generic LLM API integrations.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated native for UI building when public docs show first-party application, workspace, or low-code screen builders on top of governed enterprise data.`,
    sources,
  ),
  full_apps: note(
    `${name} can deliver operational apps inside the platform, but the rating is limited where those apps remain tied to the vendor data model and runtime.`,
    sources,
  ),
  customer_facing: note(
    `${name} is not treated as a general customer-facing app platform because external delivery depends on vendor-specific portals, SDKs, APIs, or application packaging.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop delivery because the reviewed platform docs focus on web/platform apps and agents, not arbitrary packaged desktop applications.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated partial where public docs mention mobile-capable apps or experiences, but not native mobile app packaging comparable to a dedicated app runtime.`,
    sources,
  ),
  offline: note(
    `${name} is not rated offline-first because enterprise data, workflow state, auth, and governed access are centered on the platform service.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because the canonical data model, permissions, lineage, and workflow state live in the vendor platform.`,
    sources,
  ),
  governance: note(
    `${name} is rated enterprise for governance because public docs emphasize platform security, lineage, access control, auditability, or process governance.`,
    sources,
  ),
  self_hosted: note(
    `${name} self-hosting is rated partial only where customer-controlled or hybrid deployment patterns are documented; it is not an open self-hosted runtime.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because applications, data models, automations, permissions, and AI features depend on the vendor platform model.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because enterprise platform execution boundaries exist, but public docs do not establish a portable sandbox for arbitrary untrusted tools.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated native for concurrent state where platform-managed records, ontology objects, workflows, or process instances coordinate shared enterprise state.`,
    sources,
  ),
});

const enterpriseWorkflowPlatformCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated native for visual workflow building because public docs show first-party builders for ${focus}.`,
    sources,
  ),
  replayable: note(
    `${name} is rated partial for replayability because enterprise workflow products expose histories, retries, tasks, or flow operations, but not deterministic replay of portable workflow code.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated partial for high-volume work because the platform supports enterprise workflow scale, while throughput still depends on tenant limits, connectors, queues, and connected systems.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because logic is platform metadata, flows, actions, scripts, or declarative configuration rather than portable compiled workflow code.`,
    sources,
  ),
  file_size: note(
    `${name} file handling is rated from documented platform attachments, files, or workflow payload paths; individual apps and APIs can impose lower limits.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are attachments or platform records, not first-class local project artifacts.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for data/analytics because reporting, AI, and process insights exist, but the platform is not primarily an ML notebook or data-science runtime.`,
    sources,
  ),
  ai_agents: note(
    `${name} AI support is rated from first-party agent or assistant docs, with a lower rating where agents are scoped to the vendor platform rather than a portable agent runtime.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated native or partial for UI building where public docs show app builders, workspaces, pages, forms, or customer-facing experiences.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated native for full apps when the platform can ship complete enterprise apps, but those apps remain coupled to the platform data and permission model.`,
    sources,
  ),
  customer_facing: note(
    `${name} customer-facing delivery is rated from documented portal, experience, app, or external-user capabilities rather than from internal workflow design alone.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop packaging because the reviewed docs focus on cloud/browser/mobile enterprise applications, not custom desktop app distribution.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated from first-party mobile app or mobile-platform documentation, not from responsive browser access alone.`,
    sources,
  ),
  offline: note(
    `${name} offline support is limited unless the vendor documents explicit mobile/offline sync behavior; most workflow execution remains service-centered.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because data, workflow definitions, permissions, and operational state live in the vendor platform.`,
    sources,
  ),
  governance: note(
    `${name} is rated enterprise for governance because admin, security, access-control, audit, compliance, or workflow governance are core enterprise-platform concerns.`,
    sources,
  ),
  self_hosted: note(
    `${name} self-hosting is rated from documented deployment options; private connectivity or local agents do not make the platform an open customer-run runtime.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because data objects, flows, actions, app builders, permissions, and operational behavior are coupled to the vendor platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because the hosted platform provides execution boundaries, but it is not a portable sandbox for arbitrary untrusted code and tools.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial or native for concurrent state where the platform coordinates records, tasks, flows, or cases, while external transactional semantics still depend on app design.`,
    sources,
  ),
});

const rpaPlatformCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated native for visual workflow building because the public docs show visual automation design surfaces for ${focus}.`,
    sources,
  ),
  replayable: note(
    `${name} is rated partial for replayability because Control Room, queues, histories, or bot operations support monitoring and retries, but not deterministic workflow replay.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated partial for high-volume work because robot fleets and queues can scale automation throughput, while workload capacity depends on robots, sessions, licenses, and infrastructure.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because automations are bot definitions, process models, or platform scripts rather than portable compiled business logic.`,
    sources,
  ),
  file_size: note(
    `${name} file handling depends on the app, bot, storage bucket, queue, or document-automation path; the matrix uses the closest public app-facing limit when available.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because files are inputs, attachments, documents, or storage objects processed by bots rather than local project artifacts.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for data/AI work because intelligent automation and document processing exist, but the platform is not primarily a notebook or ML training runtime.`,
    sources,
  ),
  ai_agents: note(
    `${name} AI-agent support is rated partial where current docs show agents or AI-enhanced automation, but the core product remains RPA/digital-worker orchestration.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated partial for UI building because apps, forms, assistant panels, or attended-automation surfaces exist, but not a general app-screen builder comparable to low-code app platforms.`,
    sources,
  ),
  full_apps: note(
    `${name} is rated none for full apps because the product ships automations and bot-assisted experiences rather than complete portable business apps.`,
    sources,
  ),
  customer_facing: note(
    `${name} customer-facing delivery is limited to portals, assistants, or process participation paths rather than a general external application runtime.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop app delivery even though bots often run on desktops; running robots on machines is not the same as packaging custom desktop apps.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated only where the vendor documents mobile operations or companion apps, not as native mobile app generation.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline user experiences because bot orchestration, identity, queues, and management depend on Control Room or similar platform services.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because automation definitions, robot management, queues, credentials, and audit state are platform-centered.`,
    sources,
  ),
  governance: note(
    `${name} is rated enterprise for governance because RPA platforms emphasize roles, bot management, audit, credential control, and operational governance.`,
    sources,
  ),
  self_hosted: note(
    `${name} is rated partial where on-premises or customer-managed deployment is documented, but cloud and control-plane dependencies still shape the runtime.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because bot definitions, selectors, queues, credentials, and operational tooling are tightly coupled to the RPA platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because robots run in managed sessions or machines, but public docs do not provide a portable hardened sandbox for arbitrary untrusted agent tools.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial for concurrent state because queues and robot orchestration coordinate work items, while transactional business-state safety still depends on the target systems.`,
    sources,
  ),
});

const grcPlatformCells = (
  name: string,
  focus: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} is rated partial for visual workflow building because it supports configurable ${focus}, but it is not a general workflow/runtime builder.`,
    sources,
  ),
  replayable: note(
    `${name} is rated partial for replayability because GRC records, tasks, approvals, and audit trails can be reviewed, but not deterministically replayed as workflow code.`,
    sources,
  ),
  high_volume: note(
    `${name} is rated none or partial for high-volume execution because it manages governance records and assessments, not high-throughput automation or data processing.`,
    sources,
  ),
  compiled: note(
    `${name} is rated none for compiled business logic because logic is configured as platform workflows, questionnaires, policies, or controls rather than compiled portable code.`,
    sources,
  ),
  file_size: note(
    `${name} file limits are rated from public attachment or platform documentation when available; otherwise this depends on tenant configuration and intake path.`,
    sources,
  ),
  file_native: note(
    `${name} is not file-native because evidence files and attachments support governance records rather than local project-file workflows.`,
    sources,
  ),
  data_science: note(
    `${name} is rated partial for analytics because dashboards, monitoring, and risk reporting exist, but it is not an ML or data-science runtime.`,
    sources,
  ),
  ai_agents: note(
    `${name} AI support is rated from public AI governance, Now Assist, or agentic-workflow materials rather than from generic use of external AI tools.`,
    sources,
  ),
  ui_builder: note(
    `${name} is rated partial or native for UI building where it exposes GRC workspaces, forms, questionnaires, or configurable apps, but the scope stays governance-specific.`,
    sources,
  ),
  full_apps: note(
    `${name} is not a general full-app platform; any apps or workspaces are tied to GRC/privacy/risk workflows.`,
    sources,
  ),
  customer_facing: note(
    `${name} customer-facing support is rated only for intake, portal, request, or assessment experiences, not for broad external app delivery.`,
    sources,
  ),
  desktop: note(
    `${name} is rated none for desktop app delivery because the reviewed products are hosted platform suites, not custom desktop runtimes.`,
    sources,
  ),
  mobile: note(
    `${name} mobile support is rated partial where mobile access or platform mobile capabilities apply, not as generated native mobile apps.`,
    sources,
  ),
  offline: note(
    `${name} is rated none for offline execution because governance records, evidence, approvals, and monitoring depend on the platform service.`,
    sources,
  ),
  local_first: note(
    `${name} is not local-first because canonical governance records, controls, policies, and evidence live in the vendor platform.`,
    sources,
  ),
  governance: note(
    `${name} is rated enterprise for governance because ${focus} is the product's core domain rather than an add-on capability.`,
    sources,
  ),
  self_hosted: note(
    `${name} self-hosting is rated from documented cloud, enterprise, or managed deployment options and does not imply an open portable runtime.`,
    sources,
  ),
  lock_in: note(
    `${name} is rated high lock-in because risk records, questionnaires, controls, evidence, workflows, and reporting are modeled inside the vendor platform.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} is rated partial for sandbox isolation because hosted platform boundaries exist, but arbitrary tool/code execution is not the core product capability.`,
    sources,
  ),
  concurrent_state: note(
    `${name} is rated partial for concurrent state because the platform coordinates records, tasks, approvals, and evidence workflows, not arbitrary transactional app state.`,
    sources,
  ),
});

const customBuildCells = (
  name: string,
  model: string,
  sources: SupportNoteSource[],
): Partial<Record<string, SupportNote>> => ({
  visual_workflow: note(
    `${name} are rated none for native visual workflow because ${model} usually delivers custom code and architecture unless a separate workflow-builder product is adopted.`,
    sources,
  ),
  replayable: note(
    `${name} are rated depends for replayability because durable execution, event history, retries, and audit replay must be deliberately designed or sourced from a workflow runtime.`,
    sources,
  ),
  high_volume: note(
    `${name} are rated depends for high-volume workloads because scale depends on architecture choices such as queues, databases, object storage, workers, and operational capacity.`,
    sources,
  ),
  compiled: note(
    `${name} are rated native for compiled business logic when the delivered system is built as owned application code with normal build, test, and release pipelines.`,
    sources,
  ),
  file_size: note(
    `${name} are rated depends for file limits because upload, storage, scanning, retention, and processing limits are product requirements that the implementation chooses.`,
    sources,
  ),
  file_native: note(
    `${name} are rated depends for file-native workflows because local/project-file ownership only exists if the architecture explicitly treats files as first-class artifacts.`,
    sources,
  ),
  data_science: note(
    `${name} are rated depends for data science because notebooks, pipelines, model serving, RAG, and analytics have to be selected and integrated into the custom stack.`,
    sources,
  ),
  ai_agents: note(
    `${name} are rated depends for AI agents because agent memory, tools, permissions, evaluation, and human approval controls must be explicitly built or adopted.`,
    sources,
  ),
  ui_builder: note(
    `${name} are rated native for UI building because custom teams can build arbitrary interfaces, but every screen is implementation work rather than a built-in no-code builder.`,
    sources,
  ),
  full_apps: note(
    `${name} are rated native for full apps because custom development can produce complete web, desktop, mobile, or service applications when scope and budget allow it.`,
    sources,
  ),
  customer_facing: note(
    `${name} are rated native for customer-facing delivery because custom apps can be built for external users, with security, privacy, and operations depending on implementation quality.`,
    sources,
  ),
  desktop: note(
    `${name} are rated depends for desktop because custom desktop packaging is possible, but only if it is explicitly scoped and maintained.`,
    sources,
  ),
  mobile: note(
    `${name} are rated depends for mobile because native or cross-platform mobile apps require separate product scope, distribution, testing, and support.`,
    sources,
  ),
  offline: note(
    `${name} are rated depends for offline because sync, conflict handling, local storage, and background execution are hard product features that must be designed and tested.`,
    sources,
  ),
  local_first: note(
    `${name} are rated depends for local-first because data ownership and offline-first architecture depend on chosen storage, sync, identity, and deployment patterns.`,
    sources,
  ),
  governance: note(
    `${name} are rated depends for governance because access control, audit, policy, compliance evidence, and secure SDLC practices depend on the operating model.`,
    sources,
  ),
  self_hosted: note(
    `${name} are rated by source ownership and deployment rights; custom builds can be self-hosted when contracts, infrastructure, and operations are set up for it.`,
    sources,
  ),
  lock_in: note(
    `${name} lock-in is driven by code ownership, documentation, vendor/contract terms, architecture choices, and operational knowledge transfer.`,
    sources,
  ),
  sandbox_isolation: note(
    `${name} are rated depends for sandbox isolation because safe execution of untrusted code or agent tools requires explicit isolation, threat modeling, and verification.`,
    sources,
  ),
  concurrent_state: note(
    `${name} are rated depends for concurrent state because transactional safety, idempotency, locking, and conflict resolution are implementation responsibilities.`,
    sources,
  ),
});

const competitorResearch: Record<string, CompetitorResearch> = {
  Zapier: {
    summary:
      "Zapier is a hosted automation platform with Zaps, Agents, Tables, and Interfaces. It is strong for SaaS-to-SaaS glue work and light internal experiences, but remains a Zapier-hosted platform rather than a portable app/runtime stack.",
    sources: [
      source("Zapier Agents knowledge files", "https://help.zapier.com/hc/en-us/articles/24569690575117-Add-your-own-data-to-an-agent"),
      source("Zapier product overview", "https://zapier.com/"),
    ],
    cells: {
      ...hostedAutomationCells("Zapier", "SaaS-to-SaaS automation", [
        source("Zapier product overview", "https://zapier.com/"),
        source("Send files in Zaps", "https://help.zapier.com/hc/en-us/articles/8496288813453-Send-files-in-Zaps"),
        source("Zapier Agents", "https://zapier.com/agents"),
      ]),
      visual_workflow: note(
        "Zapier is rated native for visual workflow building because Zapier's editor displays each Zap as a flow diagram with trigger and action steps, sidebar configuration, run history, versions, settings, and Copilot assistance.",
        [
          source("Zapier visual editor", "https://help.zapier.com/hc/en-us/articles/16722578092429-Use-the-editor-to-build-and-view-your-Zaps"),
          source("Zapier product overview", "https://zapier.com/"),
        ],
        "The visual surface builds Zaps and automations, not full portable applications.",
      ),
      sandbox_isolation: note(
        "Zapier is rated partial for sandbox isolation because Code by Zapier runs JavaScript/Python steps in Zapier's managed runtime with documented sandboxing, time, memory, package, and rate limits, but this is not a portable arbitrary-tool sandbox.",
        [
          source("Using Code by Zapier", "https://help.zapier.com/hc/en-us/articles/45405528551181-Using-Code-by-Zapier"),
          source("JavaScript code in Zaps", "https://help.zapier.com/hc/en-us/articles/8496310939021-Use-JavaScript-code-in-Zaps"),
          source("Code by Zapier rate limits", "https://help.zapier.com/hc/en-us/articles/29971850476173-Code-by-Zapier-rate-limits"),
        ],
      ),
      self_hosted: note(
        "Zapier remains rated none for self-hosting because Zaps, Tables, Interfaces, Agents, history, and Code by Zapier execute inside Zapier's hosted automation platform rather than a customer-run Zapier runtime.",
        [
          source("Zapier product overview", "https://zapier.com/"),
          source("Using Code by Zapier", "https://help.zapier.com/hc/en-us/articles/45405528551181-Using-Code-by-Zapier"),
        ],
      ),
    },
  },
  n8n: {
    summary:
      "n8n is a visual workflow automation platform with self-hosting and advanced AI workflow support. It is flexible for API and automation work, but app UI delivery, offline/mobile distribution, and transactional app state are outside its core scope.",
    sources: [
      source("n8n docs", "https://docs.n8n.io/"),
      source("n8n advanced AI docs", "https://docs.n8n.io/advanced-ai/"),
      source("n8n AI Agent node", "https://docs.n8n.io/integrations/builtin/cluster-nodes/root-nodes/n8n-nodes-langchain.agent/"),
      source("n8n data tables", "https://docs.n8n.io/data/schema-preview/"),
      source("n8n RBAC", "https://docs.n8n.io/user-management/rbac/"),
    ],
    cells: {
      ...openWorkflowToolCells("n8n", "workflow automation, integrations, and AI workflow orchestration", [
        source("n8n docs", "https://docs.n8n.io/"),
        source("n8n advanced AI docs", "https://docs.n8n.io/advanced-ai/"),
        source("n8n AI Agent node", "https://docs.n8n.io/integrations/builtin/cluster-nodes/root-nodes/n8n-nodes-langchain.agent/"),
        source("n8n executions", "https://docs.n8n.io/workflows/executions/all-executions/"),
        source("n8n binary data environment variables", "https://docs.n8n.io/hosting/configuration/environment-variables/binary-data/"),
      ]),
      visual_workflow: {
        summary:
          "n8n is rated native for visual workflow building because its docs center on workflows made from connected trigger, action, logic, data, and AI nodes on the canvas.",
        caveat:
          "The visual surface builds automation workflows, not packaged end-user applications.",
        sources: [
          source("n8n docs", "https://docs.n8n.io/"),
          source("n8n workflow basics", "https://docs.n8n.io/workflows/"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "n8n is rated partial for replayability because execution docs support retrying failed workflows and loading previous execution data back into the canvas for debugging, but not deterministic replay from event history.",
        sources: [
          source("n8n executions", "https://docs.n8n.io/workflows/executions/all-executions/"),
          source("n8n workflow history", "https://docs.n8n.io/workflows/history/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "n8n is now rated 200 MB because endpoint configuration documents `N8N_FORMDATA_FILE_SIZE_MAX` as a 200 MiB default for files in form-data webhook payloads.",
        evidence:
          "The same endpoint docs list `N8N_PAYLOAD_SIZE_MAX` as 16 MiB for general payloads, so this row uses the larger documented file-specific webhook setting rather than the older 50 MB Data Tables storage default.",
        caveat:
          "This is not a universal cap for every node. Self-hosted n8n can change endpoint and binary-data settings, while connected services, reverse proxies, memory, and Data Tables limits can be lower.",
        sources: [
          source("n8n endpoint environment variables", "https://docs.n8n.io/hosting/configuration/environment-variables/endpoints/"),
          source("n8n data tables", "https://docs.n8n.io/data/schema-preview/"),
          source("n8n binary data environment variables", "https://docs.n8n.io/hosting/configuration/environment-variables/binary-data/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "n8n is now rated native for AI agents because the official AI Agent node describes an autonomous agent that chooses tools and APIs, and Advanced AI docs cover AI workflows, RAG, chat triggers, tools, and LangChain nodes.",
        caveat:
          "This is agent orchestration inside n8n workflows, not a standalone portable agent runtime.",
        sources: [
          source("n8n AI Agent node", "https://docs.n8n.io/integrations/builtin/cluster-nodes/root-nodes/n8n-nodes-langchain.agent/"),
          source("n8n Advanced AI", "https://docs.n8n.io/advanced-ai/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "n8n is now rated enterprise for governance because current docs list RBAC, projects, custom roles, SAML/OIDC/LDAP options, source-control environments, external secrets, log streaming, and audit/logging surfaces across paid/self-hosted deployments.",
        caveat:
          "Some controls are enterprise-tier features; community self-hosted installs do not include every governance capability.",
        sources: [
          source("n8n RBAC", "https://docs.n8n.io/user-management/rbac/"),
          source("n8n source control and environments", "https://docs.n8n.io/source-control-environments/understand/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "n8n is rated native for self-hosting because official hosting docs include Docker Compose and multiple server/cloud deployment guides for running n8n under customer control.",
        sources: [
          source("n8n Docker Compose", "https://docs.n8n.io/hosting/installation/server-setups/docker-compose/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "n8n remains rated none for sandbox isolation because workflow code, command execution, community nodes, and self-hosted runtime configuration do not amount to a hardened default sandbox for untrusted automation.",
        caveat:
          "Operators can add infrastructure isolation around n8n, but that is deployment architecture rather than a portable product guarantee.",
        sources: [
          source("n8n Code node", "https://docs.n8n.io/code/code-node/"),
          source("n8n Docker Compose", "https://docs.n8n.io/hosting/installation/server-setups/docker-compose/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "n8n is rated partial for concurrent state because it coordinates workflow executions and supports queue/concurrency controls, but durable transactional application state still belongs in external databases or services.",
        sources: [
          source("n8n executions", "https://docs.n8n.io/workflows/executions/all-executions/"),
          source("n8n Cloud concurrency", "https://docs.n8n.io/manage-cloud/concurrency/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Node-RED": {
    summary:
      "Node-RED is a low-code, Node.js-based flow programming tool for event-driven integrations and IoT-style automation. It is excellent for wiring systems and devices, but not a governed business app or AI-agent platform.",
    sources: [
      source("Node-RED docs", "https://nodered.org/docs/"),
      source("Node-RED flow editor", "https://nodered.org/docs/user-guide/editor/"),
      source("Securing Node-RED", "https://nodered.org/docs/user-guide/runtime/securing-node-red"),
      source("Node-RED creating nodes", "https://nodered.org/docs/creating-nodes/node-js"),
    ],
    cells: {
      ...openWorkflowToolCells("Node-RED", "event-driven integrations, IoT-style automation, and Node.js flows", [
        source("Node-RED docs", "https://nodered.org/docs/"),
        source("Node-RED flow editor", "https://nodered.org/docs/user-guide/editor/"),
        source("Node-RED creating nodes", "https://nodered.org/docs/creating-nodes/node-js"),
      ]),
      visual_workflow: {
        summary:
          "Node-RED is rated native for visual workflow building because its core user experience is the browser-based flow editor for wiring nodes into event-driven flows.",
        caveat:
          "This is flow programming, not a full application UI builder with typed app state and distribution.",
        sources: [
          source("Node-RED flow editor", "https://nodered.org/docs/user-guide/editor/"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Node-RED remains rated none for replayable execution because the runtime can show/debug flows and store context, but it does not provide deterministic replay of completed messages or event history.",
        sources: [
          source("Node-RED flow editor", "https://nodered.org/docs/user-guide/editor/"),
          source("Node-RED context", "https://nodered.org/docs/user-guide/context"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Node-RED is now rated none for built-in AI agents because the official docs describe a generic flow runtime and node ecosystem, not first-party agent memory, tool orchestration, or agent runtime semantics.",
        caveat:
          "Community nodes can call LLM services, but that is integration-level extensibility rather than a native agent platform.",
        sources: [
          source("Node-RED docs", "https://nodered.org/docs/"),
          source("Node-RED creating nodes", "https://nodered.org/docs/creating-nodes/node-js"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Node-RED is rated native for self-hosting because the runtime is an installable Node.js application that can run wherever Node.js runs.",
        sources: [
          source("Node-RED getting started", "https://nodered.org/docs/getting-started/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Node-RED remains rated none for governance because official security docs cover runtime/user authentication and editor protection, but not enterprise audit, policy, approvals, or workspace governance.",
        sources: [
          source("Securing Node-RED", "https://nodered.org/docs/user-guide/runtime/securing-node-red"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Node-RED is rated depends for file and payload limits because the runtime is an installable Node.js process; practical limits depend on Node.js memory, HTTP configuration, reverse proxies, dashboard nodes, and connected services.",
        sources: [
          source("Running Node-RED locally", "https://nodered.org/docs/getting-started/local"),
          source("Securing Node-RED", "https://nodered.org/docs/user-guide/runtime/securing-node-red"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Node-RED is rated JS because custom node runtime behavior is authored in JavaScript and registered into the Node-RED runtime, not compiled into portable business logic.",
        sources: [
          source("Node-RED JavaScript nodes", "https://nodered.org/docs/creating-nodes/node-js"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Node-RED is rated none for offline user experiences because flows run in a server/runtime process and usually depend on connected devices, APIs, brokers, or services.",
        caveat:
          "The runtime can operate on a local network or edge device, but that is not the same as offline-capable end-user app sync.",
        sources: [
          source("Running Node-RED locally", "https://nodered.org/docs/getting-started/local"),
          source("Node-RED context", "https://nodered.org/docs/user-guide/context"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Node-RED remains rated none for local-first application data because flow state lives in the runtime's server-side context stores, not in a device-local app data model with sync semantics.",
        sources: [
          source("Node-RED context", "https://nodered.org/docs/user-guide/context"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Node-RED is rated none for sandbox isolation because flows and custom nodes execute inside the Node.js runtime unless the deployer adds external process/container isolation.",
        sources: [
          source("Node-RED JavaScript nodes", "https://nodered.org/docs/creating-nodes/node-js"),
          source("Running Node-RED locally", "https://nodered.org/docs/getting-started/local"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Node-RED is rated none for concurrent application state because context stores can persist values in memory or local files, but docs do not describe transactional concurrent state semantics.",
        sources: [
          source("Node-RED context", "https://nodered.org/docs/user-guide/context"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Retool: {
    summary:
      "Retool builds internal web and mobile apps, workflows, and agents with enterprise governance and self-hosted options. It is still centered on Retool-hosted or Retool-deployed internal software rather than portable offline-first applications.",
    sources: [
      source("Retool docs", "https://docs.retool.com/"),
      source("Retool Mobile", "https://retool.com/products/mobile/"),
      source("Retool Agents docs", "https://docs.retool.com/agents"),
      source("Retool Workflows docs", "https://docs.retool.com/workflows"),
      source("Retool external apps", "https://retool.com/launch-enterprise-apps/external"),
      source("Retool self-hosted deployments", "https://docs.retool.com/self-hosted"),
    ],
    cells: {
      ...lowCodeAppCells("Retool", "internal app screens, database/API queries, mobile apps, workflows, and agents", [
        source("Retool docs", "https://docs.retool.com/"),
        source("Retool Mobile", "https://retool.com/products/mobile/"),
        source("Retool Workflows docs", "https://docs.retool.com/workflows"),
        source("Retool external apps", "https://retool.com/launch-enterprise-apps/external"),
      ]),
      replayable: {
        summary:
          "Retool remains rated none for replayable execution because Workflows docs show scheduling and monitoring jobs, alerts, and ETL tasks, but not deterministic replay or event-history reconstruction.",
        caveat:
          "Retool can monitor workflow runs; this cell is about replayable execution semantics rather than observability.",
        sources: [
          source("Retool Workflows docs", "https://docs.retool.com/workflows"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Retool is rated native for AI agents because Retool documents first-party Agents that encode business processes, connect to systems of record, make deterministic and LLM-based decisions, include humans, and take actions.",
        caveat:
          "This is native inside Retool's hosted/self-hosted platform, not a portable agent runtime that can be embedded independently of Retool.",
        sources: [
          source("Retool Agents docs", "https://docs.retool.com/agents"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Retool is rated native for customer-facing delivery because Retool documents External Apps for customer, vendor, and partner portals, including embedding, dedicated portals, logins, and granular permissions.",
        caveat:
          "External Apps are still hosted/deployed in the Retool platform and priced separately from classic internal apps.",
        sources: [
          source("Retool external apps", "https://retool.com/launch-enterprise-apps/external"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Retool is rated native for mobile because Retool documents native mobile app building for iOS and Android, including distribution to field users.",
        sources: [
          source("Retool Mobile", "https://retool.com/products/mobile/"),
          source("Retool docs", "https://docs.retool.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Retool is rated partial for offline because Retool Mobile advertises offline editing for field apps, but the offline model is scoped to Retool mobile apps rather than a general offline-first runtime.",
        sources: [
          source("Retool Mobile", "https://retool.com/products/mobile/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Retool remains rated around a 40 MB backend upload limit for files that pass through Retool, while direct uploads to external storage can avoid that path.",
        caveat:
          "Retool's public docs emphasize 5 GB of Retool Storage capacity; the 40 MB per-file backend limit is documented in Retool community support threads rather than the main component reference.",
        sources: [
          source("Retool Storage", "https://retool.com/integrations/retool-storage"),
          source("Retool file upload limit discussion", "https://community.retool.com/t/max-filesize-upload-limit/19821"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Retool is now rated enterprise for governance because current docs and trust materials describe organization administration, permissions, SSO, audit logs, source-control workflows, and dedicated or self-managed deployments.",
        sources: [
          source("Retool administration docs", "https://docs.retool.com/org-users"),
          source("Retool permissions docs", "https://docs.retool.com/permissions"),
          source("Retool trust guide", "https://try.retool.com/resource/trust-guide"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Retool is now rated native for self-hosting because Retool documents self-hosted deployments, including self-managed instances that customers deploy and manage on their own Kubernetes infrastructure.",
        caveat:
          "This is proprietary Retool running under customer control, not an open-source runtime with low migration lock-in.",
        sources: [
          source("Retool self-hosted deployments", "https://docs.retool.com/self-hosted"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: note(
        "Retool remains rated none for local-first architecture because apps, permissions, workflows, agents, and deployment are managed in Retool Cloud or a Retool instance, not as device-owned local projects with sync.",
        [
          source("Retool docs", "https://docs.retool.com/"),
          source("Retool self-hosting", "https://retool.com/govern-enterprise-apps/self-hosted"),
          source("Retool Workflows docs", "https://docs.retool.com/workflows"),
        ],
        "Self-hosting can keep Retool in a customer VPC, but the Retool runtime remains the system that serves and operates the apps.",
      ),
      lock_in: note(
        "Retool is rated high lock-in because apps, components, queries, agents, workflows, permissions, and release controls are Retool runtime artifacts even when Git/source-control workflows or self-hosted deployments are used.",
        [
          source("Retool self-hosting", "https://retool.com/govern-enterprise-apps/self-hosted"),
          source("Retool Agents docs", "https://docs.retool.com/agents"),
          source("Retool Workflows docs", "https://docs.retool.com/workflows"),
        ],
      ),
      sandbox_isolation: note(
        "Retool is rated partial for sandbox isolation because hosted or self-hosted Retool separates app and workflow execution from end-user machines and provides permissions/admin controls, but it is not documented as a portable hardened sandbox for arbitrary untrusted tools.",
        [
          source("Retool permissions docs", "https://docs.retool.com/permissions"),
          source("Retool administration docs", "https://docs.retool.com/org-users"),
          source("Retool self-hosting", "https://retool.com/govern-enterprise-apps/self-hosted"),
        ],
      ),
      concurrent_state: note(
        "Retool is rated partial for concurrent state because Retool coordinates app, workflow, and agent runs, while durable business transactions, locking, and conflict handling still depend on the connected databases, APIs, and app design.",
        [
          source("Retool Workflows docs", "https://docs.retool.com/workflows"),
          source("Retool Agents docs", "https://docs.retool.com/agents"),
        ],
      ),
    },
  },
  "Power Apps": {
    summary:
      "Power Apps covers canvas and model-driven business apps, Dataverse, Power Platform governance, mobile/offline players, and now generally available agent creation from apps. It is deep inside the Microsoft ecosystem and is not a general portable runtime.",
    sources: [
      source("Model-driven app overview", "https://learn.microsoft.com/en-us/power-apps/maker/model-driven-apps/model-driven-app-overview"),
      source("Power Apps mobile offline", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-overview"),
      source("Power Apps Agent Builder GA", "https://learn.microsoft.com/en-us/power-platform/release-plan/2025wave1/power-apps/ga-agent-builder-power-apps"),
      source("Dataverse file columns", "https://learn.microsoft.com/en-us/power-apps/developer/data-platform/file-attributes"),
    ],
    cells: {
      ...lowCodeAppCells("Power Apps", "canvas/model-driven app building, Dataverse, and Microsoft Power Platform workflows", [
        source("Model-driven app overview", "https://learn.microsoft.com/en-us/power-apps/maker/model-driven-apps/model-driven-app-overview"),
        source("Power Apps mobile offline", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-overview"),
        source("Dataverse file columns", "https://learn.microsoft.com/en-us/power-apps/developer/data-platform/file-attributes"),
      ]),
      file_size: {
        summary:
          "Power Apps is now rated 10 GB for file-column storage because Microsoft documents Dataverse file columns with a maximum `MaxSizeInKB` value of 10 GB.",
        evidence:
          "Microsoft also documents chunked transfers for large files and a 128 MB limit for sending a file in a single request.",
        caveat:
          "This does not make every Power Apps file path 10 GB. Attachment/note paths, single-request uploads, controls, portals, and connector paths can be lower.",
        sources: [
          source("Dataverse file column definitions", "https://learn.microsoft.com/en-us/power-apps/developer/data-platform/file-attributes"),
          source("Use Dataverse file columns", "https://learn.microsoft.com/en-us/power-apps/developer/data-platform/file-column-data"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Power Apps is now rated native for AI agents because Microsoft's current release plan marks Agent Builder for Power Apps generally available, and maker docs describe creating agents from existing canvas apps.",
        caveat:
          "Agent Builder is currently scoped to canvas apps and requires supported Copilot Studio/Dataverse environment prerequisites.",
        sources: [
          source("Power Apps Agent Builder GA", "https://learn.microsoft.com/en-us/power-platform/release-plan/2025wave1/power-apps/ga-agent-builder-power-apps"),
          source("Build an AI agent from a canvas app", "https://learn.microsoft.com/en-us/power-apps/maker/canvas-apps/agent-builder"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Power Apps is rated native for customer-facing delivery through the broader Power Platform because Microsoft documents Power Pages for external-facing websites where outside users can sign in, create/view Dataverse data, or browse anonymously.",
        caveat:
          "This is Power Pages/Power Platform delivery, not standalone packaging of a Power Apps canvas app as an arbitrary customer app.",
        sources: [
          source("Power Pages documentation", "https://learn.microsoft.com/en-us/power-pages/"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Power Apps is rated partial for desktop because Microsoft provides Power Apps for Windows to run model-driven and canvas apps, but this is a Microsoft player/runtime rather than packaging arbitrary native desktop applications.",
        sources: [
          source("Install Power Apps for Windows", "https://learn.microsoft.com/en-us/power-apps/mobile/windows-app-install"),
          source("Use Power Apps for Windows", "https://learn.microsoft.com/en-us/power-apps/mobile/windows-app-use"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Power Apps is rated native for mobile because Microsoft documents Power Apps mobile players and mobile offline profiles for business apps.",
        sources: [
          source("Power Apps mobile offline overview", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-works-overview"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Power Apps is rated native for offline because Microsoft documents offline-first functionality that stores app data in a local database on the device and syncs writes back to Dataverse.",
        caveat:
          "Offline support is tied to Dataverse/offline profiles and Power Apps mobile, not arbitrary local-first app architecture.",
        sources: [
          source("How mobile offline works in Power Apps", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-works-overview"),
          source("Set up mobile offline for canvas apps", "https://learn.microsoft.com/en-us/power-apps/mobile/canvas-mobile-offline-setup"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: note(
        "Power Apps remains rated none for local-first architecture because the documented offline-first experience caches Dataverse data locally and syncs back to Dataverse; app assets, environments, solutions, and governance still live in Power Platform.",
        [
          source("How mobile offline works in Power Apps", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-works-overview"),
          source("Power Platform ALM basics", "https://learn.microsoft.com/en-us/power-platform/alm/basics-alm"),
        ],
        "This distinguishes useful mobile offline operation from a local-first app architecture where the local project/runtime is the primary source of truth.",
      ),
      self_hosted: note(
        "Power Apps is rated none for self-hosting because Microsoft documents on-premises gateways for secure access to private resources, but the gateway is a connector bridge and does not self-host the Power Apps runtime.",
        [
          source("Power Platform on-premises gateway", "https://learn.microsoft.com/en-us/power-platform/admin/wp-onpremises-gateway"),
          source("Power Platform ALM basics", "https://learn.microsoft.com/en-us/power-platform/alm/basics-alm"),
        ],
      ),
      lock_in: note(
        "Power Apps is rated high lock-in because apps, solutions, Dataverse metadata, Power Fx/canvas assets, flows, environments, and ALM pipelines are managed as Power Platform artifacts.",
        [
          source("Power Platform ALM basics", "https://learn.microsoft.com/en-us/power-platform/alm/basics-alm"),
          source("Dataverse file columns", "https://learn.microsoft.com/en-us/power-apps/developer/data-platform/file-attributes"),
        ],
      ),
      sandbox_isolation: note(
        "Power Apps is rated partial for sandbox isolation because Power Platform provides environments, Dataverse sandbox environments, tenant isolation, roles, and connector controls, but it is not a general sandbox for arbitrary untrusted code or tools.",
        [
          source("Power Platform ALM basics", "https://learn.microsoft.com/en-us/power-platform/alm/basics-alm"),
          source("Power Platform tenant isolation", "https://learn.microsoft.com/en-us/power-platform/admin/cross-tenant-restrictions"),
        ],
      ),
      concurrent_state: note(
        "Power Apps is rated partial for concurrent state because Dataverse-backed apps can sync offline changes and use platform data services, but app-level conflict handling and transactional workflow safety depend on Dataverse design, connectors, and solution architecture.",
        [
          source("How mobile offline works in Power Apps", "https://learn.microsoft.com/en-us/power-apps/mobile/mobile-offline-works-overview"),
          source("Power Platform ALM basics", "https://learn.microsoft.com/en-us/power-platform/alm/basics-alm"),
        ],
      ),
    },
  },
  Superblocks: {
    summary:
      "Superblocks is an internal-app platform with AI-generated apps, governed integrations, RBAC, audit logs, Git/SDLC hooks, and cloud, hybrid, or Cloud-Prem deployment. Its scope is internal operations apps, not offline app distribution.",
    sources: [
      source("Superblocks docs", "https://docs.superblocks.com/"),
      source("Superblocks hosting overview", "https://docs.superblocks.com/hosting/overview"),
      source("Superblocks size and time limits", "https://docs.superblocks.com/enterprise/hybrid-architecture/manage/size_and_time_limits"),
      source("Building with Clark AI", "https://docs.superblocks.com/building-with-clark"),
    ],
    cells: {
      ...lowCodeAppCells("Superblocks", "internal apps, governed integrations, workflows, and embedded app delivery", [
        source("Superblocks docs", "https://docs.superblocks.com/"),
        source("Superblocks hosting overview", "https://docs.superblocks.com/hosting/overview"),
        source("Superblocks size and time limits", "https://docs.superblocks.com/enterprise/hybrid-architecture/manage/size_and_time_limits"),
        source("Building with Clark AI", "https://docs.superblocks.com/building-with-clark"),
      ]),
      replayable: {
        summary:
          "Superblocks remains rated none for replayable execution because Clark checkpoints and rollback track app-generation/edit state, not deterministic replay of deployed business workflow execution.",
        sources: [
          source("Superblocks checkpoints and rollbacks", "https://docs.superblocks.com/building-with-clark/checkpoints"),
          source("Building with Clark AI", "https://docs.superblocks.com/building-with-clark"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Superblocks is rated partial for AI agents because Clark is documented as an AI coding agent for generating and editing internal apps, while the core product is not a general autonomous-agent runtime for end-user workflows.",
        caveat:
          "Clark can inspect integrations, plan, build, test, and debug apps, so this is stronger than a simple LLM helper but still app-builder scoped.",
        sources: [
          source("Building with Clark AI", "https://docs.superblocks.com/building-with-clark"),
          source("Plan and Build modes", "https://docs.superblocks.com/building-with-clark/plan-and-build-modes"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Superblocks is now rated 50 MB for practical file/payload handling because the hybrid data-plane docs list a 50 MB REST API response limit by default; lower and higher limits also exist for gRPC request and response messages.",
        evidence:
          "The data-plane docs list 30 MB for incoming gRPC messages, 100 MB for outgoing gRPC messages, and 50 MB for REST API responses by default.",
        caveat:
          "These limits are configurable in hybrid deployments, so the rating captures documented defaults rather than a hard product maximum.",
        sources: [
          source("Superblocks size and time limits", "https://docs.superblocks.com/enterprise/hybrid-architecture/manage/size_and_time_limits"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Superblocks is rated native for customer-facing delivery because it documents embedded apps for customer portals and product experiences, including public, private, and SSO authentication models.",
        sources: [
          source("Superblocks hosting overview", "https://docs.superblocks.com/hosting/overview"),
          source("Superblocks embedded authentication", "https://docs.superblocks.com/hosting/embedded-apps/authentication"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Superblocks is rated partial for self-hosting because its current docs describe Cloud, Hybrid, and Cloud-Prem deployment models, but Superblocks still manages or participates in the platform control plane/lifecycle rather than shipping an open customer-run runtime.",
        sources: [
          source("Superblocks enterprise deployment", "https://docs.superblocks.com/enterprise/deployment-overview"),
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
          source("Superblocks Cloud-Prem AWS", "https://docs.superblocks.com/enterprise/cloud-prem/aws"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Superblocks is now rated enterprise for governance because docs describe RBAC, SCIM, SSO, audit logs, Git/SDLC controls, policy agents, and IT visibility over generated apps.",
        sources: [
          source("Introduction to Superblocks", "https://docs.superblocks.com/getting-started/what-is-superblocks"),
          source("Superblocks RBAC", "https://docs.superblocks.com/admin/org-administration/rbac"),
          source("Superblocks audit logs", "https://docs.superblocks.com/admin/audit-logs"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: note(
        "Superblocks is rated none for mobile because current docs center on web internal apps and JavaScript/React embedding, not native mobile app builders, mobile players, or app-store packaging.",
        [
          source("Superblocks embedded apps", "https://docs.superblocks.com/hosting/embedded-apps/overview"),
          source("Superblocks hosting overview", "https://docs.superblocks.com/hosting/overview"),
        ],
      ),
      offline: note(
        "Superblocks is rated none for offline because Hybrid and Cloud-Prem docs describe browser-loaded apps and API/workflow execution through Superblocks control or data planes, with no documented offline sync runtime.",
        [
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
          source("Superblocks hosting overview", "https://docs.superblocks.com/hosting/overview"),
        ],
      ),
      local_first: note(
        "Superblocks remains rated none for local-first architecture because app definitions, permissions, RBAC, embedding, and control-plane governance live in Superblocks even when production data-plane execution runs inside a customer VPC.",
        [
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
          source("Superblocks RBAC", "https://docs.superblocks.com/admin/org-administration/rbac"),
        ],
      ),
      lock_in: note(
        "Superblocks is rated high lock-in because applications, Clark-generated app state, integrations, RBAC, workflows, embedded delivery, and deployment behavior depend on the Superblocks platform model.",
        [
          source("Building with Clark AI", "https://docs.superblocks.com/building-with-clark"),
          source("Superblocks embedded apps", "https://docs.superblocks.com/hosting/embedded-apps/overview"),
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
        ],
      ),
      sandbox_isolation: note(
        "Superblocks is rated partial for sandbox isolation because Hybrid can run production APIs in a private customer VPC with no inbound exposure, but docs do not present Superblocks as a portable hardened sandbox for arbitrary untrusted tools.",
        [
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
          source("Superblocks RBAC", "https://docs.superblocks.com/admin/org-administration/rbac"),
        ],
      ),
      concurrent_state: note(
        "Superblocks is rated partial for concurrent state because the platform coordinates apps, workflows, scheduled jobs, and API execution, while transactional business-state guarantees remain in connected systems and implementation design.",
        [
          source("Superblocks hybrid overview", "https://docs.superblocks.com/enterprise/hybrid-architecture/overview"),
          source("Superblocks RBAC", "https://docs.superblocks.com/admin/org-administration/rbac"),
        ],
      ),
    },
  },
  Appsmith: {
    summary:
      "Appsmith is an open-source internal app builder with drag-and-drop widgets, database/API integrations, JavaScript logic, Git workflows, self-hosting, and a separate Appsmith Agents product. Its main app-builder surface is still internal-tool oriented.",
    sources: [
      source("Appsmith docs", "https://docs.appsmith.com/"),
      source("Appsmith Agents docs", "https://docs.appsmithai.com/"),
      source("Appsmith pricing", "https://www.appsmith.com/pricing"),
    ],
    cells: {
      ...lowCodeAppCells("Appsmith", "internal app pages, widgets, JavaScript logic, and data-source integrations", [
        source("Appsmith docs", "https://docs.appsmith.com/"),
        source("Appsmith Agents docs", "https://docs.appsmithai.com/"),
        source("Appsmith pricing", "https://www.appsmith.com/pricing"),
      ]),
      replayable: {
        summary:
          "Appsmith remains rated none for replayable execution because Workflow Run History records successful and failed runs with chronological logs, but does not provide deterministic workflow replay.",
        sources: [
          source("Appsmith Workflow Run History", "https://docs.appsmith.com/workflows/reference/run-history"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Appsmith is now rated depends for file limits because current public docs describe file-picker style uploads and app sharing, but do not establish a single platform-wide file/payload ceiling.",
        evidence:
          "Appsmith materials describe a configurable maximum file size on the Filepicker widget, and older support discussions note different behavior for files above 5 MB rather than a hard global cap.",
        caveat:
          "The real limit depends on widget configuration, Appsmith Cloud versus self-hosting, request timeouts, and the target API or storage service.",
        sources: [
          source("Appsmith docs", "https://docs.appsmith.com/"),
          source("Appsmith Filepicker blog", "https://www.appsmith.com/blog/upload-and-manage-files-on-cloudinary-with-the-filepicker-widget"),
          source("Appsmith 5 MB filepicker discussion", "https://old-community.appsmith.com/t/filepicker-upload-file-bigger-than-5mb-catch-uncaughtpromiserejection/1700"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Appsmith is now rated native for AI agents because Appsmith Agents is a first-party platform for secure embedded agents powered by business data, with RAG, integrations, widgets, JavaScript logic, workflows, access control, and audit logs.",
        caveat:
          "This is Appsmith Agents rather than the classic open-source Appsmith app builder alone, so deployments should verify product availability and licensing.",
        sources: [
          source("Appsmith Agents docs", "https://docs.appsmithai.com/"),
          source("Appsmith Agents launch", "https://www.appsmith.com/blog/introducing-appsmith-agents"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Appsmith is rated native for customer-facing delivery because docs and pricing describe public embedding, private embedding, and external-client-portal patterns.",
        caveat:
          "Private embedding and external portals are enterprise-oriented capabilities, and Appsmith remains a platform-hosted or self-hosted app runtime.",
        sources: [
          source("Embed Appsmith", "https://docs.appsmith.com/advanced-concepts/embed-appsmith-into-existing-application"),
          source("Appsmith pricing", "https://www.appsmith.com/pricing"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Appsmith is now rated enterprise for governance because current pricing and docs list granular access controls, audit logs, SAML/OIDC SSO, SCIM, CI/CD, private embedding, SOC 2 Type 2, and airgapped options on paid tiers.",
        sources: [
          source("Appsmith pricing", "https://www.appsmith.com/pricing"),
          source("Appsmith audit logs", "https://docs.appsmith.com/advanced-concepts/audit-logs"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Appsmith is rated native for self-hosting because official docs provide Docker, Kubernetes, cloud-provider, and airgapped installation paths for running Appsmith under customer control.",
        sources: [
          source("Appsmith Docker install", "https://docs.appsmith.com/getting-started/setup/installation-guides/docker"),
          source("Appsmith pricing", "https://www.appsmith.com/pricing"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: note(
        "Appsmith is rated none for mobile because official docs position Appsmith as a web-application/internal-tool builder with widgets, datasources, queries, and JavaScript, not a native mobile app runtime or packaging system.",
        [
          source("Appsmith overview", "https://docs.appsmith.com/build-apps/overview"),
          source("Appsmith docs", "https://docs.appsmith.com/"),
        ],
      ),
      offline: note(
        "Appsmith is rated none for offline because docs describe web apps connected to datasources, queries, JavaScript, and an Appsmith Cloud or self-hosted server, with no documented offline sync runtime.",
        [
          source("Appsmith overview", "https://docs.appsmith.com/build-apps/overview"),
          source("Appsmith Docker install", "https://docs.appsmith.com/getting-started/setup/installation-guides/docker"),
        ],
      ),
      local_first: note(
        "Appsmith remains rated none for local-first architecture because apps run through Appsmith Cloud or a self-hosted Appsmith server, and datasources/queries remain server-backed rather than device-local with sync.",
        [
          source("Appsmith overview", "https://docs.appsmith.com/build-apps/overview"),
          source("Appsmith Docker install", "https://docs.appsmith.com/getting-started/setup/installation-guides/docker"),
        ],
      ),
      lock_in: note(
        "Appsmith is rated low lock-in relative to proprietary low-code platforms because it is open-source, supports self-hosting, and documents Git version control across common Git providers, even though Appsmith apps still depend on Appsmith's runtime.",
        [
          source("Appsmith docs", "https://docs.appsmith.com/"),
          source("Appsmith Docker install", "https://docs.appsmith.com/getting-started/setup/installation-guides/docker"),
          source("Appsmith Git version control", "https://docs.appsmith.com/advanced-concepts/version-control-with-git"),
        ],
      ),
      sandbox_isolation: note(
        "Appsmith is rated none for sandbox isolation because docs focus on app widgets, server-backed queries, JavaScript expressions, roles, and audit logs rather than a hardened sandbox for arbitrary untrusted code or tool execution.",
        [
          source("Appsmith overview", "https://docs.appsmith.com/build-apps/overview"),
          source("Appsmith audit logs", "https://docs.appsmith.com/advanced-concepts/audit-logs"),
        ],
      ),
      concurrent_state: note(
        "Appsmith is rated partial for concurrent state because it keeps query responses as reactive application state and can write to connected datasources, but transactional locking and conflict handling live in those databases/APIs and app logic.",
        [
          source("Appsmith overview", "https://docs.appsmith.com/build-apps/overview"),
          source("Appsmith context object", "https://docs.appsmith.com/reference/appsmith-framework/context-object"),
        ],
      ),
    },
  },
  Tableau: {
    summary:
      "Tableau is a BI and analytics platform with Tableau Prep flows, Tableau Server/Cloud scheduling, mobile apps, and enterprise governance. It is strong for analytics workflows, but not for building operational apps or autonomous agents.",
    sources: [
      source("Tableau Prep Flow Workspace", "https://help.tableau.com/current/server/en-us/prep_conductor_workspace.htm"),
      source("Tableau Prep on the Web", "https://help.tableau.com/current/prep/en-gb/prep_web_auth.htm"),
      source("Tableau Agent", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
      source("Tableau Embedding API authentication", "https://help.tableau.com/current/api/embedding_api/en-us/docs/embedding_api_auth.html"),
      source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
      source("Tableau Mobile", "https://www.tableau.com/products/mobile"),
    ],
    cells: {
      ...biPlatformCells("Tableau", "dashboards, Tableau Prep flows, extracts, and governed analytics", [
        source("Tableau Prep Flow Workspace", "https://help.tableau.com/current/server/en-us/prep_conductor_workspace.htm"),
        source("Tableau Prep on the Web", "https://help.tableau.com/current/prep/en-gb/prep_web_auth.htm"),
        source("Tableau Agent", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
        source("Tableau Embedding API authentication", "https://help.tableau.com/current/api/embedding_api/en-us/docs/embedding_api_auth.html"),
        source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
        source("Tableau Mobile", "https://www.tableau.com/products/mobile"),
      ]),
      visual_workflow: note(
        "Tableau is rated partial for visual workflow because Tableau Prep provides visual flow authoring and Tableau dashboards are visual analytics surfaces, but this is data-prep/BI workflow rather than operational app automation.",
        [
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
          source("Tableau Prep Flow Workspace", "https://help.tableau.com/current/server/en-us/prep_conductor_workspace.htm"),
        ],
      ),
      replayable: note(
        "Tableau remains rated none for replayable execution because Prep Conductor can schedule and track flow runs, but it does not provide deterministic business-workflow replay from event history.",
        [
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
        ],
      ),
      high_volume: note(
        "Tableau is rated partial for high-volume data because it supports extracts, Prep flows, scheduled refresh, and Hyper update paths, while practical scale depends on Tableau Cloud/Server capacity, data sources, extracts, and APIs.",
        [
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
          source("Tableau Hyper update REST API", "https://help.tableau.com/current/api/rest_api/en-us/REST/rest_api_how_to_update_data_to_hyper.htm"),
        ],
      ),
      compiled: note(
        "Tableau is rated none for compiled business logic because calculations, dashboards, flows, and data models are Tableau analytics assets rather than portable compiled workflow/application code.",
        [
          source("Tableau Prep Flow Workspace", "https://help.tableau.com/current/server/en-us/prep_conductor_workspace.htm"),
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
        ],
      ),
      file_size: {
        summary:
          "Tableau remains rated depends because file and payload limits vary by surface: Tableau Prep on the Web limits file connections to 1 GB, while Hyper update uploads use separate REST API limits.",
        caveat:
          "Tableau Desktop, Tableau Cloud, Tableau Server configuration, extract format, and API path can all change the practical limit.",
        sources: [
          source("Tableau Prep on the Web", "https://help.tableau.com/current/prep/en-gb/prep_web_auth.htm"),
          source("Tableau Hyper update REST API", "https://help.tableau.com/current/api/rest_api/en-us/REST/rest_api_how_to_update_data_to_hyper.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Tableau is rated partial for file-native work because workbooks, packaged workbooks, extracts, Hyper files, and flat-file connections are common BI artifacts, but files are still analytics inputs/outputs rather than local app project state.",
        [
          source("Tableau Prep on the Web", "https://help.tableau.com/current/prep/en-gb/prep_web_auth.htm"),
          source("Tableau Hyper update REST API", "https://help.tableau.com/current/api/rest_api/en-us/REST/rest_api_how_to_update_data_to_hyper.htm"),
        ],
      ),
      data_science: note(
        "Tableau is rated partial for data science because it supports data preparation, visual analytics, calculations, and AI-assisted analytics, while model training and notebook workflows live in adjacent tools.",
        [
          source("Tableau Agent help", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
          source("AI in Tableau", "https://www.tableau.com/solutions/ai-analytics"),
        ],
      ),
      ai_agents: {
        summary:
          "Tableau remains rated partial for AI agents because Tableau Agent and Tableau Next provide agentic analytics assistance for data preparation, visualization, calculations, and Q&A, but they are analytics assistants rather than a general autonomous agent runtime.",
        caveat:
          "Tableau Agent is scoped to Tableau Desktop, Cloud, Server web authoring, Prep, Catalog, and Tableau Next analytics experiences.",
        sources: [
          source("Tableau Agent help", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
          source("Tableau Next", "https://www.tableau.com/products/tableau-next"),
          source("AI in Tableau", "https://www.tableau.com/solutions/ai-analytics"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "Tableau is rated native for UI building in the analytics sense because users build dashboards, worksheets, stories, and embedded visualizations, but that UI model is BI-specific.",
        [
          source("Tableau Embedding API", "https://help.tableau.com/current/api/embedding_api/en-us/index.html"),
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
        ],
      ),
      full_apps: note(
        "Tableau remains rated none for full apps because dashboards, Prep flows, and embedded visualizations require another application layer for operational workflows, auth, transactions, and app logic.",
        [
          source("Tableau Embedding API authentication", "https://help.tableau.com/current/api/embedding_api/en-us/docs/embedding_api_auth.html"),
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
        ],
      ),
      customer_facing: {
        summary:
          "Tableau is rated partial for customer-facing delivery because Tableau's Embedding API and connected apps support embedded analytics in external applications with scoped JWT-based access, but the delivered surface is BI content rather than a full customer app runtime.",
        sources: [
          source("Tableau Embedding API authentication", "https://help.tableau.com/current/api/embedding_api/en-us/docs/embedding_api_auth.html"),
          source("Configure Tableau connected apps", "https://help.tableau.com/current/online/en-us/connected_apps_direct.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: note(
        "Tableau is rated partial for desktop because Tableau Desktop is a first-party authoring client, but it is not a runtime for packaging custom desktop applications.",
        [
          source("Tableau Desktop", "https://www.tableau.com/products/desktop"),
          source("Tableau Agent help", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
        ],
      ),
      mobile: {
        summary:
          "Tableau is rated partial for mobile because Tableau Mobile lets users browse and interact with Tableau content on iOS and Android, but it does not generate custom native mobile apps.",
        sources: [
          source("Explore content on Tableau Mobile", "https://help.tableau.com/current/mobile/mobile-user/en-us/tableau_mobile_explore.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Tableau is rated partial for offline because Tableau Mobile makes favorite views and workbooks available as offline previews, not as a full offline authoring or execution environment.",
        sources: [
          source("Explore content on Tableau Mobile", "https://help.tableau.com/current/mobile/mobile-user/en-us/tableau_mobile_explore.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: note(
        "Tableau is rated none for local-first architecture because published workbooks, governance, schedules, extracts, permissions, and sharing center on Tableau Server or Tableau Cloud.",
        [
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
        ],
      ),
      governance: {
        summary:
          "Tableau is rated enterprise for governance because Tableau documents site roles, projects, custom permissions, Active Directory/LDAP or SCIM group synchronization, certification/governance models, and activity-log permission auditing.",
        sources: [
          source("Governance in Tableau", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
          source("Audit permissions with Activity Log", "https://help.tableau.com/current/server-linux/en-us/activity_log_audit_permissions.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Tableau is rated partial for self-hosting because Tableau Server can be customer-run, while Tableau Cloud, Tableau Next, and some AI/advanced-management capabilities remain cloud/service dependent.",
        sources: [
          source("Install and configure Tableau Server", "https://help.tableau.com/current/server/en-us/install_config_top.htm"),
          source("Tableau Agent help", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Tableau is rated high lock-in because workbooks, dashboards, Prep flows, extracts, permissions, embedded analytics, and governance workflows are coupled to Tableau assets and server/cloud behavior.",
        [
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
          source("Tableau Embedding API", "https://help.tableau.com/current/api/embedding_api/en-us/index.html"),
        ],
      ),
      sandbox_isolation: note(
        "Tableau is rated partial for sandbox isolation because Tableau Cloud/Server provide platform execution boundaries for BI workloads, but Tableau is not a sandbox for arbitrary untrusted code or agent tools.",
        [
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
          source("Tableau Agent help", "https://help.tableau.com/current/pro/desktop/en-us/desktop_einstein.htm"),
        ],
      ),
      concurrent_state: note(
        "Tableau is rated partial for concurrent state because Server/Cloud coordinate shared content, schedules, permissions, extracts, and flow runs, while transactional business state belongs in source systems or external apps.",
        [
          source("Tableau Prep Conductor", "https://help.tableau.com/current/online/en-us/prep_conductor_online_intro.htm"),
          source("Tableau governance", "https://help.tableau.com/current/blueprint/en-us/bp_governance_in_tableau.htm"),
        ],
      ),
    },
  },
  "Power BI": {
    summary:
      "Power BI is Microsoft's analytics platform with semantic models, Desktop authoring, mobile apps, Premium/Fabric capacity, and Report Server for on-prem reporting. It is a BI/reporting stack, not a general app or workflow runtime.",
    sources: [
      source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
      source("Power BI Report Server", "https://learn.microsoft.com/en-us/power-bi/report-server/?view=powerbi-ps"),
      source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
      source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
      source("Power BI mobile offline", "https://learn.microsoft.com/en-us/power-bi/consumer/mobile/mobile-apps-offline-data"),
    ],
    cells: {
      ...biPlatformCells("Power BI", "semantic models, reports, dashboards, refresh, and Fabric/Power BI capacity", [
        source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
        source("Power BI Report Server", "https://learn.microsoft.com/en-us/power-bi/report-server/?view=powerbi-ps"),
        source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
        source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
        source("Power BI mobile offline", "https://learn.microsoft.com/en-us/power-bi/consumer/mobile/mobile-apps-offline-data"),
      ]),
      visual_workflow: note(
        "Power BI is rated partial for visual workflow because Desktop and the service provide visual report, dashboard, model, and data-prep authoring, but not a durable operational workflow canvas.",
        [
          source("What is Power BI Desktop", "https://learn.microsoft.com/en-us/power-bi/fundamentals/desktop-what-is-desktop"),
          source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
        ],
      ),
      replayable: note(
        "Power BI remains rated none for replayable execution because refreshes, deployment pipelines, and report activity are operational BI processes, not deterministic replay of workflow logic.",
        [
          source("Power BI admin auditing", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-auditing"),
          source("Power BI deployment pipelines", "https://learn.microsoft.com/en-us/power-bi/create-reports/deployment-pipelines-overview"),
        ],
      ),
      high_volume: note(
        "Power BI is rated partial for high-volume analytics because Premium/Fabric capacity and large semantic models support large BI datasets, while throughput depends on capacity, model mode, refresh, and connected data sources.",
        [
          source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
          source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
        ],
      ),
      compiled: note(
        "Power BI is rated none for compiled business logic because DAX, semantic models, reports, dashboards, and refresh definitions are BI artifacts rather than portable compiled workflow code.",
        [
          source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
          source("What is Power BI Desktop", "https://learn.microsoft.com/en-us/power-bi/fundamentals/desktop-what-is-desktop"),
        ],
      ),
      file_size: {
        summary:
          "Power BI is rated 10 GB for file/model headroom because Microsoft documents large semantic models up to 10 GB in Power BI Premium/Fabric capacity.",
        caveat:
          "The limit depends on capacity, semantic-model mode, and tenant settings; this is not a universal upload limit for every Power BI artifact.",
        sources: [
          source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Power BI is rated partial for file-native work because PBIX/PBIP/report files and imports are part of authoring, but governed state, refresh, sharing, and embeddings live in the service, Report Server, or Fabric capacity.",
        [
          source("What is Power BI Desktop", "https://learn.microsoft.com/en-us/power-bi/fundamentals/desktop-what-is-desktop"),
          source("Power BI Report Server", "https://learn.microsoft.com/en-us/power-bi/report-server/?view=powerbi-ps"),
        ],
      ),
      data_science: note(
        "Power BI is rated partial for data science because it provides semantic models, DAX, visuals, Copilot, and Fabric-adjacent analytics, but it is not itself a notebook or ML training runtime.",
        [
          source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
          source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
        ],
      ),
      ai_agents: {
        summary:
          "Power BI is rated partial for AI agents because Microsoft documents Copilot experiences for creating and consuming semantic models and reports, including chat, summaries, DAX/report assistance, and mobile Copilot, but not a general autonomous agent runtime.",
        caveat:
          "Copilot requires supported Microsoft Fabric/Power BI capacity and prepared semantic models; it augments analytics work rather than replacing a workflow agent platform.",
        sources: [
          source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
          source("Power BI Copilot tutorial", "https://learn.microsoft.com/power-bi/create-reports/tutorial-copilot-power-bi-introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "Power BI is rated native for analytics UI building because authors build reports, dashboards, semantic-model experiences, and embedded BI content, but this is not a general application UI builder.",
        [
          source("What is Power BI Desktop", "https://learn.microsoft.com/en-us/power-bi/fundamentals/desktop-what-is-desktop"),
          source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
        ],
      ),
      full_apps: note(
        "Power BI remains rated none for full apps because reports, dashboards, and embedded analytics require a separate application layer for operational workflows, auth, transactions, and app logic.",
        [
          source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
          source("Embed Power BI in a secure portal", "https://learn.microsoft.com/en-us/power-bi/collaborate-share/service-embed-secure"),
        ],
      ),
      customer_facing: {
        summary:
          "Power BI is rated partial for customer-facing delivery because Microsoft documents embedded analytics for customer-facing reports, dashboards, and tiles in your own apps, but the experience remains embedded BI rather than complete app delivery.",
        sources: [
          source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: note(
        "Power BI is rated partial for desktop because Power BI Desktop is a first-party authoring client, but it does not package custom desktop apps for end users.",
        [
          source("What is Power BI Desktop", "https://learn.microsoft.com/en-us/power-bi/fundamentals/desktop-what-is-desktop"),
        ],
      ),
      mobile: {
        summary:
          "Power BI is rated partial for mobile because Microsoft provides Power BI mobile apps for viewing and interacting with reports, not for generating custom mobile applications.",
        sources: [
          source("Power BI mobile apps", "https://learn.microsoft.com/en-us/power-bi/consumer/mobile/mobile-apps-for-mobile-devices"),
          source("Power BI mobile offline", "https://learn.microsoft.com/en-us/power-bi/consumer/mobile/mobile-apps-offline-data"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Power BI is now rated partial for offline because Microsoft documents offline viewing in the Power BI mobile apps for previously accessed dashboards and read-only reports, with cached data limits.",
        caveat:
          "This is not offline report authoring or general offline execution: DirectQuery/live reports are not cached, several tile/report types are unavailable, and the mobile apps cache up to 250 MB.",
        sources: [
          source("Power BI mobile offline", "https://learn.microsoft.com/en-us/power-bi/consumer/mobile/mobile-apps-offline-data"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: note(
        "Power BI is rated none for local-first architecture because governance, sharing, refresh, embedded analytics, Copilot, and tenant administration center on the Power BI service, Fabric capacity, or Report Server.",
        [
          source("Power BI admin portal", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-portal"),
          source("Power BI Report Server", "https://learn.microsoft.com/en-us/power-bi/report-server/?view=powerbi-ps"),
        ],
      ),
      governance: {
        summary:
          "Power BI is rated enterprise for governance because Microsoft documents tenant administration, audit activity logs, sensitivity labels, endorsements, deployment pipelines, and Fabric/Power BI capacity controls.",
        sources: [
          source("Power BI admin portal", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-portal"),
          source("Track user activities in Power BI", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-auditing"),
          source("Sensitivity labels in Power BI", "https://learn.microsoft.com/en-us/power-bi/enterprise/service-security-sensitivity-label-overview"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Power BI is now rated partial for self-hosted/on-prem because Power BI Report Server is an on-premises reporting portal, but it is a simpler reporting product and not feature-equivalent to the Power BI service.",
        sources: [
          source("Power BI Report Server", "https://learn.microsoft.com/en-us/power-bi/report-server/?view=powerbi-ps"),
          source("Power BI on-premises reporting scenario", "https://learn.microsoft.com/en-us/power-bi/guidance/powerbi-implementation-planning-usage-scenario-on-premises-reporting"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Power BI is rated high lock-in because semantic models, DAX, reports, dashboards, embedded tokens, tenant settings, sensitivity labels, and Copilot behavior are Power BI/Fabric platform artifacts.",
        [
          source("Power BI admin portal", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-portal"),
          source("Power BI embedded analytics", "https://learn.microsoft.com/en-us/power-bi/developer/embedded/embedded-analytics-power-bi"),
        ],
      ),
      sandbox_isolation: note(
        "Power BI is rated partial for sandbox isolation because Microsoft hosts and governs BI execution in Power BI/Fabric or Report Server boundaries, but it is not a sandbox for arbitrary untrusted agent tools.",
        [
          source("Power BI security whitepaper", "https://learn.microsoft.com/en-us/power-bi/guidance/whitepaper-powerbi-security"),
          source("Copilot in Power BI", "https://learn.microsoft.com/en-us/power-bi/create-reports/copilot-integration"),
        ],
      ),
      concurrent_state: note(
        "Power BI is rated partial for concurrent state because the platform coordinates shared reports, models, refreshes, permissions, and capacity, while transactional business state lives in source systems or external apps.",
        [
          source("Power BI admin portal", "https://learn.microsoft.com/en-us/power-bi/admin/service-admin-portal"),
          source("Power BI large semantic models", "https://learn.microsoft.com/en-za/power-bi/enterprise/service-premium-large-models"),
        ],
      ),
    },
  },
  Looker: {
    summary:
      "Looker centers on LookML semantic modeling, governed metrics, dashboards, embedded analytics, and Google Cloud integrations. Its developer model is analytics-first, not operational workflow or full app delivery.",
    sources: [
      source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
      source("Looker SQL interface", "https://docs.cloud.google.com/looker/docs/sql-interface"),
      source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
      source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
      source("Looker mobile app", "https://docs.cloud.google.com/looker/docs/looker-core-mobile-app"),
      source("Customer-hosted Looker", "https://docs.cloud.google.com/looker/docs/customer-hosted-instances"),
    ],
    cells: {
      ...biPlatformCells("Looker", "LookML semantic models, dashboards, governed metrics, and embedded analytics", [
        source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
        source("Looker SQL interface", "https://docs.cloud.google.com/looker/docs/sql-interface"),
        source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
      ]),
      visual_workflow: note(
        "Looker is rated partial for visual workflow because users build dashboards, Explores, and governed analytics experiences, while the core modeling layer is LookML and not an operational workflow canvas.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        ],
      ),
      replayable: note(
        "Looker remains rated none for replayable execution because schedules, queries, and dashboard activity are analytics operations, not deterministic replay of business workflows.",
        [
          source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
        ],
      ),
      high_volume: note(
        "Looker is rated partial for high-volume analytics because it pushes governed analytics to connected warehouses and SQL interfaces, while scale depends on the warehouse, model design, cache, schedules, and instance configuration.",
        [
          source("Looker SQL interface", "https://docs.cloud.google.com/looker/docs/sql-interface"),
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
        ],
      ),
      compiled: note(
        "Looker is rated none for compiled business logic because LookML models, dashboards, Explores, and embeds are analytics-layer assets rather than portable compiled workflow code.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
        ],
      ),
      ai_agents: {
        summary:
          "Looker is rated partial for AI agents because Google documents Gemini-assisted analytics experiences around Looker, but this is BI assistance rather than a general autonomous agent runtime.",
        sources: [
          source("Gemini in Looker overview", "https://docs.cloud.google.com/looker/docs/gemini-in-looker-overview"),
          source("Looker Conversational Analytics", "https://docs.cloud.google.com/looker/docs/conversational-analytics"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "Looker is rated native for analytics UI building because users create dashboards, Looks, Explores, and embedded BI content, but not general operational app screens.",
        [
          source("Looker embed overview", "https://docs.cloud.google.com/looker/docs/embed-overview"),
          source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        ],
      ),
      full_apps: note(
        "Looker remains rated none for full apps because embedded dashboards and governed metrics require a separate application layer for transactions, workflows, auth, and broader app UX.",
        [
          source("Looker embed overview", "https://docs.cloud.google.com/looker/docs/embed-overview"),
          source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        ],
      ),
      customer_facing: {
        summary:
          "Looker is rated partial for customer-facing delivery because Looker's embed solution and signed embedding support dashboards, Explores, Looks, and query visualizations inside external apps, while the delivered experience is embedded BI content rather than a full app runtime.",
        sources: [
          source("Looker embed overview", "https://docs.cloud.google.com/looker/docs/embed-overview"),
          source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: note(
        "Looker is rated none for desktop app delivery because Looker is delivered through browser, mobile, embed, and hosted/customer-hosted analytics experiences rather than native desktop app packaging.",
        [
          source("Customer-hosted Looker", "https://docs.cloud.google.com/looker/docs/customer-hosted-instances"),
          source("Looker mobile app", "https://docs.cloud.google.com/looker/docs/looker-core-mobile-app"),
        ],
      ),
      mobile: {
        summary:
          "Looker is rated partial for mobile because the Looker mobile app lets users view Looks, dashboards, boards, folders, and shared content on mobile devices, but it is not a custom mobile-app builder.",
        sources: [
          source("Looker mobile app", "https://docs.cloud.google.com/looker/docs/looker-core-mobile-app"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Looker remains rated none for offline because current mobile docs describe authenticated mobile access and content browsing, but not offline cached viewing comparable to Power BI or Tableau Mobile.",
        sources: [
          source("Looker mobile app", "https://docs.cloud.google.com/looker/docs/looker-core-mobile-app"),
          source("Enable Looker mobile app", "https://docs.cloud.google.com/looker/docs/mobile-app-enablement"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: note(
        "Looker is rated none for local-first architecture because governed models, dashboards, permissions, schedules, embeds, audit logs, and data agents are centered on the Looker instance.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
        ],
      ),
      governance: {
        summary:
          "Looker is rated enterprise for governance because Google documents LookML governed semantic modeling, roles and permissions, Cloud Audit Logs for Looker (Google Cloud core), and customer-hosted or Google-managed deployment options.",
        sources: [
          source("Prepare a Looker instance for users", "https://docs.cloud.google.com/looker/docs/looker-core-instance-setup"),
          source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Looker is rated high lock-in because LookML projects, dashboards, Explores, permissions, embed URLs, audit logs, and Gemini/agent features are coupled to Looker platform semantics.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker signed embedding", "https://docs.cloud.google.com/looker/docs/signed-embedding"),
        ],
      ),
      sandbox_isolation: note(
        "Looker is rated partial for sandbox isolation because Looker instances and Google Cloud controls isolate BI workloads, but Looker is not a sandbox for arbitrary untrusted code or agent tool execution.",
        [
          source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
          source("Gemini in Looker overview", "https://docs.cloud.google.com/looker/docs/gemini-in-looker-overview"),
        ],
      ),
      concurrent_state: note(
        "Looker is rated partial for concurrent state because it coordinates shared semantic models, dashboards, permissions, audit logs, schedules, and embeds, while transactional business state remains in external systems.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker audit logging", "https://docs.cloud.google.com/looker/docs/looker-core-audit-logging"),
        ],
      ),
      self_hosted: {
        summary:
          "Looker is now rated partial for self-hosting because Google documents customer-hosted Looker instances, but that deployment model still depends on Looker's platform and support model.",
        caveat:
          "This is not an open-source self-host package; it is a customer-managed Looker deployment.",
        sources: [
          source("Customer-hosted Looker", "https://docs.cloud.google.com/looker/docs/customer-hosted-instances"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Looker remains rated depends because Looker usually queries databases and governed semantic models rather than accepting a single universal workflow file size.",
        caveat:
          "Limits depend on the connected warehouse, PDT/export paths, API endpoint, schedule format, and embedded content type.",
        sources: [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker SQL interface", "https://docs.cloud.google.com/looker/docs/sql-interface"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Looker is rated none for file-native workflows because LookML and dashboards model warehouse-backed analytics; file handling is not a first-class local project data layer.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Looker SQL interface", "https://docs.cloud.google.com/looker/docs/sql-interface"),
        ],
      ),
      data_science: note(
        "Looker is rated partial for data science because it provides governed semantic modeling, SQL access, dashboards, and Gemini-assisted analytics, but model training and notebooks live in adjacent data platforms.",
        [
          source("LookML introduction", "https://docs.cloud.google.com/looker/docs/what-is-lookml"),
          source("Gemini in Looker overview", "https://docs.cloud.google.com/looker/docs/gemini-in-looker-overview"),
        ],
      ),
    },
  },
  Airflow: {
    summary:
      "Apache Airflow is a Python DAG orchestration platform for scheduled data and infrastructure workflows with pluggable executors. It is strong for replayable data pipelines, but intentionally developer-first and not an end-user app builder.",
    sources: [
      source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
      source("Airflow backfill", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/backfill.html"),
      source("Airflow auth manager", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/auth-manager/index.html"),
      source("Airflow docs", "https://airflow.apache.org/docs/"),
    ],
    cells: {
      ...orchestrationCells("Airflow", "DAGs, tasks, schedules, executors, and data pipeline orchestration", [
        source("Airflow DAGs", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/dags.html"),
        source("Airflow backfill", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/backfill.html"),
        source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
      ]),
      visual_workflow: {
        summary:
          "Airflow remains rated none for visual workflow building because DAGs are authored in Python code; the web UI is for monitoring and operations, not drawing executable workflows.",
        sources: [
          source("Airflow DAGs", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/dags.html"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Airflow is rated native for replayable execution because it documents DAG runs, task retries, clearing/rerunning task instances, and backfills over historical data intervals.",
        caveat:
          "This is scheduler-level rerun/backfill support, not deterministic event-history replay like Temporal.",
        sources: [
          source("Airflow backfill", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/backfill.html"),
          source("Airflow tasks", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/tasks.html"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Airflow is rated native for high-volume orchestration because it documents multiple executors and distributed execution patterns for running many tasks across worker infrastructure.",
        caveat:
          "Airflow orchestrates high-volume work; the heavy data processing usually happens in external systems such as Spark, warehouses, or Kubernetes jobs.",
        sources: [
          source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Airflow is rated native for compiled/code-defined business logic because DAGs and tasks are authored as Python code, versioned like software, and executed through configured executors rather than mutable no-code recipes.",
        caveat:
          "Python itself is interpreted, but this cell distinguishes code-defined deployable logic from hosted visual configuration.",
        sources: [
          source("Airflow DAGs", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/dags.html"),
          source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Airflow is rated depends for file and payload limits because DAGs normally pass metadata, XComs, object-storage references, or external-system identifiers rather than owning one product-level upload limit.",
        caveat:
          "Large data should live in warehouses, object stores, queues, or task-local storage; executor, XCom backend, API, and infrastructure settings determine practical limits.",
        sources: [
          source("Airflow object storage", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/objectstorage.html"),
          source("Airflow XComs", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/xcoms.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Airflow remains rated none for built-in AI agents because official docs describe DAG scheduling, tasks, operators, sensors, executors, and backfills, not first-party agent memory, planning, or tool-use loops.",
        sources: [
          source("Airflow DAGs", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/dags.html"),
          source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Airflow is rated none for UI building because the web UI and DAG views are operational consoles for monitoring and managing runs, not app-screen builders for end users.",
        sources: [
          source("Airflow UI overview", "https://airflow.apache.org/docs/apache-airflow/stable/ui.html"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Airflow is rated basic for governance because official docs cover auth managers and authorization hooks for users, roles, groups, DAGs, and API access, but not a full enterprise business governance plane.",
        sources: [
          source("Airflow auth manager", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/auth-manager/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Airflow is rated native for self-hosting because Apache Airflow is an installable open-source service with official installation and production deployment documentation.",
        sources: [
          source("Airflow installation", "https://airflow.apache.org/docs/apache-airflow/stable/installation/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Airflow is rated partial for sandbox isolation because executor choice can run tasks in separate processes, workers, containers, or pods, but isolation is controlled by the deployment and executor rather than a universal Airflow sandbox.",
        sources: [
          source("Airflow executors", "https://airflow.apache.org/docs/apache-airflow/stable/core-concepts/executor/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Airflow is rated partial for concurrent workflow state because DAG runs, task instances, schedules, and backfill concurrency are coordinated through Airflow's scheduler and metadata database, but arbitrary task-side business state is not transactional.",
        caveat:
          "Tasks still need their own idempotency and external-state handling when they update databases, files, APIs, or warehouses.",
        sources: [
          source("Airflow database ERD", "https://airflow.apache.org/docs/apache-airflow/stable/database-erd-ref.html"),
          source("Airflow scheduler", "https://airflow.apache.org/docs/apache-airflow/stable/administration-and-deployment/scheduler.html"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Temporal: {
    summary:
      "Temporal provides durable execution for code-defined workflows with replay and state reconstruction from event history. It is excellent infrastructure for reliable services, but not a visual business app builder.",
    sources: [
      source("Temporal docs", "https://docs.temporal.io/"),
      source("Temporal Cloud limits", "https://docs.temporal.io/cloud/limits"),
      source("Temporal Cloud roles and permissions", "https://docs.temporal.io/cloud/manage-access/roles-and-permissions"),
      source("Temporal Cloud audit logs", "https://docs.temporal.io/cloud/audit-logs"),
      source("Temporal production deployments", "https://docs.temporal.io/production-deployment"),
    ],
    cells: {
      ...orchestrationCells("Temporal", "durable workflows, activities, workers, and event-history-backed service orchestration", [
        source("Temporal docs", "https://docs.temporal.io/"),
        source("Temporal workflows", "https://docs.temporal.io/workflows"),
        source("Temporal Cloud limits", "https://docs.temporal.io/cloud/limits"),
      ]),
      visual_workflow: {
        summary:
          "Temporal is rated none for visual workflow building because Temporal workflows are written in SDK code and the UI is for visibility, operations, and debugging rather than drawing executable business workflows.",
        sources: [
          source("Temporal workflows", "https://docs.temporal.io/workflows"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Temporal is rated native for replayable execution because workflows reconstruct state by replaying event history and requiring deterministic workflow code.",
        sources: [
          source("Temporal event history", "https://docs.temporal.io/workflows#event-history"),
          source("Temporal deterministic constraints", "https://docs.temporal.io/workflows#deterministic-constraints"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Temporal is rated native for high-volume orchestration because Temporal Cloud documents namespace, workflow, signal, update, and event-history limits, and production docs separate the service from horizontally scalable workers.",
        caveat:
          "Throughput still depends on worker fleet sizing, task queues, history size, namespace limits, downstream systems, and whether the deployment is Temporal Cloud or self-hosted.",
        sources: [
          source("Temporal Cloud limits", "https://docs.temporal.io/cloud/limits"),
          source("Temporal production deployments", "https://docs.temporal.io/production-deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Temporal is rated native for compiled/code-defined business logic because workflow behavior is authored in SDK application code and deployed through workers, with determinism enforced by workflow replay.",
        sources: [
          source("Temporal workflows", "https://docs.temporal.io/workflows"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Temporal is rated 2 MB because Temporal Cloud limits a single payload/blob to 2 MB, and Temporal recommends passing large data by reference instead of through workflow history.",
        caveat:
          "This is an orchestration-history payload limit, not a file-storage feature; large files should live in external object storage.",
        sources: [
          source("Temporal Cloud limits", "https://docs.temporal.io/cloud/limits"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Temporal remains rated none for built-in AI agents because docs describe durable workflow and activity execution, not a first-party autonomous agent framework with model loops, tools, memory, or planning.",
        caveat:
          "Developers can build agent services on Temporal, but that is application code on top of the workflow runtime.",
        sources: [
          source("Temporal workflows", "https://docs.temporal.io/workflows"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Temporal is rated none for UI building because Temporal Web/Cloud UI surfaces workflow operations and visibility, not custom forms, dashboards, or app screens.",
        sources: [
          source("Temporal docs", "https://docs.temporal.io/"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Temporal is rated native for data and long-running compute orchestration because workflows coordinate durable jobs, retries, activities, task queues, and external compute; the actual analytics or ML processing runs in worker code and external systems.",
        sources: [
          source("Temporal workflows", "https://docs.temporal.io/workflows"),
          source("Temporal production deployments", "https://docs.temporal.io/production-deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Temporal is now rated enterprise for governance because Temporal Cloud documents account and namespace RBAC, service accounts, API keys, SAML/SCIM identity integration, permission references, and audit logs.",
        caveat:
          "These controls are Temporal Cloud governance features; self-hosted Temporal deployments must design and operate their own surrounding IAM and audit architecture.",
        sources: [
          source("Temporal Cloud roles and permissions", "https://docs.temporal.io/cloud/manage-access/roles-and-permissions"),
          source("Temporal Cloud audit logs", "https://docs.temporal.io/cloud/audit-logs"),
          source("Temporal Cloud SAML", "https://docs.temporal.io/cloud/saml"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Temporal is rated native for self-hosting because official production docs describe running a self-hosted Temporal Service and deploying workers wherever the customer controls the runtime.",
        sources: [
          source("Temporal production deployments", "https://docs.temporal.io/production-deployment"),
          source("Temporal self-hosted guide", "https://docs.temporal.io/self-hosted-guide"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Temporal is rated partial for sandbox isolation because workflow and activity code runs inside customer-controlled worker processes or containers, so isolation depends on worker deployment choices rather than a built-in untrusted-code sandbox.",
        sources: [
          source("Temporal production deployments", "https://docs.temporal.io/production-deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Temporal is rated native for concurrent workflow state because each workflow execution has durable state and serialized event history managed by the Temporal service.",
        caveat:
          "External data stores still need their own concurrency controls; Temporal protects workflow execution state, not every downstream database write.",
        sources: [
          source("Temporal workflows", "https://docs.temporal.io/workflows"),
          source("Temporal event history", "https://docs.temporal.io/workflows#event-history"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Ontology data platform": {
    summary:
      "This row represents ontology-driven enterprise data platforms such as Palantir Foundry/AIP. They combine governed data models, operational applications, AI, and security, but usually require deep platform adoption.",
    sources: [
      source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
      source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
      source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
      source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
    ],
    cells: {
      ...enterpriseDataPlatformCells("Ontology data platforms", "ontology-backed operational data, application building, workflow management, and AIP agents", [
        source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
        source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
      ]),
      visual_workflow: {
        summary:
          "Ontology data platforms are rated partial for visual workflow building because Palantir-style platforms expose operational app and workflow-building surfaces on top of ontology data, but those workflows remain tied to the ontology/runtime model.",
        sources: [
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "Ontology data platforms are now rated partial for replayability because Palantir AIP documents logging, tracing, action histories, and evals for agent/workflow behavior, but not deterministic replay of portable workflow code.",
        [
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
          source("Palantir AIP features", "https://www.palantir.com/docs/foundry/aip/aip-features"),
        ],
      ),
      high_volume: note(
        "Ontology data platforms are rated native for high-volume enterprise data because Foundry/AIP materials describe governed data operations, billion-object ontology queries, tens-of-thousands of actions, and operational application scale.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
      ),
      compiled: note(
        "Ontology data platforms are rated none for compiled business logic because applications, actions, agents, and ontology-backed workflows are platform artifacts rather than portable compiled workflow code.",
        [
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
          source("Palantir AIP features", "https://www.palantir.com/docs/foundry/aip/aip-features"),
        ],
      ),
      file_size: {
        summary:
          "Ontology data platforms are now rated depends for file/payload limits because large-data ingestion is a core platform use case, but public docs do not expose a single universal attachment or workflow payload cap.",
        caveat:
          "This row represents a category rather than one SKU, so limits depend on the deployment, dataset type, API path, and application surface.",
        sources: [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Ontology data platforms are rated none for file-native workflows because files, documents, datasets, ontology objects, and lineage are governed platform assets rather than local project files.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        ],
      ),
      ai_agents: {
        summary:
          "Ontology data platforms are now rated native for AI agents when represented by Palantir AIP, because AIP Agent Studio documents configurable agents grounded in ontology actions and data.",
        caveat:
          "This reflects Palantir-style ontology platforms; other ontology/data platforms may expose weaker agent tooling.",
        sources: [
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: note(
        "Ontology data platforms are rated partial for full apps because Palantir documents Workshop and OSDK application building, but those apps remain coupled to the ontology, permissions, APIs, and platform runtime.",
        [
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        ],
      ),
      customer_facing: note(
        "Ontology data platforms remain rated none for general customer-facing app delivery because the public materials emphasize enterprise internal operations and platform/API deployment rather than a broad external customer-app runtime.",
        [
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        ],
      ),
      desktop: note(
        "Ontology data platforms are rated none for desktop app packaging because Foundry/AIP docs focus on web/platform, SDK, API, mobile, and edge application patterns rather than generated native desktop apps.",
        [
          source("Palantir AIP overview", "https://www.palantir.com/docs/foundry/aip/overview/"),
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
        ],
      ),
      mobile: note(
        "Ontology data platforms are rated partial for mobile because Palantir AIP materials describe mobile AI application scenarios, but not a general native mobile app builder comparable to dedicated app platforms.",
        [
          source("Palantir AIP overview", "https://www.palantir.com/docs/foundry/aip/overview/"),
        ],
      ),
      offline: note(
        "Ontology data platforms are rated none for offline execution because ontology data, permissions, action logs, agents, and application state are governed by the platform service.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
      ),
      local_first: note(
        "Ontology data platforms are rated none for local-first architecture because the canonical ontology, lineage, permissions, actions, and workflow state live in the vendor platform.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
      ),
      ui_builder: {
        summary:
          "Ontology data platforms are rated native for UI building because Palantir documents application-building tools for operational apps on governed ontology data.",
        caveat:
          "The UI is tightly coupled to the platform's ontology, permissions, and deployment model.",
        sources: [
          source("Palantir application building", "https://www.palantir.com/docs/foundry/app-building/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: note(
        "Ontology data platforms are rated enterprise for governance because AIP/Foundry materials emphasize granular permissions, action governance, audit logs, security models, data lineage, and monitored AI workflows.",
        [
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        ],
      ),
      self_hosted: note(
        "Ontology data platforms are rated partial for self-hosting because Palantir-style deployments can involve customer-controlled environments and Apollo-managed delivery, but the platform remains proprietary and not an open runtime.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP overview", "https://www.palantir.com/docs/foundry/aip/overview/"),
        ],
      ),
      data_science: {
        summary:
          "Ontology data platforms are rated native for analytics and ML workflows because Palantir Foundry/AIP center on governed data integration, ontology-backed operational models, AI agents, and application-building on enterprise data.",
        sources: [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: note(
        "Ontology data platforms are rated partial for sandbox isolation because AIP materials describe governed, autoscaling, sandboxed application integration, but not a portable sandbox for arbitrary untrusted tools.",
        [
          source("Palantir AIP overview", "https://www.palantir.com/docs/foundry/aip/overview/"),
          source("Palantir AIP Agent Studio", "https://www.palantir.com/docs/foundry/agent-studio/overview/"),
        ],
      ),
      concurrent_state: note(
        "Ontology data platforms are rated native for concurrent state because ontology objects, actions, permissions, applications, and AI agents coordinate shared operational data in the platform model.",
        [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
      ),
      lock_in: {
        summary:
          "Ontology data platforms are rated high lock-in because applications, access controls, actions, and AI agents depend on the vendor ontology and runtime model.",
        sources: [
          source("Palantir Foundry overview", "https://www.palantir.com/docs/foundry/platform-overview"),
          source("Palantir AIP architecture", "https://www.palantir.com/docs/foundry/architecture-center/aip-architecture"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "ERP process platform": {
    summary:
      "This row represents ERP-adjacent process platforms such as SAP Build and SAP BTP. They combine low-code apps, process automation, forms, workflows, RPA, and SAP integration, but are strongest inside their vendor ecosystem.",
    sources: [
      source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
      source("SAP Build Apps", "https://help.sap.com/docs/BUILD_APPS"),
      source("SAP Build Process Automation limits", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/limits"),
      source("SAP Joule in SAP Build", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/joule-in-sap-build"),
    ],
    cells: {
      ...enterpriseDataPlatformCells("ERP process platforms", "ERP-adjacent low-code apps, process automation, forms, workflow visibility, and desktop automation", [
        source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        source("SAP Build Process Automation limits", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/limits"),
        source("SAP Build desktop agent", "https://help.sap.com/docs/SAP_BUILD_PROCESS_AUTOMATION_DESKTOP_AGENT/eae9c11a17a14cfc93e08b22e8574305/16a8e1e7d4ab47b6a961fb8705c27bcb.html"),
        source("SAP Joule in SAP Build", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/joule-in-sap-build"),
      ]),
      file_size: {
        summary:
          "ERP process platforms are now rated 50 MB for this row because SAP Build Process Automation documents a 50 MB maximum file size for the upload file action, while some forms and task attachments are smaller.",
        caveat:
          "This category covers multiple SAP/ERP-adjacent surfaces, so exact limits vary by process automation action, form, attachment, and backend service.",
        sources: [
          source("SAP Build Process Automation limits", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/limits"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: {
        summary:
          "ERP process platforms are rated partial for visual workflow building because SAP Build Process Automation provides process and automation modeling, but the workflows are still tied to SAP/BTP process semantics.",
        sources: [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "ERP process platforms are rated partial for replayability because SAP Build Process Automation models processes and executes workflows with runtime visibility, but public docs do not describe deterministic replay of completed workflow code.",
        [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
        ],
      ),
      high_volume: note(
        "ERP process platforms are rated partial for high-volume execution because SAP Build Process Automation documents request-rate quotas and process/form/workflow capabilities, while throughput is governed by BTP tenant limits and connected SAP systems.",
        [
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        ],
      ),
      compiled: note(
        "ERP process platforms are rated none for compiled business logic because processes, forms, business rules, automations, and apps are SAP Build/BTP artifacts rather than portable compiled workflow code.",
        [
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        ],
      ),
      ai_agents: {
        summary:
          "ERP process platforms remain partial for AI agents because SAP documents Joule assistance in SAP Build, but public docs frame it as AI-assisted building and process work rather than a general portable agent runtime.",
        sources: [
          source("SAP Joule in SAP Build", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/joule-in-sap-build"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "ERP process platforms are rated native for UI building because SAP Build Apps is documented for drag-and-drop enterprise web and mobile app development, alongside process forms and task experiences.",
        [
          source("SAP Build Apps learning", "https://learning.sap.com/products/sap-build/build-apps"),
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
        ],
      ),
      full_apps: note(
        "ERP process platforms are rated native for full apps because SAP Build Apps supports no-code web/mobile enterprise application development, but the strongest fit remains SAP/BTP-centered business applications.",
        [
          source("SAP Build Apps learning", "https://learning.sap.com/products/sap-build/build-apps"),
          source("SAP Build", "https://www.sap.com/products/technology-platform/build.html"),
        ],
      ),
      customer_facing: note(
        "ERP process platforms are rated partial for customer-facing delivery because SAP Build Apps can create web/mobile apps, while SAP process automation tasks and forms are primarily enterprise process surfaces tied to SAP/BTP identity and data.",
        [
          source("SAP Build Apps learning", "https://learning.sap.com/products/sap-build/build-apps"),
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
        ],
      ),
      desktop: note(
        "ERP process platforms are rated none for desktop app packaging because SAP Build desktop-agent docs concern desktop automation execution, not generated native desktop applications.",
        [
          source("SAP Build desktop agent", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-desktop-agent-2"),
        ],
      ),
      mobile: note(
        "ERP process platforms are rated partial for mobile because SAP Build Apps targets web and mobile apps and SAP task experiences can surface through SAP Mobile Start, but this remains SAP/BTP app delivery.",
        [
          source("SAP Build Apps learning", "https://learning.sap.com/products/sap-build/build-apps"),
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
        ],
      ),
      offline: note(
        "ERP process platforms are rated none for offline execution because SAP Build Process Automation, Joule, task centers, forms, and process visibility depend on SAP BTP services and connected systems.",
        [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
        ],
      ),
      local_first: note(
        "ERP process platforms are rated none for local-first architecture because canonical processes, forms, tasks, bots, business rules, and application data are centered on SAP BTP and connected SAP systems.",
        [
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        ],
      ),
      data_science: {
        summary:
          "ERP process platforms are rated partial for analytics and ML workflows because SAP/BTP process automation can connect ERP data and use Joule assistance, but the row is centered on process automation rather than notebooks, model training, or analytical storage.",
        sources: [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Joule in SAP Build", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/joule-in-sap-build"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: note(
        "ERP process platforms are rated enterprise for governance because SAP Build/BTP process automation uses tenant services, roles, quotas, task-center experiences, and SAP ecosystem controls for enterprise process work.",
        [
          source("What is SAP Build?", "https://help.sap.com/docs/build/sap-build-core/create-abap-cloud-project?locale=en-US"),
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
        ],
      ),
      self_hosted: {
        summary:
          "ERP process platforms remain partial for self-hosting because SAP Build Process Automation is a BTP cloud service, while it can still integrate with on-premise SAP and desktop-agent environments.",
        caveat:
          "The process platform itself is not a simple customer-run open runtime.",
        sources: [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Build desktop agent", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-desktop-agent-2"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "ERP process platforms are rated high lock-in because apps, processes, forms, business rules, bots, Joule integrations, task-center surfaces, and SAP connectors are SAP Build/BTP artifacts.",
        [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Build", "https://www.sap.com/products/technology-platform/build.html"),
        ],
      ),
      sandbox_isolation: note(
        "ERP process platforms are rated partial for sandbox isolation because SAP BTP and desktop-agent execution provide platform boundaries, but SAP Build is not a portable sandbox for arbitrary untrusted tools.",
        [
          source("SAP Build desktop agent", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-desktop-agent-2"),
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
        ],
      ),
      concurrent_state: note(
        "ERP process platforms are rated native for concurrent process state because SAP Build Process Automation runs process instances, tasks, workflows, rules, and visibility on a managed process runtime.",
        [
          source("SAP Build Process Automation", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/about-sap-build-process-automation"),
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
        ],
      ),
      file_native: note(
        "ERP process platforms are rated none for file-native workflows because uploaded documents, forms, task attachments, and automation files are process artifacts, not local project files owned by an offline-first runtime.",
        [
          source("SAP Build Process Automation limits", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/limits"),
          source("SAP Build Process Automation quotas", "https://help.sap.com/docs/build-process-automation/sap-build-process-automation/quotas"),
        ],
      ),
    },
  },
  ServiceNow: {
    summary:
      "ServiceNow combines App Engine, Flow Designer, IntegrationHub, mobile apps, workspaces, and AI Platform capabilities for enterprise service workflows. It is broad and governed, but bound to the ServiceNow data and workflow model.",
    sources: [
      source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/washingtondc/build-workflows/build-workflows.html"),
      source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
      source("ServiceNow Mobile Platform", "https://www.servicenow.com/docs/r/mobile/mobile-config-navigation.html"),
      source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
    ],
    cells: {
      ...enterpriseWorkflowPlatformCells("ServiceNow", "Flow Designer, IntegrationHub, App Engine, workspaces, and Now Platform service workflows", [
        source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/washingtondc/build-workflows/build-workflows.html"),
        source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
        source("ServiceNow Mobile Platform", "https://www.servicenow.com/docs/r/mobile/mobile-config-navigation.html"),
        source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
      ]),
      file_size: {
        summary:
          "ServiceNow is now rated 1024 MB because the current ServiceNow attachment docs state the default maximum attachment size is 1024 MB for a new base-system instance.",
        caveat:
          "Admins can change the maximum attachment size, and individual portals/widgets/processes can still impose stricter limits.",
        sources: [
          source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
          source("ServiceNow attachment administration", "https://www.servicenow.com/docs/r/platform-administration/r_AdministeringAttachments.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "ServiceNow is now rated native for AI agents because ServiceNow documents AI Agent Studio, AI Agent Orchestrator, and generally available AI agents across the Now Platform.",
        caveat:
          "These agents are native to the ServiceNow platform and its enterprise workflows, not a portable standalone agent framework.",
        sources: [
          source("ServiceNow Yokohama AI agents", "https://newsroom.servicenow.com/press-releases/details/2025/ServiceNows-latest-platform-release-adds-to-thousands-of-AI-agents-across-CRM-HR-IT-and-more-for-faster-smarter-workflows-and-maximum-business-impact-03-12-2025-traffic/default.aspx"),
          source("ServiceNow AI", "https://www.servicenow.com/now-platform/now-intelligence.html"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: note(
        "ServiceNow is rated native for visual workflow building because Flow Designer, App Engine Studio, and App Engine are first-party low-code surfaces for building workflows and workflow apps on the Now Platform.",
        [
          source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/build-workflows/build-workflows.html"),
          source("ServiceNow App Engine Studio", "https://www.servicenow.com/au/products/app-engine-studio.html"),
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
        ],
      ),
      replayable: note(
        "ServiceNow remains rated partial for replayability because platform workflows expose operational histories, approvals, tasks, and remediation state, but public docs do not describe deterministic event-history replay of portable workflow code.",
        [
          source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/build-workflows/build-workflows.html"),
          source("ServiceNow Customer Service Management", "https://www.servicenow.com/products/customer-service-management.html"),
        ],
      ),
      high_volume: note(
        "ServiceNow is rated partial for high-volume execution because it targets enterprise workflow scale and now markets RaptorDB/process-mining performance for the AI Platform, while tenant limits, licensing, integrations, and instance sizing still matter.",
        [
          source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
        ],
      ),
      compiled: note(
        "ServiceNow remains rated none for compiled business logic because workflows, actions, scripts, tables, and app definitions are Now Platform metadata/runtime artifacts rather than portable compiled workflow code.",
        [
          source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/build-workflows/build-workflows.html"),
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
        ],
      ),
      file_native: note(
        "ServiceNow remains rated none for file-native workflows because files are platform attachments or records, not local project artifacts managed by an offline-first runtime.",
        [
          source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
        ],
      ),
      data_science: note(
        "ServiceNow is rated partial for analytics and ML workflows because the AI Platform includes analytics, process mining, RaptorDB, and workflow intelligence, but it is not primarily a notebook, ML-training, or data-science pipeline platform.",
        [
          source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
          source("ServiceNow Customer Service Management", "https://www.servicenow.com/products/customer-service-management.html"),
        ],
      ),
      ui_builder: note(
        "ServiceNow is rated native for end-user UI building because App Engine, UI Builder, workspaces, service portals, and mobile experiences are first-party Now Platform app surfaces.",
        [
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
          source("ServiceNow Customer Service Management", "https://www.servicenow.com/products/customer-service-management.html"),
          source("ServiceNow Native Mobile", "https://horizon.servicenow.com/native-mobile/overview"),
        ],
      ),
      full_apps: note(
        "ServiceNow is rated native for full apps because App Engine is documented for creating governed business workflow apps on the Now Platform, but those apps stay coupled to ServiceNow data, identity, and runtime services.",
        [
          source("ServiceNow App Engine Studio", "https://www.servicenow.com/au/products/app-engine-studio.html"),
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
        ],
      ),
      customer_facing: note(
        "ServiceNow is rated partial for customer-facing delivery because Customer Service Management and portals support customer self-service and service workflows, but ServiceNow is not a general-purpose consumer app distribution runtime.",
        [
          source("ServiceNow Customer Service Management", "https://www.servicenow.com/products/customer-service-management.html"),
        ],
      ),
      desktop: note(
        "ServiceNow remains rated none for desktop application packaging because the documented app surfaces are web, workspace, portal, and mobile experiences rather than arbitrary native desktop apps.",
        [
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
          source("ServiceNow Native Mobile", "https://horizon.servicenow.com/native-mobile/overview"),
        ],
      ),
      mobile: note(
        "ServiceNow is rated partial for mobile because Now Mobile and native mobile design resources support ServiceNow workflow experiences on mobile, but they do not make ServiceNow a custom mobile app builder for arbitrary apps.",
        [
          source("Now Mobile", "https://play.google.com/store/apps/details?id=com.servicenow.requestor"),
          source("ServiceNow Native Mobile", "https://horizon.servicenow.com/native-mobile/overview"),
        ],
      ),
      offline: note(
        "ServiceNow remains rated none for offline execution because Now Platform app and mobile materials emphasize connected instance access and mobile workflow experiences, not offline-first app execution with sync.",
        [
          source("Now Mobile", "https://play.google.com/store/apps/details?id=com.servicenow.requestor"),
          source("ServiceNow Native Mobile", "https://horizon.servicenow.com/native-mobile/overview"),
        ],
      ),
      local_first: note(
        "ServiceNow remains rated none for local-first architecture because records, workflows, identity, analytics, and app definitions live in the Now Platform instance rather than on user devices.",
        [
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
          source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
        ],
      ),
      governance: note(
        "ServiceNow is rated enterprise for governance because the Now Platform centers on enterprise identity, roles, workflow controls, audit/compliance products, and governed AI/workflow operations.",
        [
          source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
        ],
      ),
      self_hosted: note(
        "ServiceNow is rated partial for self-hosting because ServiceNow publishes a self-hosted software guide and MID Server guidance, but the normal Now Platform model remains ServiceNow-hosted or tightly vendor-controlled.",
        [
          source("ServiceNow self-hosted software guide", "https://www.servicenow.com/content/dam/servicenow-assets/public/en-us/doc-type/legal/self-hosted-software-guide-jan2024.pdf"),
          source("ServiceNow MID Server connectivity", "https://www.servicenow.com/docs/en-US/bundle/zurich-servicenow-platform/page/product/mid-server/concept/c_MIDServerConnectionPrerequisites.html"),
        ],
      ),
      lock_in: note(
        "ServiceNow is rated high lock-in because apps, records, service workflows, agents, integrations, and governance depend on Now Platform metadata and runtime services.",
        [
          source("ServiceNow application development", "https://www.servicenow.com/uk/products/application-development.html"),
          source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
        ],
      ),
      sandbox_isolation: note(
        "ServiceNow is rated partial for sandbox isolation because the hosted platform separates tenant execution and roles, but public docs do not expose a portable sandbox for arbitrary untrusted agent tools.",
        [
          source("Security on the ServiceNow AI Platform", "https://www.servicenow.com/company/trust/security.html"),
          source("ServiceNow MID Server connectivity", "https://www.servicenow.com/docs/en-US/bundle/zurich-servicenow-platform/page/product/mid-server/concept/c_MIDServerConnectionPrerequisites.html"),
        ],
      ),
      concurrent_state: note(
        "ServiceNow is rated partial for concurrent state because tasks, records, approvals, service cases, and flows coordinate shared workflow state, while transactional semantics remain tied to the Now Platform data model.",
        [
          source("ServiceNow Flow Designer", "https://www.servicenow.com/docs/r/build-workflows/build-workflows.html"),
          source("ServiceNow Customer Service Management", "https://www.servicenow.com/products/customer-service-management.html"),
        ],
      ),
    },
  },
  Salesforce: {
    summary:
      "Salesforce combines Flow Builder, Lightning App Builder, mobile/offline capabilities, and Agentforce around CRM and related clouds. It is powerful for Salesforce-centric work, but data and logic live in the Salesforce platform model.",
    sources: [
      source("Lightning App Builder", "https://www.salesforce.com/products/platform/best-practices/how-do-you-build-an-app/"),
      source("Salesforce Agentforce", "https://www.salesforce.com/agentforce/"),
      source("Salesforce mobile offline guide", "https://resources.docs.salesforce.com/latest/latest/en-us/sfdc/pdf/mobile_offline.pdf"),
      source("Salesforce Files", "https://help.salesforce.com/s/articleView?id=sf.collab_files_overview.htm&language=en_US"),
    ],
    cells: {
      ...enterpriseWorkflowPlatformCells("Salesforce", "Flow Builder, Lightning App Builder, Experience Cloud, mobile apps, and Agentforce", [
        source("Lightning App Builder", "https://www.salesforce.com/products/platform/best-practices/how-do-you-build-an-app/"),
        source("Salesforce Agentforce", "https://www.salesforce.com/agentforce/"),
        source("Salesforce mobile offline guide", "https://resources.docs.salesforce.com/latest/latest/en-us/sfdc/pdf/mobile_offline.pdf"),
        source("Salesforce Files", "https://help.salesforce.com/s/articleView?id=sf.collab_files_overview.htm&language=en_US"),
      ]),
      file_size: {
        summary:
          "Salesforce is now rated 2 GB because Salesforce Files documents a 2 GB maximum file size for uploads through Salesforce Classic, Chatter, or Experience Cloud LWR sites.",
        caveat:
          "Older attachment APIs and specific upload paths can be much smaller, so this rating uses Salesforce Files rather than legacy Attachment records.",
        sources: [
          source("Salesforce Files", "https://help.salesforce.com/s/articleView?id=sf.collab_files_overview.htm&language=en_US"),
          source("Salesforce integration patterns", "https://architect.salesforce.com/docs/architect/fundamentals/guide/integration-patterns"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Salesforce is now rated native for AI agents because Salesforce describes Agentforce as the Agentforce 360 Platform for building, controlling, and deploying trusted autonomous AI agents across Salesforce applications and workflows.",
        caveat:
          "Agentforce is native to Salesforce's platform, data, trust, and CRM ecosystem rather than a portable general-purpose agent runtime.",
        sources: [
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform"),
          source("Agentforce announcement", "https://www.salesforce.com/news/press-releases/2024/09/12/agentforce-announcement/"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: note(
        "Salesforce is rated native for visual workflow building because Salesforce Flow is documented as a no-code platform for visual guided experiences, background automation, and approvals, and Lightning App Builder provides drag-and-drop app pages.",
        [
          source("Salesforce Flow", "https://help.salesforce.com/s/articleView?id=platform.platform_automation.htm&language=en_US&type=5"),
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
        ],
      ),
      replayable: note(
        "Salesforce is rated partial for replayability because Flow supports paused/failed interviews, debug paths, and operational recovery, but public docs do not describe deterministic event-history replay of Salesforce automation.",
        [
          source("Paused Flow Interview considerations", "https://help.salesforce.com/s/articleView?id=sf.flow_considerations_design_pause.htm&language=en_US&type=5"),
          source("Flow debugging", "https://help.salesforce.com/s/articleView?id=release-notes.rn_automate_flow_debug.htm&language=en_US&release=232&type=5"),
        ],
      ),
      high_volume: note(
        "Salesforce is rated partial for high-volume execution because Bulk API, Platform Events, Batch Apex, and async processing support large workloads, but Salesforce explicitly documents governor, capacity, and licensing limits.",
        [
          source("Salesforce async processing guide", "https://architect.salesforce.com/docs/architect/fundamentals/guide/async-fundamentals"),
          source("Salesforce async decision guide", "https://architect.salesforce.com/decision-guides/async-processing"),
        ],
      ),
      compiled: note(
        "Salesforce remains rated none for portable compiled business logic because Flow, metadata, Apex, and Agentforce logic run inside Salesforce rather than compiling into a vendor-neutral workflow runtime.",
        [
          source("Salesforce Flow", "https://help.salesforce.com/s/articleView?id=platform.platform_automation.htm&language=en_US&type=5"),
          source("Apex Developer Guide", "https://resources.docs.salesforce.com/latest/latest/en-us/sfdc/pdf/salesforce_apex_developer_guide.pdf"),
        ],
      ),
      file_native: note(
        "Salesforce remains rated none for file-native workflows because Salesforce Files are CRM/platform records and content objects, not local project files owned by an offline-first runtime.",
        [
          source("Salesforce Files", "https://help.salesforce.com/s/articleView?id=sf.collab_files_overview.htm&language=en_US"),
        ],
      ),
      data_science: note(
        "Salesforce is rated partial for analytics and ML workflows because CRM Analytics, Tableau, Data 360, and Agentforce analytics are built around Salesforce data, but Salesforce is not primarily a notebook or ML-training platform.",
        [
          source("CRM Analytics", "https://www.salesforce.com/analytics/crm/"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      ui_builder: note(
        "Salesforce is rated native for UI building because Lightning App Builder creates mobile and Lightning Experience pages with drag-and-drop components, dynamic forms, actions, and visibility rules.",
        [
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
          source("Lightning App Builder help", "https://help.salesforce.com/s/articleView?id=sf.lightning_app_builder_overview.htm&language=en_US&type=5"),
        ],
      ),
      full_apps: note(
        "Salesforce is rated native for full apps because the Lightning Platform, app pages, Flow, Apex, Experience Cloud, and AppExchange model can deliver complete Salesforce-native business applications.",
        [
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      customer_facing: note(
        "Salesforce is rated native for customer-facing apps because Experience Cloud customer portals include Experience Builder, authenticated portals, help centers, service catalogs, workflows, and Mobile Publisher.",
        [
          source("Salesforce customer portal", "https://www.salesforce.com/products/experience-cloud/customer-community/"),
        ],
      ),
      desktop: note(
        "Salesforce remains rated none for desktop application packaging because its app model targets Salesforce web, mobile, Experience Cloud, and platform-hosted apps rather than arbitrary native desktop binaries.",
        [
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
          source("Salesforce customer portal", "https://www.salesforce.com/products/experience-cloud/customer-community/"),
        ],
      ),
      mobile: note(
        "Salesforce is rated native for mobile because Lightning App Builder targets Salesforce mobile pages, Salesforce Mobile App Plus supports mobile offline, and Mobile Publisher can turn Experience Cloud portals into branded iOS/Android apps.",
        [
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
          source("Salesforce Mobile Offline", "https://help.salesforce.com/s/articleView?id=sf.salesforce_app_plus_offline.htm&language=en_US&type=5"),
          source("Salesforce customer portal", "https://www.salesforce.com/products/experience-cloud/customer-community/"),
        ],
      ),
      offline: note(
        "Salesforce is rated partial for offline execution because Mobile Offline lets users create, edit, and delete records without connectivity and sync later, but this is scoped to supported Salesforce mobile data and components.",
        [
          source("Salesforce Mobile Offline", "https://help.salesforce.com/s/articleView?id=sf.salesforce_app_plus_offline.htm&language=en_US&type=5"),
          source("Work offline with Salesforce mobile", "https://help.salesforce.com/s/articleView?id=sf.salesforce_app_offline.htm&language=en_US&type=5"),
        ],
      ),
      local_first: note(
        "Salesforce remains rated none for local-first architecture because the canonical data, metadata, permissions, flows, and agents live in Salesforce cloud orgs even when mobile users cache records offline.",
        [
          source("Salesforce Mobile Offline", "https://help.salesforce.com/s/articleView?id=sf.salesforce_app_plus_offline.htm&language=en_US&type=5"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      governance: note(
        "Salesforce is rated enterprise for governance because Shield, Event Monitoring, Field Audit Trail, platform access controls, and Agentforce governance/security controls are core enterprise surfaces.",
        [
          source("Salesforce Shield", "https://help.salesforce.com/articleView?id=salesforce_shield.htm&language=en_US"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      self_hosted: note(
        "Salesforce remains rated none for self-hosting because Salesforce apps and agents run on Salesforce cloud/platform services, not a customer-run open runtime.",
        [
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
        ],
      ),
      lock_in: note(
        "Salesforce is rated high lock-in because apps, CRM data, Flow/Apex logic, Agentforce agents, permissions, Experience Cloud, and analytics are tied to Salesforce metadata and platform services.",
        [
          source("Lightning App Builder", "https://www.salesforce.com/platform/drag-and-drop-app-builder/"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      sandbox_isolation: note(
        "Salesforce is rated partial for sandbox isolation because Salesforce provides tenant security, platform permissions, and governed execution, but it is not a portable sandbox for arbitrary untrusted tools.",
        [
          source("Salesforce Shield", "https://help.salesforce.com/articleView?id=salesforce_shield.htm&language=en_US"),
          source("Agentforce 360 Platform", "https://www.salesforce.com/platform/agentforce-platform/"),
        ],
      ),
      concurrent_state: note(
        "Salesforce is rated native for concurrent state because platform records, transactions, locks, flow interviews, approvals, and database semantics coordinate shared CRM/application state.",
        [
          source("Salesforce record locking", "https://help.salesforce.com/s/articleView?id=000387767&language=en_US&type=1"),
          source("Paused Flow Interview considerations", "https://help.salesforce.com/s/articleView?id=sf.flow_considerations_design_pause.htm&language=en_US&type=5"),
        ],
      ),
    },
  },
  Regrello: {
    summary:
      "Regrello is now positioned through Salesforce as Agentforce Supply Chain, an AI-native system for manufacturing and supply-chain back-office processes. It is strong for structured supply-chain workflows, but now sits inside the Salesforce platform orbit rather than a portable app runtime.",
    sources: [
      source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
      source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
      source("Regrello home", "https://www.regrello.com/"),
      source("Regrello security", "https://www.regrello.com/security"),
    ],
    cells: {
      ...enterpriseWorkflowPlatformCells("Regrello", "supply-chain operations workflows, human-in-the-loop processes, AI agents, and process standardization", [
        source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
        source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        source("Regrello careers", "https://www.regrello.com/careers"),
        source("Regrello security", "https://www.regrello.com/security"),
      ]),
      visual_workflow: note(
        "Regrello is rated native for visual/no-code workflow building because Agentforce Supply Chain documents reusable digital blueprints for processes created from natural language, documents, or diagrams, plus a blueprint library.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
      ),
      replayable: note(
        "Regrello is rated partial for replayability because Agentforce Supply Chain manages automated workflow execution across humans and AI agents, but public docs do not describe deterministic event-history replay.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
      ),
      high_volume: note(
        "Regrello is rated partial for high-volume work because Salesforce positions Agentforce Supply Chain for large manufacturing and supply-chain operations, but public docs do not publish workflow throughput, queue, or payload limits.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello security", "https://www.regrello.com/security"),
        ],
      ),
      compiled: note(
        "Regrello remains rated none for compiled business logic because processes are modeled as hosted blueprints, workflows, tasks, and AI-agent actions rather than compiled portable workflow code.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
        ],
      ),
      ai_agents: {
        summary:
          "Regrello is rated native for AI agents because Agentforce Supply Chain is documented as using AI-powered agents to complete supply-chain back-office tasks and orchestrate humans and specialized AI agents.",
        caveat:
          "The agent runtime is now part of Salesforce Agentforce Supply Chain, not a portable framework or standalone runtime.",
        sources: [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Regrello remains rated depends for file/payload limits because Agentforce Supply Chain mentions creating blueprints from documents or diagrams, but public docs do not expose a single upload or payload maximum.",
        caveat:
          "Enterprise contract terms and configured workflow artifacts may define practical limits that are not public.",
        sources: [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Regrello remains rated none for file-native workflows because documents and diagrams feed hosted process blueprints rather than becoming local project files managed by an offline-first runtime.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
        ],
      ),
      data_science: note(
        "Regrello is rated partial for analytics and ML workflows because Agentforce Supply Chain focuses on process visibility, automation, and AI-assisted operations, but it is not a notebook, warehouse, or model-training platform.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello security", "https://www.regrello.com/security"),
        ],
      ),
      ui_builder: note(
        "Regrello is rated partial for UI building because Agentforce Supply Chain provides no-code process blueprints and operational workflow surfaces, but not a general screen/form/dashboard builder for arbitrary apps.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
        ],
      ),
      full_apps: note(
        "Regrello remains rated none for full apps because Agentforce Supply Chain is a packaged supply-chain/back-office process solution, not a platform for shipping arbitrary standalone apps.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
      ),
      customer_facing: note(
        "Regrello is rated partial for customer-facing delivery because supply-chain workflows can coordinate external process participants, but public docs do not describe a general customer-facing app delivery model.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello privacy policy", "https://www.regrello.com/privacy-policy"),
        ],
      ),
      desktop: note(
        "Regrello remains rated none for desktop delivery because public materials describe a cloud/Salesforce process platform, not packaged native desktop apps.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello home", "https://www.regrello.com/"),
        ],
      ),
      mobile: note(
        "Regrello remains rated partial for mobile because Regrello's privacy policy covers mobile applications as part of the Services, but public product pages do not describe a mobile app builder.",
        [
          source("Regrello privacy policy", "https://www.regrello.com/privacy-policy"),
        ],
      ),
      offline: note(
        "Regrello remains rated none for offline execution because current Agentforce Supply Chain and Regrello materials describe cloud services and do not document offline workflow execution.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello security", "https://www.regrello.com/security"),
        ],
      ),
      local_first: note(
        "Regrello remains rated none for local-first architecture because customer data and workflow execution are described as cloud-hosted service data, not device-local project data with sync semantics.",
        [
          source("Regrello security", "https://www.regrello.com/security"),
          source("Regrello privacy policy", "https://www.regrello.com/privacy-policy"),
        ],
      ),
      governance: {
        summary:
          "Regrello is rated enterprise for governance because Agentforce Supply Chain is now Salesforce-backed and Regrello security materials document tenant separation, encryption, MFA for internal access, annual third-party pentesting, and SOC 2 Type 2 reporting.",
        sources: [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello security", "https://www.regrello.com/security"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Regrello remains rated none for self-hosting because current public materials present Agentforce Supply Chain/Regrello as Salesforce-hosted cloud services and do not document a customer-run runtime.",
        sources: [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello security", "https://www.regrello.com/security"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Regrello is rated high lock-in because current positioning routes Regrello into Salesforce Agentforce Supply Chain, with hosted process blueprints, AI agents, data, and governance tied to the Salesforce/Regrello platform.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Regrello home", "https://www.regrello.com/"),
        ],
      ),
      sandbox_isolation: note(
        "Regrello is rated partial for sandbox isolation because Regrello security materials document VPC isolation, strict tenant separation, dedicated databases/buckets/caches per tenant, and security testing, but not a portable untrusted-code sandbox.",
        [
          source("Regrello security", "https://www.regrello.com/security"),
        ],
      ),
      concurrent_state: note(
        "Regrello is rated partial for concurrent state because Agentforce Supply Chain coordinates digital blueprints, human tasks, AI agents, and operational workflows, while transactional state semantics are platform-specific.",
        [
          source("Agentforce Supply Chain", "https://www.salesforce.com/agentforce/agentforce-supply-chain/"),
          source("Agentforce Supply Chain help", "https://help.salesforce.com/s/articleView?id=platform.automate_agentforce_supply_chain.htm&language=en_US&type=5"),
        ],
      ),
    },
  },
  UiPath: {
    summary:
      "UiPath is an enterprise automation/RPA platform with Studio, robots, Orchestrator queues, apps, and newer agent features. It is strong for desktop and legacy-system automation, but carries RPA deployment and platform-governance overhead.",
    sources: [
      source("UiPath queue monitoring", "https://docs.uipath.com/orchestrator/automation-cloud-public-sector/latest/user-guide/monitoring-queues"),
      source("UiPath Studio Web capabilities", "https://docs.uipath.com/studio-web/automation-suite/2.2510/user-guide/capabilities"),
      source("UiPath Agent Builder", "https://www.uipath.com/product/agent-builder"),
      source("UiPath File Uploader", "https://docs.uipath.com/studio-web/automation-cloud/latest/user-guide/file-uploader"),
    ],
    cells: {
      ...rpaPlatformCells("UiPath", "Studio, Studio Web, robots, Orchestrator queues, Apps, Maestro, and Agent Builder", [
        source("UiPath queue monitoring", "https://docs.uipath.com/orchestrator/automation-cloud-public-sector/latest/user-guide/monitoring-queues"),
        source("UiPath Studio Web capabilities", "https://docs.uipath.com/studio-web/automation-suite/2.2510/user-guide/capabilities"),
        source("UiPath Agent Builder", "https://www.uipath.com/product/agent-builder"),
        source("UiPath File Uploader", "https://docs.uipath.com/studio-web/automation-cloud/latest/user-guide/file-uploader"),
      ]),
      visual_workflow: {
        summary:
          "UiPath is rated native for visual workflow building because Studio, Studio Web, and Apps provide first-party visual surfaces for automations, app screens, controls, events, and Orchestrator-connected workflows.",
        sources: [
          source("UiPath Studio Web capabilities", "https://docs.uipath.com/studio-web/automation-suite/2.2510/user-guide/capabilities"),
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "UiPath is rated partial for replayability because Orchestrator queues support transaction monitoring, status history, review requests, and auto-retry settings, but not deterministic replay of a completed workflow from an event log.",
        sources: [
          source("UiPath queues and transactions", "https://docs.uipath.com/orchestrator/standalone/2021.10/user-guide/about-queues-and-transactions"),
          source("UiPath queue management", "https://docs.uipath.com/orchestrator/v2020.10/docs/managing-queues-in-orchestrator"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "UiPath is rated partial for high-volume throughput because Orchestrator queues can hold unlimited work items and robot fleets can process queue transactions, but capacity still depends on robots, licenses, sessions, target systems, and deployment sizing.",
        sources: [
          source("UiPath queues and transactions", "https://docs.uipath.com/orchestrator/standalone/2021.10/user-guide/about-queues-and-transactions"),
          source("UiPath Orchestrator", "https://www.uipath.com/product/orchestrator"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "UiPath is rated none for compiled business logic because automations, app definitions, queues, and robots are UiPath platform artifacts rather than portable compiled workflow code.",
        sources: [
          source("UiPath Studio Web capabilities", "https://docs.uipath.com/studio-web/automation-suite/2.2510/user-guide/capabilities"),
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "UiPath is now rated 10 MB for app-facing file uploads because the current File Uploader docs recommend a 10 MB maximum file size for uploaded files.",
        caveat:
          "UiPath Orchestrator package uploads historically default around 30 MB and storage buckets can use external backing stores; this cell uses the UiPath Apps/File Uploader surface because it is the closest app-facing file workflow.",
        sources: [
          source("UiPath File Uploader", "https://docs.uipath.com/studio-web/automation-cloud/latest/user-guide/file-uploader"),
          source("UiPath storage buckets", "https://docs.uipath.com/orchestrator/standalone/2022.10/user-guide/about-storage-buckets"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "UiPath is now rated native for AI agents because UiPath documents Agent Builder for creating, testing, deploying, and orchestrating agents with robots, people, and governance.",
        caveat:
          "The agent capability is native to the UiPath automation platform rather than a portable open agent runtime.",
        sources: [
          source("UiPath Agent Builder", "https://www.uipath.com/product/agent-builder"),
          source("UiPath Agentic AI", "https://www.uipath.com/platform/agentic-automation/agentic-ai"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "UiPath is rated partial for UI building because UiPath Apps is a low-code app builder with controls, data access, conditional logic, and automation-backed screens, but it is scoped to the UiPath platform.",
        sources: [
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
          source("UiPath Apps product", "https://www.uipath.com/product/apps"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "UiPath is now rated partial for full apps because UiPath Apps can build and publish custom business applications, but those applications run through UiPath Cloud or Automation Suite and are not portable standalone app packages.",
        sources: [
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
          source("UiPath Apps product", "https://www.uipath.com/product/apps"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "UiPath is rated partial for customer-facing delivery because Public Apps can expose UiPath Apps outside Automation Cloud, including vendor or public forms, but public apps are anonymous and carry explicit security caveats.",
        sources: [
          source("UiPath public apps", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/use-public-apps"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "UiPath is now rated partial for desktop delivery because Apps are browser-run experiences that can be wrapped for a desktop-app experience, while UiPath does not package arbitrary native desktop applications from the platform.",
        sources: [
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "UiPath is rated partial for mobile delivery because Apps documentation supports mobile-friendly layouts and browser access across mobile devices, but not generated native iOS or Android apps.",
        sources: [
          source("UiPath mobile-friendly Apps", "https://docs.uipath.com/apps/automation-suite/2.2510/user-guide/build-a-mobile-friendly-app"),
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "UiPath remains rated none for offline user experiences because Apps, Orchestrator queues, robots, agents, and governance depend on Automation Cloud or Automation Suite services even when Suite itself is installed on customer infrastructure.",
        sources: [
          source("UiPath Apps data flow", "https://docs.uipath.com/apps/automation-suite/2023.10/legacy-user-guide/data-flow-between-uipath-apps-and-orchestrator"),
          source("UiPath Automation Suite", "https://docs.uipath.com/automation-suite/automation-suite/2.2510/admin-guide/about-the-automation-suite-experience"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "UiPath is rated none for local-first architecture because canonical apps, queues, assets, jobs, permissions, and audit data live in UiPath Orchestrator or Automation Suite rather than in device-owned local projects.",
        sources: [
          source("UiPath Apps data flow", "https://docs.uipath.com/apps/automation-suite/2023.10/legacy-user-guide/data-flow-between-uipath-apps-and-orchestrator"),
          source("UiPath Orchestrator", "https://www.uipath.com/product/orchestrator"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "UiPath is rated none for file-native workflows because files are uploaded through controls, storage buckets, or document automation paths rather than managed as first-class local project artifacts.",
        sources: [
          source("UiPath File Uploader", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/file-uploader"),
          source("UiPath storage buckets", "https://docs.uipath.com/orchestrator/standalone/2022.10/user-guide/about-storage-buckets"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "UiPath is rated partial for data science because the platform includes process mining, document understanding, insights, and AI automation features, but it is not primarily a notebook or model-training runtime.",
        sources: [
          source("UiPath Process Mining", "https://www.uipath.com/product/process-mining"),
          source("UiPath Document Understanding", "https://www.uipath.com/product/document-understanding"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "UiPath is rated enterprise for governance because Automation Suite and Orchestrator materials document deployment administration, Automation Ops governance, audit logs, roles, queues, machines, folders, and managed robot operations.",
        sources: [
          source("UiPath Automation Suite", "https://docs.uipath.com/automation-suite/automation-suite/2.2510/admin-guide/about-the-automation-suite-experience"),
          source("UiPath audit logs", "https://docs.uipath.com/uipath-cli/standalone/latest/user-guide/uip-orchestrator-audit-logs"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "UiPath is rated partial for self-hosting because Automation Suite can be installed on-premises or in customer cloud environments, while UiPath remains a proprietary platform rather than an open portable runtime.",
        sources: [
          source("UiPath Automation Suite", "https://docs.uipath.com/automation-suite/automation-suite/2.2510/admin-guide/about-the-automation-suite-experience"),
          source("UiPath Automation Suite product", "https://www.uipath.com/product/automation-suite"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "UiPath is rated high lock-in because robots, queues, app screens, agents, storage buckets, governance policies, and Orchestrator operations are modeled inside the UiPath platform.",
        sources: [
          source("UiPath Apps introduction", "https://docs.uipath.com/apps/automation-cloud/latest/user-guide/introduction"),
          source("UiPath Orchestrator", "https://www.uipath.com/product/orchestrator"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "UiPath is rated partial for sandbox isolation because robots run under Orchestrator-controlled machines and Automation Suite boundaries, but public docs do not make that a portable hardened sandbox for arbitrary untrusted tools.",
        sources: [
          source("UiPath Orchestrator", "https://www.uipath.com/product/orchestrator"),
          source("UiPath Automation Suite", "https://docs.uipath.com/automation-suite/automation-suite/2.2510/admin-guide/about-the-automation-suite-experience"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "UiPath is rated partial for concurrent state because Orchestrator queues coordinate transaction status, retries, and robot work distribution, while business data consistency still depends on target systems and workflow design.",
        sources: [
          source("UiPath queues and transactions", "https://docs.uipath.com/orchestrator/standalone/2021.10/user-guide/about-queues-and-transactions"),
          source("UiPath queue management", "https://docs.uipath.com/orchestrator/v2020.10/docs/managing-queues-in-orchestrator"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Automation Anywhere": {
    summary:
      "Automation Anywhere Automation 360 is an RPA and intelligent automation platform with Control Room, Bot Agent, AI Agent Studio, document automation, and cloud/on-prem options. It is optimized for digital-worker automation rather than portable application delivery.",
    sources: [
      source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
      source("Automation Anywhere APA Platform", "https://www.automationanywhere.com/products/agentic-process-automation-system"),
      source("Automation Anywhere Bot Agent", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/bot-agent/bot-agent-overview.html"),
      source("Automation Anywhere cloud storage usage", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-pc-cloud-storage-usage.html"),
    ],
    cells: {
      ...rpaPlatformCells("Automation Anywhere", "Automation 360, Control Room, Bot Agent, AI Agent Studio, document automation, and agentic process automation", [
        source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
        source("Automation Anywhere APA Platform", "https://www.automationanywhere.com/products/agentic-process-automation-system"),
        source("Automation Anywhere Bot Agent", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/bot-agent/bot-agent-overview.html"),
        source("Automation Anywhere cloud storage usage", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-pc-cloud-storage-usage.html"),
      ]),
      visual_workflow: {
        summary:
          "Automation Anywhere is rated native for visual workflow building because Automation 360 and Co-Pilot docs describe bot building, forms, embedded automation, and agentic process automation in the platform.",
        sources: [
          source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Automation Anywhere is rated partial for replayability because Control Room centralizes bot deployment, scheduling, monitoring, version control, and audit trails, but not deterministic replay of completed workflow state.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Manage Automation 360", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-client/bot-creator/using-the-workbench/cloud-manage.html"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Automation Anywhere is rated partial for high-volume execution because Control Room manages bot execution, Bot Runners, schedules, and centralized deployment, while throughput still depends on runners, device pools, licenses, and target applications.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Automation Anywhere is rated none for compiled business logic because bots, forms, Co-Pilot configurations, and AI-agent skills are Automation 360 platform assets rather than portable compiled app code.",
        sources: [
          source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Automation Anywhere is now rated 50 MB because the cloud-storage docs state that a file uploaded through the Select File element cannot be greater than 50 MB.",
        caveat:
          "The documented limit is attached to the Select File element, not every possible automation payload or external storage path.",
        sources: [
          source("Automation Anywhere cloud storage usage", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-pc-cloud-storage-usage.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Automation Anywhere is now rated native for AI agents because its current platform materials describe AI Agent Studio and an Agentic Process Automation system for governed agent, bot, document, and API orchestration.",
        caveat:
          "This is native inside Automation Anywhere's APA/Automation 360 platform, not a portable standalone agent framework.",
        sources: [
          source("Automation Anywhere APA Platform", "https://www.automationanywhere.com/products/agentic-process-automation-system"),
          source("Automation Anywhere home", "https://www.automationanywhere.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Automation Anywhere is rated partial for UI building because Automation Co-Pilot provides front-end forms and embedded automation panels, but those surfaces are process-participation UI rather than a general app builder.",
        sources: [
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
          source("Embedded Automation Co-Pilot", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-architecture-implementation/embedded-automations.html"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Automation Anywhere remains rated none for full apps because its documented user-facing surfaces run bots, forms, and embedded automations rather than packaging complete standalone business applications.",
        sources: [
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
          source("Embedded Automation Co-Pilot", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-architecture-implementation/embedded-automations.html"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Automation Anywhere is rated partial for customer-facing delivery because Automation Co-Pilot can be embedded in web applications to expose automations, but it is an embedded assistant/widget model rather than full external app delivery.",
        sources: [
          source("Embedded Automation Co-Pilot", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-architecture-implementation/embedded-automations.html"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Automation Anywhere is rated none for desktop app delivery because bot runners and Co-Pilot desktop access automate or operate on desktop environments; they do not generate packaged customer desktop applications.",
        sources: [
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
          source("Automation Anywhere Bot Agent", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/bot-agent/bot-agent-overview.html"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Automation Anywhere is now rated none for mobile app delivery because current Co-Pilot documentation says forms and mobile-device use are not supported, and the Teams integration is not supported on mobile devices.",
        sources: [
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
          source("Co-Pilot in Microsoft Teams", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-architecture-implementation/embedded-ms-teams-setup.html"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Automation Anywhere is rated none for offline user experiences because Control Room, Bot Agent registration, Co-Pilot, credentials, scheduling, and cloud/on-prem governance are platform-connected services.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Automation Anywhere Bot Agent", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/bot-agent/bot-agent-overview.html"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Automation Anywhere is rated none for local-first architecture because bots, credentials, roles, schedules, versions, and execution state are centralized through Control Room rather than stored as device-owned local projects.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Manage Automation 360", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-client/bot-creator/using-the-workbench/cloud-manage.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Automation Anywhere is rated none for file-native workflows because files are uploaded through forms, cloud storage, document automation, or bot actions rather than managed as local project artifacts.",
        sources: [
          source("Automation Anywhere cloud storage usage", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-pc-cloud-storage-usage.html"),
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Automation Anywhere is rated partial for data and AI work because the platform includes Document Automation, Bot Insight, AI Agent Studio, and model connections, but not a general notebook or ML training environment.",
        sources: [
          source("Automation Anywhere docs", "https://docs.automationanywhere.com/"),
          source("AI Agent Studio FAQs", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/ai-studio-faq.html"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Automation Anywhere is rated enterprise for governance because Control Room centralizes users, roles, licenses, Credential Vault, bot schedules, audit logs, version control, and bot execution control.",
        sources: [
          source("Manage Automation 360", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-client/bot-creator/using-the-workbench/cloud-manage.html"),
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Automation Anywhere is rated partial for self-hosting because Automation 360 includes cloud and on-premises deployment models, but Control Room remains a proprietary platform control plane.",
        sources: [
          source("Automation Anywhere glossary", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/glossary.html"),
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Automation Anywhere is rated high lock-in because bots, credentials, lockers, schedules, Control Room configuration, Co-Pilot surfaces, and AI-agent assets are platform-specific.",
        sources: [
          source("Manage Automation 360", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/aae-client/bot-creator/using-the-workbench/cloud-manage.html"),
          source("Automation Anywhere APA Platform", "https://www.automationanywhere.com/products/agentic-process-automation-system"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Automation Anywhere is rated partial for sandbox isolation because Control Room governs bot execution and Bot Runners, but public docs do not present it as a portable hardened sandbox for arbitrary untrusted tools.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Automation Anywhere Bot Agent", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/enterprise-cloud/topics/bot-agent/bot-agent-overview.html"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Automation Anywhere is rated partial for concurrent state because Control Room, schedules, queues, and bot runners coordinate automation execution, while transactional business state still belongs to the systems being automated.",
        sources: [
          source("Automation Anywhere Control Room architecture", "https://docs.automationanywhere.com/bundle/enterprise-v11.3/page/enterprise/topics/aae-architecture-implementation/control-room-overview.html"),
          source("Automation Co-Pilot for Business Users", "https://docs.automationanywhere.com/bundle/enterprise-v2019/page/cp-bu-introduction.html"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Blue Prism": {
    summary:
      "Blue Prism is an enterprise RPA platform organized around Process Studio, digital workers, work queues, schedules, and Control Room operations. It is strong for governed bot fleets, but not a general app, AI-agent, or offline-first runtime.",
    sources: [
      source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
      source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
      source("Blue Prism Control Room", "https://documentation.blueprism.com/bp-7-2/en-us/frmControlRoom.htm?TocPath=Interface%7CControl%7C_____0"),
      source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
    ],
    cells: {
      ...rpaPlatformCells("Blue Prism", "Process Studio, digital workers, work queues, schedules, and Control Room operations", [
        source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
        source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
        source("Blue Prism Control Room", "https://documentation.blueprism.com/bp-7-2/en-us/frmControlRoom.htm?TocPath=Interface%7CControl%7C_____0"),
        source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
      ]),
      visual_workflow: {
        summary:
          "Blue Prism is rated native for visual workflow building because Process Studio is the documented visual design surface for automation processes.",
        caveat:
          "This is RPA process design, not a modern app/workflow canvas for typed business apps.",
        sources: [
          source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Blue Prism is rated partial for replayability because Control Room and work queues support operational monitoring and retry-style bot operations, but not deterministic workflow replay.",
        sources: [
          source("Blue Prism Control Room", "https://documentation.blueprism.com/bp-7-2/en-us/frmControlRoom.htm?TocPath=Interface%7CControl%7C_____0"),
          source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Blue Prism is rated partial for high-volume execution because Application Server Controlled Resources and Control Room are designed to coordinate digital workers at scale, while capacity still depends on runtime resources and infrastructure.",
        sources: [
          source("Blue Prism ASCR", "https://documentation.blueprism.com/bp-7-1/en-us/ascr/ascr.htm"),
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Blue Prism is rated none for compiled business logic because processes, objects, queues, schedules, and Control Room assets are Blue Prism platform artifacts rather than portable compiled workflow code.",
        sources: [
          source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Blue Prism stays rated depends for file limits because the official docs reviewed describe queues, resources, and deployment architecture, but do not expose a single universal app-facing upload limit.",
        sources: [
          source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
          source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Blue Prism is rated none for file-native workflows because files are handled as bot inputs, queue data, or application documents rather than as local project artifacts managed by the runtime.",
        sources: [
          source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
          source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Blue Prism is rated partial for data and AI work because the platform supports intelligent automation and operational analytics around digital workers, but it is not a notebook, data pipeline, or model-training runtime.",
        sources: [
          source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
          source("SS&C Blue Prism intelligent automation", "https://www.blueprism.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Blue Prism remains rated partial for AI agents because public materials show intelligent automation and AI integrations around RPA, but not a native general-purpose agent runtime.",
        sources: [
          source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
          source("SS&C Blue Prism intelligent automation", "https://www.blueprism.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Blue Prism is rated partial for UI building because Hub, Interact, and browser-based Control Room surfaces support human interaction with automations, but not a general app-screen builder.",
        sources: [
          source("Blue Prism what's new", "https://documentation.blueprism.com/bp-7-4/en-us/whats-new.htm"),
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Blue Prism remains rated none for full apps because its public docs focus on RPA processes, digital workers, queues, schedules, APIs, and Control Room rather than complete portable business applications.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
          source("Blue Prism Control Room", "https://documentation.blueprism.com/bp-7-2/en-us/frmControlRoom.htm?TocPath=Interface%7CControl%7C_____0"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Blue Prism remains rated none for customer-facing app delivery because its documented surfaces are operational RPA tools and optional Hub/Control Room components, not external customer application runtime features.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
          source("Blue Prism what's new", "https://documentation.blueprism.com/bp-7-4/en-us/whats-new.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Blue Prism is rated none for desktop app delivery because interactive clients and runtime resources operate automations on desktops, but the platform does not generate packaged desktop applications.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
          source("Blue Prism ASCR", "https://documentation.blueprism.com/bp-7-1/en-us/ascr/ascr.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Blue Prism remains rated none for mobile app delivery because the official docs reviewed describe Enterprise, Hub, Control Room, APIs, and digital-worker operations, not generated native mobile apps.",
        sources: [
          source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Blue Prism is rated none for offline user experiences because runtime resources, Control Room, queues, schedules, authentication, and APIs are coordinated through the Blue Prism environment rather than offline-capable end-user apps.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
          source("Blue Prism Control Room", "https://documentation.blueprism.com/bp-7-2/en-us/frmControlRoom.htm?TocPath=Interface%7CControl%7C_____0"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Blue Prism is rated none for local-first architecture because processes, queues, schedules, users, authentication, and runtime coordination are stored in the Blue Prism environment and database.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Blue Prism is rated enterprise for governance because Enterprise deployments include authentication, Control Room, API, application-server architecture, queue monitoring, schedules, sessions, and digital-worker health controls.",
        sources: [
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
          source("Blue Prism what's new", "https://documentation.blueprism.com/bp-7-4/en-us/whats-new.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Blue Prism is rated partial for self-hosting because Blue Prism Enterprise can be customer-operated, while Cloud and managed deployment paths remain vendor/platform controlled.",
        sources: [
          source("Blue Prism docs", "https://docs.blueprism.com/en-US"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Blue Prism is rated high lock-in because processes, business objects, work queues, schedules, runtime resources, APIs, and Control Room operations are modeled in Blue Prism-specific platform concepts.",
        sources: [
          source("Blue Prism Process Studio", "https://documentation.blueprism.com/bp-7-2/en-us/frmProcessStudio.htm"),
          source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Blue Prism is rated partial for sandbox isolation because digital workers run as managed runtime resources through application-server and Control Room architecture, but that is not a portable hardened sandbox for arbitrary untrusted tools.",
        sources: [
          source("Blue Prism ASCR", "https://documentation.blueprism.com/bp-7-1/en-us/ascr/ascr.htm"),
          source("Blue Prism architecture overview", "https://documentation.blueprism.com/bp-7-1/en-us/Guides/infrastructure-reference/architecture-overview.htm?TocPath=Guides%7CInfrastructure+Reference+Architecture%7C_____1"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Blue Prism is rated partial for concurrent state because work queues, schedules, sessions, and digital workers coordinate RPA work, while business transaction consistency remains the responsibility of target systems and process design.",
        sources: [
          source("Blue Prism work queues", "https://documentation.blueprism.com/bp-7-2/en-us/helpWorkQueues.htm"),
          source("Blue Prism what's new", "https://documentation.blueprism.com/bp-7-4/en-us/whats-new.htm"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "ServiceNow GRC": {
    summary:
      "ServiceNow GRC/IRM manages policy, compliance, risk, audit, business continuity, and continuous monitoring workflows on the Now Platform. It is a governance suite, not a general automation or application runtime.",
    sources: [
      source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
      source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
      source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
      source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
    ],
    cells: {
      ...grcPlatformCells("ServiceNow GRC", "IRM/GRC policy, risk, compliance, audit, monitoring, and remediation workflows", [
        source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
        source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
        source("ServiceNow AI Agents", "https://www.servicenow.com/products/ai-agents.html"),
        source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
      ]),
      visual_workflow: note(
        "ServiceNow GRC is rated partial for visual workflow because IRM workflows, work configurations, workspaces, and remediation tasks are configurable on the Now Platform, but the scope is GRC/IRM process management rather than a portable workflow runtime.",
        [
          source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
        ],
      ),
      replayable: note(
        "ServiceNow GRC is rated partial for replayability because records, tasks, approvals, issues, and remediation history can be reviewed and acted on, but completed workflows are not deterministic replayable code artifacts.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
        ],
      ),
      high_volume: note(
        "ServiceNow GRC is rated partial for high-volume work because the Now Platform supports enterprise risk records, dashboards, tasks, and monitoring, while GRC execution remains service/workflow throughput rather than a bulk data-processing engine.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
        ],
      ),
      compiled: note(
        "ServiceNow GRC is rated none for compiled business logic because rules, flows, records, workspaces, and scripts are Now Platform configuration rather than portable compiled workflow code.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
        ],
      ),
      file_size: {
        summary:
          "ServiceNow GRC uses the same Now Platform attachment model, so it is now rated 1024 MB based on ServiceNow's current default maximum attachment size for new base-system instances.",
        caveat:
          "GRC-specific intake experiences or admin settings can impose lower limits.",
        sources: [
          source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "ServiceNow GRC is rated none for file-native workflows because files are platform attachments on risk, policy, issue, or evidence records, not local project files with device-level ownership.",
        [
          source("ServiceNow manage attachments", "https://www.servicenow.com/docs/r/platform-user-interface/t_ManagingAttachments.html"),
          source("ServiceNow mobile attachments", "https://horizon.servicenow.com/native-mobile/basics/leading-practices/attachments"),
        ],
      ),
      data_science: note(
        "ServiceNow GRC is rated partial for data science because IRM provides dashboards, real-time insights, risk scores, analytics, and AI-assisted risk workflows, but it is not an ML notebook or model-training environment.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow IRM agentic workflows", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-common-functions/using-agentic-ai-workflows.html"),
        ],
      ),
      ai_agents: note(
        "ServiceNow GRC is now rated native for AI agents because ServiceNow documents Integrated Risk Management agentic workflows and AI agents for issue resolution, regulatory insights, and regulatory action planning.",
        [
          source("ServiceNow IRM agentic workflows", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-common-functions/using-agentic-ai-workflows.html"),
          source("ServiceNow AI Agents SDK guide", "https://servicenow.github.io/sdk/guides/building-ai-agents-guide"),
        ],
        "The agents are native to the Now Platform and scoped by installed applications, licensing, activation, and governance.",
      ),
      ui_builder: note(
        "ServiceNow GRC is rated native for UI building because the Now Platform exposes IRM workspaces, forms, mobile interfaces, and configurable record experiences for risk users and administrators.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow mobile attachments", "https://horizon.servicenow.com/native-mobile/basics/leading-practices/attachments"),
        ],
      ),
      full_apps: note(
        "ServiceNow GRC is rated partial for full apps because IRM applications are complete risk-management experiences on the Now Platform, but they are not portable standalone apps outside ServiceNow.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
        ],
      ),
      customer_facing: note(
        "ServiceNow GRC is rated partial for customer-facing delivery because ServiceNow can expose tasks, portals, and mobile experiences, while GRC/IRM primarily serves internal risk, compliance, audit, and control teams.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow mobile attachments", "https://horizon.servicenow.com/native-mobile/basics/leading-practices/attachments"),
        ],
      ),
      desktop: note(
        "ServiceNow GRC is rated none for desktop packaging because IRM runs as Now Platform web/mobile experiences, not generated native desktop applications.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
        ],
      ),
      mobile: note(
        "ServiceNow GRC is rated partial for mobile because ServiceNow Risk Management materials mention mobile interfaces and ServiceNow Mobile supports record attachments, but this is platform mobile access rather than generated native app delivery.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow mobile attachments", "https://horizon.servicenow.com/native-mobile/basics/leading-practices/attachments"),
        ],
      ),
      offline: note(
        "ServiceNow GRC is rated none for offline execution because IRM records, controls, workflows, attachments, AI agents, and mobile access are centered on the Now Platform service.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow IRM agentic workflows", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-common-functions/using-agentic-ai-workflows.html"),
        ],
      ),
      local_first: note(
        "ServiceNow GRC is rated none for local-first architecture because canonical risk, policy, control, issue, evidence, and workflow records live in the Now Platform.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
        ],
      ),
      governance: note(
        "ServiceNow GRC is rated enterprise for governance because governance, risk, compliance, audit, business continuity, controls, issues, and policy workflows are the product domain.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
        ],
      ),
      self_hosted: note(
        "ServiceNow GRC is rated none for self-hosting because public IRM materials describe ServiceNow applications on the Now Platform rather than a customer-run portable runtime.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
        ],
      ),
      lock_in: note(
        "ServiceNow GRC is rated high lock-in because IRM records, workspaces, workflows, AI agents, permissions, attachments, and analytics are modeled as Now Platform artifacts.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow IRM agentic workflows", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-common-functions/using-agentic-ai-workflows.html"),
        ],
      ),
      sandbox_isolation: note(
        "ServiceNow GRC is rated partial for sandbox isolation because the hosted Now Platform supplies tenant and platform execution boundaries, but IRM is not a portable arbitrary-code sandbox.",
        [
          source("ServiceNow GRC docs", "https://www.servicenow.com/docs/r/governance-risk-compliance/r_WhatIsGRC.html"),
          source("ServiceNow AI Agents SDK guide", "https://servicenow.github.io/sdk/guides/building-ai-agents-guide"),
        ],
      ),
      concurrent_state: note(
        "ServiceNow GRC is rated partial for concurrent state because Now Platform records, tasks, issues, approvals, and remediation workflows coordinate shared risk work, while domain-specific transaction safety remains app-design dependent.",
        [
          source("ServiceNow Risk Management", "https://www.servicenow.com/docs/r/governance-risk-compliance/grc-risk-management-workspace/grc-risk-overview.html"),
          source("ServiceNow workflow configuration", "https://www.servicenow.com/docs/r/governance-risk-compliance/continuous-risk-monitoring/work-configuration.html"),
        ],
      ),
    },
  },
  Archer: {
    summary:
      "Archer is an integrated risk management platform for operational, IT, third-party, audit, policy, and compliance programs. Its configurable apps and workflows are GRC-oriented rather than general-purpose app delivery.",
    sources: [
      source("Archer platform", "https://www.archerirm.com/"),
      source("Archer solutions docs", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/solutions/solutions_intro.htm"),
      source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
    ],
    cells: {
      ...grcPlatformCells("Archer", "integrated risk management, audit, policy, compliance, and third-party risk workflows", [
        source("Archer platform", "https://www.archerirm.com/"),
        source("Archer solutions docs", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/solutions/solutions_intro.htm"),
        source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
      ]),
      visual_workflow: {
        summary:
          "Archer is rated partial for visual workflow because Archer Advanced Workflow supports configurable GRC workflow processes, but the product is not a general visual automation runtime.",
        sources: [
          source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "Archer is rated partial for replayability because Advanced Workflow creates jobs, versions, history logs, and troubleshooting paths for records, but those are not deterministic replay of portable workflow code.",
        [
          source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
          source("Archer building workflows", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/advancedworkflow/adv_wrkflw_building.htm"),
        ],
      ),
      high_volume: note(
        "Archer is rated none for high-volume execution because it is an IRM record and workflow platform; even data-feed docs recommend staggering feeds and limiting simultaneous feed runs to avoid excess server load.",
        [
          source("Archer data feeds", "https://help.archerirm.cloud/platform_2025_04/en-us/content/platform/integration/int_dfm_basics.htm"),
          source("Archer TPRM data feeds", "https://help.archerirm.cloud/thirdpty_riskmgmt_69/en-us/Content/Solutions/ThirdPtyGov/tpg_tprm_df_setting_up.htm"),
        ],
      ),
      compiled: note(
        "Archer is rated none for compiled business logic because applications, questionnaires, workflows, dashboards, and data feeds are Archer platform configuration rather than portable compiled code.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
          source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
        ],
      ),
      ai_agents: {
        summary:
          "Archer remains rated none for built-in AI agents because Archer's public platform and solutions documentation centers on IRM applications and workflows, not autonomous agent tooling.",
        caveat:
          "Some Archer deployments may integrate external AI, but that is not the same as first-party agent orchestration.",
        sources: [
          source("Archer platform", "https://www.archerirm.com/"),
          source("Archer solutions docs", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/solutions/solutions_intro.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Archer is rated enterprise for governance because the product category and public docs focus on integrated risk management, compliance, audit, policy, and control workflows.",
        sources: [
          source("Archer platform", "https://www.archerirm.com/"),
          source("Archer solutions docs", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/solutions/solutions_intro.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Archer is rated partial for self-hosting because Archer has both cloud and enterprise deployment heritage, but current public cloud docs do not make the product equivalent to an open self-hosted runtime.",
        sources: [
          source("Archer platform", "https://www.archerirm.com/"),
          source("Archer cloud documentation", "https://help.archerirm.cloud/platform_202508/en-us/content/home.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Archer's file-limit cell stays depends because Archer Cloud documentation does not expose one universal attachment or workflow payload maximum for this comparison row.",
        sources: [
          source("Archer cloud documentation", "https://help.archerirm.cloud/platform_202508/en-us/content/home.htm"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Archer is rated none for file-native workflows because attachments and document repositories support risk records, assessments, and data feeds rather than local project-file ownership.",
        [
          source("Archer TPRM data feeds", "https://help.archerirm.cloud/thirdpty_riskmgmt_69/en-us/Content/Solutions/ThirdPtyGov/tpg_tprm_df_setting_up.htm"),
          source("Archer building workflows", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/advancedworkflow/adv_wrkflw_building.htm"),
        ],
      ),
      data_science: note(
        "Archer is rated partial for analytics because it provides reports, dashboards, risk data aggregation, and scheduled data feeds, but not a data-science notebook or ML training runtime.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
          source("Archer data feeds", "https://help.archerirm.cloud/platform_2025_04/en-us/content/platform/integration/int_dfm_basics.htm"),
          source("Archer dashboards", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/ngx/ngx_intro_dashboards.htm"),
        ],
      ),
      ui_builder: note(
        "Archer is rated partial for UI building because the platform supports configurable applications, questionnaires, layouts, reports, and dashboards, but the UI model remains IRM-specific.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
          source("Archer dashboards", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/ngx/ngx_intro_dashboards.htm"),
        ],
      ),
      full_apps: note(
        "Archer is now rated partial for full apps because Archer Platform docs say teams can build their own applications without code, but those apps are Archer IRM applications rather than portable standalone products.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
        ],
      ),
      customer_facing: note(
        "Archer remains rated none for customer-facing app delivery because public docs describe risk, compliance, and first-line-of-defense users rather than external customer application hosting.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
        ],
      ),
      desktop: note(
        "Archer is rated none for desktop app delivery because Archer is a web/SaaS or on-premises IRM platform, not a generator for packaged native desktop applications.",
        [
          source("Archer getting started", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/getting_started.htm"),
        ],
      ),
      mobile: note(
        "Archer is rated partial for mobile because Archer IRM Mobile lets users view tasks, records, and complete Advanced Workflow actions, while large data volumes are still best handled in desktop browser experiences.",
        [
          source("Archer mobile app", "https://help.archerirm.cloud/mobile_14/Content/Platform/UserTasks/usr_mobile_basics.htm"),
          source("Archer dashboards", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/ngx/ngx_intro_dashboards.htm"),
        ],
      ),
      offline: note(
        "Archer is rated none for offline execution because records, workflows, mobile tasks, data feeds, and dashboards depend on an Archer instance and its platform services.",
        [
          source("Archer mobile app", "https://help.archerirm.cloud/mobile_14/Content/Platform/UserTasks/usr_mobile_basics.htm"),
          source("Archer getting started", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/getting_started.htm"),
        ],
      ),
      local_first: note(
        "Archer is rated none for local-first architecture because canonical risk records, workflows, assessments, data feeds, dashboards, and permissions live in the Archer instance.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
          source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
        ],
      ),
      lock_in: note(
        "Archer is rated high lock-in because applications, questionnaires, workflows, reports, dashboards, data feeds, access roles, and records are modeled in Archer-specific platform concepts.",
        [
          source("Archer platform help", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/archer.htm"),
          source("Archer data feeds", "https://help.archerirm.cloud/platform_2025_04/en-us/content/platform/integration/int_dfm_basics.htm"),
        ],
      ),
      sandbox_isolation: note(
        "Archer is rated partial for sandbox isolation because SaaS/on-premises platform boundaries and access controls exist, but Archer is not a sandbox for arbitrary untrusted code or agent tools.",
        [
          source("Archer getting started", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/gettingstarted/getting_started.htm"),
          source("Archer building workflows", "https://help.archerirm.cloud/platform_202508/en-us/content/platform/advancedworkflow/adv_wrkflw_building.htm"),
        ],
      ),
      concurrent_state: note(
        "Archer is rated partial for concurrent state because records, locks, tasks, workflow jobs, and data feeds coordinate shared GRC work, while general transactional app-state semantics are not the product focus.",
        [
          source("Archer mobile app", "https://help.archerirm.cloud/mobile_14/Content/Platform/UserTasks/usr_mobile_basics.htm"),
          source("Archer advanced workflow", "https://help.archerirm.cloud/platform_2024_11/en-us/content/platform/advancedworkflow/adv_wrkflw_basics.htm"),
        ],
      ),
    },
  },
  OneTrust: {
    summary:
      "OneTrust is a trust, privacy, GRC, and AI governance platform with inventories, risk assessments, approval workflows, attestations, monitoring, and policy enforcement. It is governance-first, not an execution platform for arbitrary apps.",
    sources: [
      source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
      source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
      source("OneTrust AI Governance announcement", "https://www.onetrust.com/news/onetrust-introduces-ai-governance-solution/"),
    ],
    cells: {
      ...grcPlatformCells("OneTrust", "privacy, trust, risk, compliance, AI governance, assessments, and policy workflows", [
        source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
        source("OneTrust AI Governance announcement", "https://www.onetrust.com/news/onetrust-introduces-ai-governance-solution/"),
      ]),
      ai_agents: {
        summary:
          "OneTrust is rated partial for AI agents because it provides AI governance and control workflows for AI systems, but public materials do not describe OneTrust as a first-party autonomous agent runtime.",
        sources: [
          source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
          source("OneTrust AI Governance announcement", "https://www.onetrust.com/news/onetrust-introduces-ai-governance-solution/"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: {
        summary:
          "OneTrust is rated partial for visual workflow because it supports assessments, tasks, approvals, inventories, and governance workflows, but not a general-purpose workflow/app builder.",
        sources: [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "OneTrust is rated partial for replayability because assessments, approvals, risk workflows, evidence, and audit-ready documentation can be tracked, but workflow logic is not replayed deterministically from event history.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
        ],
      ),
      high_volume: note(
        "OneTrust is rated none for high-volume execution because it manages governance, risk, privacy, third-party, and AI-control workflows rather than bulk automation or high-throughput data processing.",
        [
          source("OneTrust products", "https://www.onetrust.com/products/"),
          source("OneTrust third-party risk", "https://www.onetrust.com/products/third-party-risk-management/"),
        ],
      ),
      compiled: note(
        "OneTrust is rated none for compiled business logic because workflows, assessments, integrations, policies, and inventories are OneTrust platform configuration rather than portable compiled app code.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust integrations", "https://www.onetrust.com/integrations/"),
        ],
      ),
      file_size: note(
        "OneTrust stays rated depends for file limits because public product pages reviewed do not expose a single platform-wide upload or attachment maximum for this comparison row.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust trust center", "https://www.onetrust.com/trust/"),
        ],
      ),
      file_native: note(
        "OneTrust is rated none for file-native workflows because evidence, documents, and data maps support governance records rather than local project-file management.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust products", "https://www.onetrust.com/products/"),
        ],
      ),
      data_science: note(
        "OneTrust is rated partial for analytics because it provides dashboards, risk indexes, regulatory intelligence, AI inventory, monitoring, and audit-ready documentation, but not notebooks or model-training tools.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
        ],
      ),
      ui_builder: note(
        "OneTrust is rated partial for UI building because the platform offers no-code configuration, assessments, portals, dashboards, and a unified trust center, but not a general application-screen builder.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust products", "https://www.onetrust.com/products/"),
        ],
      ),
      full_apps: note(
        "OneTrust remains rated none for full apps because it delivers governance products and portals, not complete portable business applications built and shipped by customers.",
        [
          source("OneTrust products", "https://www.onetrust.com/products/"),
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        ],
      ),
      customer_facing: note(
        "OneTrust is rated partial for customer-facing delivery because consent, preference, data-subject, trust-center, and transparency experiences can face external stakeholders, but they are governance-specific portals rather than general apps.",
        [
          source("OneTrust products", "https://www.onetrust.com/products/"),
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        ],
      ),
      desktop: note(
        "OneTrust is rated none for desktop app delivery because public materials describe a managed platform, integrations, and web-facing governance products rather than generated native desktop apps.",
        [
          source("OneTrust products", "https://www.onetrust.com/products/"),
        ],
      ),
      mobile: note(
        "OneTrust is rated partial for mobile only because its consent and preference products include mobile-app privacy/consent use cases, not because OneTrust generates custom native mobile apps.",
        [
          source("OneTrust products", "https://www.onetrust.com/products/"),
        ],
      ),
      offline: note(
        "OneTrust is rated none for offline execution because governance workflows, inventories, integrations, trust centers, and AI governance controls are centered on the OneTrust platform service.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust integrations", "https://www.onetrust.com/integrations/"),
        ],
      ),
      local_first: note(
        "OneTrust is rated none for local-first architecture because canonical inventories, assessments, policies, data maps, evidence, and workflows live in the OneTrust platform.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
        ],
      ),
      governance: {
        summary:
          "OneTrust is rated enterprise for governance because governance, privacy, risk, compliance, and AI control programs are the core product domain.",
        sources: [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust AI Governance", "https://www.onetrust.com/products/ai-governance/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "OneTrust remains rated none for self-hosting because public product materials present it as a managed trust/governance platform and do not document a customer-run runtime.",
        sources: [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "OneTrust is rated high lock-in because inventories, risk objects, consent records, workflows, integrations, reports, and regulatory content are modeled inside the OneTrust platform.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust integrations", "https://www.onetrust.com/integrations/"),
        ],
      ),
      sandbox_isolation: note(
        "OneTrust is rated partial for sandbox isolation because it is a hosted governance platform with permissions and security controls, but not a sandbox for arbitrary untrusted workflow or agent execution.",
        [
          source("OneTrust trust center", "https://www.onetrust.com/trust/"),
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
        ],
      ),
      concurrent_state: note(
        "OneTrust is rated partial for concurrent state because it coordinates governance records, approvals, inventories, tasks, and integrations, while arbitrary transactional app-state semantics are outside the product focus.",
        [
          source("OneTrust platform", "https://www.onetrust.com/why-onetrust"),
          source("OneTrust integrations", "https://www.onetrust.com/integrations/"),
        ],
      ),
    },
  },
  CrewAI: {
    summary:
      "CrewAI is an open-source Python framework for crews, agents, tools, memory, knowledge, and event-driven flows. It is strong for code-defined multi-agent systems, but not a visual app platform or distribution runtime.",
    sources: [
      source("CrewAI docs", "https://docs.crewai.com/en/index"),
      source("CrewAI introduction", "https://docs.crewai.com/en/introduction"),
      source("CrewAI flows", "https://docs.crewai.com/en/concepts/flows"),
      source("CrewAI files", "https://docs.crewai.com/en/concepts/files"),
      source("CrewAI memory", "https://docs.crewai.com/en/concepts/memory"),
      source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
      source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
    ],
    cells: {
      ...codeFrameworkCells(
        "CrewAI",
        "Python agents, crews, memory, knowledge, and event-driven flows",
        [
          source("CrewAI docs", "https://docs.crewai.com/en/index"),
          source("CrewAI introduction", "https://docs.crewai.com/en/introduction"),
          source("CrewAI flows", "https://docs.crewai.com/en/concepts/flows"),
          source("CrewAI files", "https://docs.crewai.com/en/concepts/files"),
          source("CrewAI memory", "https://docs.crewai.com/en/concepts/memory"),
        ],
        "CrewAI is rated partial for data science because it can orchestrate agents, tools, files, knowledge, and memory around analytical code, but it is not itself a notebook, training, or data platform.",
      ),
      ai_agents: {
        summary:
          "CrewAI is rated native for AI agents because agents, crews, tools, memory, and multi-agent collaboration are core documented concepts.",
        sources: [
          source("CrewAI introduction", "https://docs.crewai.com/en/introduction"),
          source("CrewAI agents", "https://docs.crewai.com/en/concepts/agents"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "CrewAI remains rated none for built-in big-data throughput because the open-source framework runs in the user's Python/application infrastructure; AMP can deploy and scale crews, but it is not a data-pipeline engine.",
        sources: [
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "CrewAI remains rated none for compiled business logic because crews and flows are Python projects or AMP automations, not compiled portable workflow artifacts.",
        sources: [
          source("CrewAI introduction", "https://docs.crewai.com/en/introduction"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "CrewAI's file-limit cell stays depends because the Files API supports path, URL, and bytes inputs, while practical ceilings come from the runtime, storage, model provider, or AMP deployment path.",
        caveat:
          "CrewAI documents file types and usage, but not one universal product-wide upload ceiling for this comparison row.",
        sources: [
          source("CrewAI files", "https://docs.crewai.com/en/concepts/files"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "CrewAI is now rated partial for replayable execution because CrewAI Flows document stateful event-driven workflows and persistence/resume patterns, but not deterministic event-history replay.",
        sources: [
          source("CrewAI flows", "https://docs.crewai.com/en/concepts/flows"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: {
        summary:
          "CrewAI is now rated partial for visual workflow building because CrewAI AMP includes Crew Studio, described as a no-code/low-code interface for creating and customizing crews, while the open-source framework remains code-first.",
        caveat:
          "This is an AMP production-platform capability, not the default open-source SDK experience.",
        sources: [
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "CrewAI is rated native for file-native agent inputs because CrewAI documents a Files API for images, PDFs, audio, video, text, path/URL/bytes sources, and file use with crews, tasks, flows, and standalone agents.",
        caveat:
          "The file-processing package is documented as early access, so this rating is about agent file input handling rather than a full local document workspace.",
        sources: [
          source("CrewAI files", "https://docs.crewai.com/en/concepts/files"),
          source("CrewAI knowledge", "https://docs.crewai.com/en/concepts/knowledge"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "CrewAI is rated partial for data science because agents, tools, files, knowledge, memory, and flows can coordinate analytical work, but CrewAI is not a notebook, training, or warehouse platform.",
        sources: [
          source("CrewAI knowledge", "https://docs.crewai.com/en/concepts/knowledge"),
          source("CrewAI files", "https://docs.crewai.com/en/concepts/files"),
          source("CrewAI flows", "https://docs.crewai.com/en/concepts/flows"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "CrewAI remains rated none for app UI building because Crew Studio builds and customizes crews; it does not ship arbitrary forms, dashboards, or app screens.",
        sources: [
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "CrewAI remains rated none for full apps because AMP deploys crews as automations/API endpoints, not complete web, desktop, or mobile applications.",
        sources: [
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "CrewAI is rated partial for customer-facing delivery because deployed crews can expose REST API endpoints and public URLs protected by bearer tokens, but customer apps still need to be built around those endpoints.",
        sources: [
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "CrewAI remains rated none for desktop app delivery because docs cover Python projects, AMP deployments, and APIs, not packaging desktop applications.",
        sources: [
          source("CrewAI docs", "https://docs.crewai.com/en/index"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "CrewAI remains rated none for mobile app delivery because mobile clients must be built separately around CrewAI or AMP APIs.",
        sources: [
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "CrewAI is rated partial for offline/local use because the open-source Python framework can run in customer-controlled environments, but deployed AMP crews, model calls, tools, and observability are network-dependent.",
        sources: [
          source("CrewAI installation", "https://docs.crewai.com/en/installation"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "CrewAI is rated partial for local-first architecture because crews can be developed as local Python projects, but production state, APIs, traces, and deployments often live in AMP or external services.",
        sources: [
          source("CrewAI installation", "https://docs.crewai.com/en/installation"),
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "CrewAI is now rated basic for governance because AMP adds managed deployments, environment-variable handling, API tokens, execution history, metrics, traces, logs, and team production workflows, while the open-source framework remains lightweight.",
        caveat:
          "The rating is not enterprise because the reviewed public docs do not expose a broad policy/audit/access-control governance suite comparable to enterprise workflow platforms.",
        sources: [
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "CrewAI is rated native for self-hosting at the framework level because it is an installable Python package that can run in customer-controlled applications, even though AMP is a managed deployment platform.",
        sources: [
          source("CrewAI installation", "https://docs.crewai.com/en/installation"),
          source("CrewAI introduction", "https://docs.crewai.com/en/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "CrewAI is rated low lock-in because open-source crews are Python projects, but AMP deployments, Crew Studio, tool repositories, and execution history add platform-specific migration work.",
        sources: [
          source("CrewAI installation", "https://docs.crewai.com/en/installation"),
          source("CrewAI AMP", "https://docs.crewai.com/enterprise/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "CrewAI remains rated none for sandbox isolation because docs show tools and crews running in Python or AMP deployments, but not a hardened portable sandbox for arbitrary untrusted tool/code execution.",
        sources: [
          source("CrewAI tools", "https://docs.crewai.com/en/concepts/tools"),
          source("Deploy Crew", "https://docs.crewai.com/enterprise/guides/deploy-crew"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "CrewAI is now rated partial for concurrent state because flows document structured state, persistence, and event-driven execution, while transactional concurrent app-state safety still depends on the surrounding storage and app design.",
        sources: [
          source("CrewAI flows", "https://docs.crewai.com/en/concepts/flows"),
          source("CrewAI memory", "https://docs.crewai.com/en/concepts/memory"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  AutoGen: {
    summary:
      "Microsoft AutoGen is a multi-agent conversation framework with customizable agents, tools, humans-in-the-loop, group chats, and code execution patterns. It is an SDK/research-style framework rather than a packaged business app platform.",
    sources: [
      source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
      source("AutoGen teams", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/teams.html"),
      source("AutoGen state", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html"),
      source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
      source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
    ],
    cells: {
      ...codeFrameworkCells(
        "AutoGen",
        "multi-agent conversations, tools, group chats, and code execution patterns",
        [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen teams", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/teams.html"),
          source("AutoGen state", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html"),
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        "AutoGen is rated partial for data science because it can coordinate agents, tools, notebooks, and code execution around analytical tasks, but it is not a data-science platform by itself.",
      ),
      visual_workflow: {
        summary:
          "AutoGen is now rated partial for visual workflow building because AutoGen Studio provides a web UI for prototyping agent workflows, but the core framework remains code-first.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
          source("AutoGen Studio paper", "https://arxiv.org/abs/2408.15247"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "AutoGen is rated partial for replayable execution because agents and teams can save/load state and Studio can test workflows, but the public docs do not describe deterministic event-history replay.",
        sources: [
          source("AutoGen state", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html"),
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "AutoGen is rated native for AI agents because its public docs center on multi-agent conversations, tools, and customizable agent behaviors.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen teams", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/teams.html"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "AutoGen remains rated none for built-in big-data throughput because it is a developer framework; scaling workers, queues, model-provider quotas, and hosting are supplied by the surrounding application.",
        caveat:
          "The distributed runtime and Docker execution patterns help structure applications, but they are not a managed high-throughput data platform.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "AutoGen remains rated none for compiled business logic because agents, teams, GraphFlow workflows, and Studio configurations are Python/JSON runtime definitions rather than compiled portable workflow artifacts.",
        sources: [
          source("AutoGen GraphFlow", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/graph-flow.html"),
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "AutoGen's file-limit cell stays depends because files, multimodal messages, code work directories, and tool artifacts are handled by the application, executor, model provider, or storage layer.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "AutoGen remains rated none for file-native workflows because files are inputs or artifacts for tools/code executors, not first-class local project documents managed by AutoGen itself.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "AutoGen is rated partial for data science because it can coordinate agents, tools, GraphRAG/HTTP/MCP workbenches, notebooks, and Docker/Jupyter-style code execution, but it is not a notebook or ML platform by itself.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "AutoGen remains rated none for app UI building because Studio is for prototyping agent teams and workflows, not for shipping arbitrary forms, dashboards, or business app screens.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "AutoGen remains rated none for full apps because Studio explicitly says it is not production-ready and developers must build their own applications, authentication, security, and deployment around the framework.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "AutoGen remains rated none for customer-facing delivery because the framework can be embedded in a product, but public docs do not provide a supported customer app or hosted deployment surface.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "AutoGen remains rated none for desktop delivery because docs cover Python packages, Studio, and Docker execution, not packaging desktop applications.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "AutoGen remains rated none for mobile delivery because public docs do not describe native mobile app packaging for AutoGen teams or Studio workflows.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "AutoGen is rated partial for offline/local use because the framework and Studio can run locally, but model calls, external tools, MCP servers, and production app services are usually network-dependent.",
        sources: [
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "AutoGen is rated partial for local-first architecture because teams and state can be serialized by the application, but AutoGen does not define end-user offline sync or local-first data ownership semantics.",
        sources: [
          source("AutoGen state", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "AutoGen is rated native for self-hosting because it is an installable open-source developer framework, and Studio can run locally or in a Docker container for prototyping.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "AutoGen is rated low lock-in because teams, agents, tools, and Studio configurations are developer-controlled code/configuration, though AutoGen APIs and component schemas still create migration work.",
        sources: [
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
          source("AutoGen agents", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/agents.html"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "AutoGen is rated partial for sandbox isolation because its docs describe Docker-style code execution patterns, but safe isolation depends on how the application configures executors.",
        sources: [
          source("AutoGen code execution", "https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/design-patterns/code-execution-groupchat.html"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "AutoGen is rated partial for concurrent state because agents, teams, and runtimes can save/load state and coordinate conversations, but docs caution that saving a running team may be inconsistent and transactional app state is external.",
        sources: [
          source("AutoGen state", "https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html"),
          source("AutoGen teams reference", "https://microsoft.github.io/autogen/stable/reference/python/autogen_agentchat.teams.html"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "AutoGen remains rated none for governance because the framework docs do not provide enterprise admin, audit, policy, or access-control layers by default.",
        sources: [
          source("AutoGen agent chat docs", "https://microsoft.github.io/autogen/docs/Use-Cases/agent_chat/"),
          source("AutoGen Studio", "https://microsoft.github.io/autogen/stable/user-guide/autogenstudio-user-guide/index.html"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  LangGraph: {
    summary:
      "LangGraph is LangChain's graph-based framework for stateful agents, durable execution, persistence, interrupts, and long-running workflows. It provides agent orchestration primitives, not a visual app builder or native app distribution layer.",
    sources: [
      source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
      source("LangGraph durable execution", "https://docs.langchain.com/oss/python/langgraph/durable-execution"),
      source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
      source("LangSmith Studio", "https://docs.langchain.com/oss/python/langgraph/studio"),
      source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
      source("LangSmith threads", "https://docs.langchain.com/langsmith/use-threads"),
    ],
    cells: {
      ...codeFrameworkCells(
        "LangGraph",
        "long-running stateful agent graphs",
        [
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
          source("LangGraph durable execution", "https://docs.langchain.com/oss/python/langgraph/durable-execution"),
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
          source("LangSmith Studio", "https://docs.langchain.com/oss/python/langgraph/studio"),
        ],
        "LangGraph is rated partial for data science because it can orchestrate stateful agents, tools, retrieval, and analytical services, but it is not a notebook or ML platform by itself.",
      ),
      visual_workflow: {
        summary:
          "LangGraph is rated partial for visual workflow building because LangSmith Studio provides a visual interface for developing, testing, and debugging agents, while LangGraph itself remains code-defined.",
        sources: [
          source("LangSmith Studio", "https://docs.langchain.com/langsmith/studio"),
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "LangGraph is now rated native for replayable execution because durable execution and checkpoint-backed resumption are core documented capabilities.",
        caveat:
          "This is runtime-level durability for agent graphs, not a packaged business-user replay UI.",
        sources: [
          source("LangGraph durable execution", "https://docs.langchain.com/oss/python/langgraph/durable-execution"),
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
          source("LangSmith time travel", "https://docs.langchain.com/langsmith/human-in-the-loop-time-travel"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "LangGraph is rated native for AI agents because the official overview frames it as a low-level orchestration framework and runtime for long-running, stateful agents.",
        sources: [
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "LangGraph remains rated none for big-data throughput because LangSmith Deployment scales long-running agent workloads horizontally, but the product is not a bulk data-processing or analytical pipeline engine.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
          source("LangSmith Agent Server scale", "https://docs.langchain.com/langsmith/agent-server-scale"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "LangGraph remains rated none for compiled business logic because graph behavior is Python or JavaScript code plus runtime configuration, not compiled portable workflow/application artifacts.",
        sources: [
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "LangGraph's file-limit cell stays depends because Agent Server inputs, artifacts, checkpointer state, stores, and model/tool payloads are determined by the deployed app and backend services rather than one LangGraph-wide upload cap.",
        sources: [
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "LangGraph remains rated none for file-native workflows because files are application inputs, tool artifacts, or store data; LangGraph's first-class object is graph/thread state, not local project files.",
        sources: [
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "LangGraph is rated partial for data science because it orchestrates stateful agents, tools, retrieval, semantic search, and evaluation workflows, but it is not a notebook, warehouse, or model-training platform.",
        sources: [
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "LangGraph remains rated none for app UI building because Studio is an agent IDE/debugging surface and Agent Chat UI is a client pattern, not a general forms/dashboard/app-screen builder.",
        sources: [
          source("LangSmith Studio", "https://docs.langchain.com/langsmith/studio"),
          source("LangGraph Agent Chat UI", "https://docs.langchain.com/oss/python/langgraph/agent-chat-ui"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "LangGraph remains rated none for full apps because LangSmith Deployment ships Agent Server runtimes and APIs; teams still build their own product UI, auth model, and app shell.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
          source("LangSmith streaming API", "https://docs.langchain.com/langsmith/streaming"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "LangGraph is now rated partial for customer-facing delivery because Agent Server provides streaming run APIs, threads, custom authentication, MCP/A2A endpoints, and user-facing agent interaction primitives, but not a complete packaged customer app.",
        sources: [
          source("LangSmith streaming API", "https://docs.langchain.com/langsmith/streaming"),
          source("LangSmith threads", "https://docs.langchain.com/langsmith/use-threads"),
          source("LangSmith custom auth", "https://docs.langchain.com/langsmith/custom-auth"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "LangGraph remains rated none for desktop delivery because docs cover SDKs, Agent Server, Studio, and deployment targets, not desktop app packaging.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "LangGraph remains rated none for mobile delivery because mobile clients must be built separately on top of Agent Server APIs or application backends.",
        sources: [
          source("LangSmith streaming API", "https://docs.langchain.com/langsmith/streaming"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "LangGraph is rated partial for offline/local use because graphs can run locally and standalone Agent Server can be customer-operated, but model calls, stores, tracing, and deployed agent APIs are normally service-dependent.",
        sources: [
          source("LangGraph local server", "https://docs.langchain.com/oss/python/langgraph/local-server"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "LangGraph is rated partial for local-first architecture because developers can use local graphs and custom stores/checkpointers, but LangGraph does not define end-user offline sync or device-local data ownership semantics.",
        sources: [
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
          source("LangSmith Agent Server", "https://docs.langchain.com/langsmith/agent-server"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "LangGraph is rated basic for governance because LangSmith Deployment adds tracing, Studio, custom auth, private conversations, configurable headers, encryption, CI/CD, and deployment controls, but the open framework itself is not a broad enterprise governance suite.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
          source("LangSmith Studio", "https://docs.langchain.com/langsmith/studio"),
          source("LangSmith custom auth", "https://docs.langchain.com/langsmith/custom-auth"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "LangGraph is rated native for self-hosting because LangSmith Deployment documents standalone Agent Server with Docker/Compose/Kubernetes and full self-hosted LangSmith platform deployment options.",
        caveat:
          "Self-hosted LangSmith full platform is an enterprise deployment path; the open-source graph library can also run in customer-controlled apps.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
          source("LangSmith standalone servers", "https://docs.langchain.com/langsmith/deploy-standalone-server"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "LangGraph is rated low lock-in at the framework level because graphs are code and can run locally or in customer infrastructure, although LangSmith Deployment APIs, Studio, traces, and hosted control-plane features add platform-specific migration work.",
        sources: [
          source("LangGraph overview", "https://docs.langchain.com/oss/python/langgraph"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "LangGraph remains rated none for sandbox isolation because public docs cover Agent Server deployment, auth, and state persistence, but do not provide a default hardened sandbox for arbitrary untrusted tool or code execution.",
        sources: [
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
          source("LangSmith custom routes", "https://docs.langchain.com/langsmith/custom-routes"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "LangGraph is rated partial for concurrent state because threads, checkpoints, runs, time travel, and double-texting controls coordinate graph execution state, while application-level transactional business state remains external.",
        sources: [
          source("LangGraph persistence", "https://docs.langchain.com/oss/python/langgraph/persistence"),
          source("LangSmith threads", "https://docs.langchain.com/langsmith/use-threads"),
          source("LangSmith time travel", "https://docs.langchain.com/langsmith/human-in-the-loop-time-travel"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Dify: {
    summary:
      "Dify is an open-source LLM app platform with agents, visual workflows, knowledge bases, tools, and self-host configuration. It is close to an LLM app builder, but public defaults still show document/file limits and an LLM-centric product scope.",
    sources: [
      source("Dify agent docs", "https://docs.dify.ai/en/use-dify/build/agent"),
      source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
      source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
      source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
      source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
      source("Dify workflow web apps", "https://docs.dify.ai/en/use-dify/publish/webapp/workflow-webapp"),
    ],
    cells: {
      ...openWorkflowToolCells("Dify", "LLM apps, agents, knowledge retrieval, and visual workflows", [
        source("Dify agent docs", "https://docs.dify.ai/en/use-dify/build/agent"),
        source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
        source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
        source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
      ]),
      visual_workflow: {
        summary:
          "Dify is rated native for visual workflow building because Dify documents workflow apps with graph-style orchestration nodes for LLM applications.",
        caveat:
          "The workflow builder is LLM-app centered rather than a general-purpose business application runtime.",
        sources: [
          source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Dify is rated partial for replayability because workflow run history creates complete log entries with inputs, outputs, and metadata, but docs do not describe deterministic event-history replay.",
        sources: [
          source("Dify run history", "https://docs.dify.ai/en/use-dify/debug/history-and-logs"),
          source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Dify remains rated none for big-data throughput because environment docs list execution steps, run time, file count, variable-size, parallel-branch, and worker-pool limits rather than unbounded data-pipeline capacity.",
        sources: [
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Dify remains rated none for compiled business logic because workflows are graph/node configuration and exported app DSL, not compiled portable application logic.",
        sources: [
          source("Dify orchestration logic", "https://docs.dify.ai/en/use-dify/build/orchestrate-node"),
          source("Dify publish to marketplace", "https://docs.dify.ai/en/use-dify/publish/publish-to-marketplace"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Dify is rated 15 MB because its self-host environment variables document `UPLOAD_FILE_SIZE_LIMIT` with a default of 15 MB for document uploads.",
        evidence:
          "Dify separately documents lower image attachment limits and higher media limits, so this cell uses the document-upload default relevant to knowledge/RAG workflows.",
        sources: [
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Dify remains rated none for file-native workflows because files are uploaded documents, generated files, or knowledge-base assets served through Dify storage URLs, not local-first project artifacts.",
        sources: [
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Dify is rated partial for data-science workflows because workflows can process files, call tools, use knowledge bases, and run batch workflow web apps, but Dify is not a notebook or ML training platform.",
        sources: [
          source("Dify workflow web apps", "https://docs.dify.ai/en/use-dify/publish/webapp/workflow-webapp"),
          source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Dify is rated native for AI agents because agent applications, tools, knowledge retrieval, and orchestration are first-party documented Dify features.",
        sources: [
          source("Dify agent docs", "https://docs.dify.ai/en/use-dify/build/agent"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Dify is rated partial for UI building because published web apps create generated interfaces for chat, workflow forms, saved results, and embeds, but Dify does not provide a general arbitrary app-screen builder.",
        sources: [
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
          source("Dify workflow web apps", "https://docs.dify.ai/en/use-dify/publish/webapp/workflow-webapp"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Dify remains rated none for full apps because web apps and marketplace templates expose Dify LLM apps and workflows, not complete portable business, desktop, or mobile applications.",
        sources: [
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
          source("Dify publish to marketplace", "https://docs.dify.ai/en/use-dify/publish/publish-to-marketplace"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Dify is rated partial for customer-facing delivery because Dify can publish web apps, API endpoints, iframes, chat bubbles, and JavaScript embeds, but this remains Dify-app delivery rather than full external app distribution.",
        sources: [
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
          source("Dify web app embedding", "https://docs.dify.ai/en/use-dify/publish/webapp/embedding-in-websites"),
          source("Dify web app access", "https://docs.dify.ai/en/use-dify/publish/webapp/web-app-access"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Dify remains rated none for desktop delivery because publishing docs cover web apps, APIs, embeds, and integrations, not desktop application packaging.",
        sources: [
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Dify remains rated none for mobile delivery because published web apps are responsive web experiences, not generated native mobile applications.",
        sources: [
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Dify remains rated none for offline end-user execution because web apps, workflow runs, model calls, tools, and storage operate through the Dify service and connected providers.",
        sources: [
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
          source("Dify publishing overview", "https://docs.dify.ai/en/use-dify/publish/README"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Dify remains rated none for local-first architecture because workflow/app state, files, execution logs, and published apps are server-centered even when Dify is self-hosted.",
        sources: [
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Dify is rated native for self-hosting because the official docs include self-host installation and configuration paths.",
        sources: [
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Dify is rated partial for sandbox isolation because it supports hosted/self-hosted execution controls, but public docs do not establish a portable hardened sandbox for arbitrary untrusted automation.",
        sources: [
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
          source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Dify is rated basic for governance because docs show web-app access permissions, workspace roles for publishing, self-host configuration, logs, and security-related environment controls, but not a broad enterprise workflow governance suite.",
        sources: [
          source("Dify web app access", "https://docs.dify.ai/en/use-dify/publish/webapp/web-app-access"),
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
          source("Dify environment variables", "https://docs.dify.ai/en/self-host/configuration/environments"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Dify is rated low lock-in because it is open source, self-hostable, and can export app DSL/templates, but app behavior remains tied to Dify workflow nodes, plugins, datasets, and runtime semantics.",
        sources: [
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
          source("Dify publish to marketplace", "https://docs.dify.ai/en/use-dify/publish/publish-to-marketplace"),
          source("Dify orchestration logic", "https://docs.dify.ai/en/use-dify/build/orchestrate-node"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Dify is now rated partial for concurrent state because it can persist and run hosted/self-hosted workflow and conversation state, but public docs do not document transactional concurrent-write semantics for arbitrary app data.",
        sources: [
          source("Dify workflow docs", "https://docs.dify.ai/en/use-dify/build/workflow"),
          source("Dify self-host deployment", "https://docs.dify.ai/en/getting-started/install-self-hosted"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Langdock: {
    summary:
      "Langdock is an enterprise AI workspace with agents, visual workflows, knowledge, actions, mobile/PWA access, governance, and managed or dedicated deployment options. It is strong for company AI rollout, but still platform-centered.",
    sources: [
      source("Langdock feature overview", "https://docs.langdock.com/resources/feature-overview"),
      source("Langdock workflows introduction", "https://docs.langdock.com/product/workflows/introduction"),
      source("Langdock supported file types", "https://docs.langdock.com/resources/faq/supported-file-types"),
      source("Langdock action file support", "https://docs.langdock.com/resources/integrations/file-support-for-actions"),
    ],
    cells: {
      replayable: note(
        "Langdock is rated partial for replayable execution because workflow execution and monitoring are documented, but Langdock's workflow materials do not describe deterministic event-history replay or state reconstruction.",
        [
          source("Langdock workflows introduction", "https://docs.langdock.com/product/workflows/introduction"),
          source("Langdock workflows product page", "https://www.langdock.com/products/workflows"),
        ],
      ),
      high_volume: note(
        "Langdock is rated none for big-data throughput because its documented product scope is AI workflows, agents, file uploads, and enterprise rollout rather than high-throughput data pipelines or batch processing.",
        [
          source("Langdock feature overview", "https://docs.langdock.com/resources/feature-overview"),
          source("Supported file types", "https://docs.langdock.com/resources/faq/supported-file-types"),
        ],
      ),
      compiled: note(
        "Langdock is rated none for compiled business logic because workflows and custom actions are platform configuration or sandboxed code snippets, not portable compiled workflow/application logic.",
        [
          source("Langdock workflows introduction", "https://docs.langdock.com/product/workflows/introduction"),
          source("File support for actions", "https://docs.langdock.com/resources/integrations/file-support-for-actions"),
        ],
      ),
      file_native: note(
        "Langdock is rated none for file-native workflows because files are uploaded into chats, knowledge, and actions rather than managed as local-first project artifacts.",
        [
          source("Supported file types", "https://docs.langdock.com/resources/faq/supported-file-types"),
          source("File support for actions", "https://docs.langdock.com/resources/integrations/file-support-for-actions"),
        ],
      ),
      data_science: note(
        "Langdock is rated partial for data-science workflows because agents and workflows can use files, knowledge, actions, and connected apps, but Langdock is not a notebook, training, or analytical pipeline platform.",
        [
          source("Langdock feature overview", "https://docs.langdock.com/resources/feature-overview"),
          source("Langdock workflows introduction", "https://docs.langdock.com/product/workflows/introduction"),
        ],
      ),
    },
  },
  Flowise: {
    summary:
      "Flowise is an open-source low-code platform for chatflows, agentflows, RAG, custom code, and self-hosted LLM workflows. It is visual and developer-friendly, but mainly ships chat/agent endpoints rather than complete governed apps.",
    sources: [
      source("Flowise docs", "https://docs.flowiseai.com/"),
      source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
      source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
      source("Flowise Docker deployment", "https://docs.flowiseai.com/configuration/deployment/docker"),
      source("Flowise embed", "https://docs.flowiseai.com/using-flowise/embed"),
      source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
    ],
    cells: {
      ...openWorkflowToolCells("Flowise", "chatflows, agentflows, RAG pipelines, custom code, and self-hosted LLM workflows", [
        source("Flowise docs", "https://docs.flowiseai.com/"),
        source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
        source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
        source("Flowise Docker deployment", "https://docs.flowiseai.com/configuration/deployment/docker"),
      ]),
      visual_workflow: {
        summary:
          "Flowise is rated native for visual workflow building because chatflows and agentflows are built through Flowise's visual low-code canvas.",
        sources: [
          source("Flowise docs", "https://docs.flowiseai.com/"),
          source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Flowise remains rated none for replayable execution because public docs expose chat messages, prediction APIs, and flow definitions, but not deterministic workflow replay or event-history reconstruction.",
        caveat:
          "Operators can inspect chat history and rerun calls manually; that is not replayable execution semantics.",
        sources: [
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
          source("Flowise Chatflows API", "https://docs.flowiseai.com/api-reference/chatflows"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Flowise remains rated none for big-data throughput because its docs focus on chatflows, agentflows, uploads, APIs, and deployment configuration rather than high-throughput data pipelines.",
        sources: [
          source("Flowise deployment", "https://docs.flowiseai.com/configuration/deployment"),
          source("Flowise rate limit", "https://docs.flowiseai.com/configuration/rate-limit"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Flowise remains rated none for compiled business logic because flows are visual JSON/configuration and node execution, not compiled portable application logic.",
        sources: [
          source("Flowise Chatflows API", "https://docs.flowiseai.com/api-reference/chatflows"),
          source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Flowise is rated 50 MB because its environment-variable docs list `FLOWISE_FILE_SIZE_LIMIT` with a default value of `50mb` for uploads.",
        caveat:
          "Self-hosted deployments can change this setting, and reverse proxies/body-parser settings may still impose their own limits.",
        sources: [
          source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Flowise remains rated none for file-native workflows because uploads are chatflow inputs or document-store/vector-store data, not local-first project files managed by the runtime.",
        sources: [
          source("Flowise uploads", "https://docs.flowiseai.com/using-flowise/uploads"),
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Flowise remains rated none for data-science workflows because it can build RAG/chat/agent pipelines, but it is not a notebook, model-training, or analytical compute platform.",
        sources: [
          source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Flowise is rated native for AI agents because Agentflows are a first-party documented Flowise capability for multi-step agent orchestration.",
        sources: [
          source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Flowise is rated partial for UI building because it provides an embeddable, customizable chat widget and flow UI, but not a general forms/dashboard/app-screen builder.",
        sources: [
          source("Flowise embed", "https://docs.flowiseai.com/using-flowise/embed"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Flowise remains rated none for full apps because it exposes chatflows, agentflows, APIs, and embeddable widgets rather than complete desktop, mobile, or business applications.",
        sources: [
          source("Flowise embed", "https://docs.flowiseai.com/using-flowise/embed"),
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Flowise is rated partial for customer-facing delivery because docs show public or API-key protected chatflows and an embeddable website chat widget, but this is chatbot delivery rather than a full customer app platform.",
        sources: [
          source("Flowise embed", "https://docs.flowiseai.com/using-flowise/embed"),
          source("Flowise chatflow access control", "https://docs.flowiseai.com/configuration/authorization/chatflow-level"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Flowise remains rated none for desktop app delivery because official deployment and API docs do not describe packaging Flowise-built flows as desktop applications.",
        sources: [
          source("Flowise deployment", "https://docs.flowiseai.com/configuration/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Flowise remains rated none for mobile app delivery because public docs do not describe native mobile packaging for chatflows or agentflows.",
        sources: [
          source("Flowise embed", "https://docs.flowiseai.com/using-flowise/embed"),
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Flowise remains rated none for offline end-user execution because flows run through a server/API and usually depend on model providers, vector stores, and connected tools.",
        sources: [
          source("Flowise deployment", "https://docs.flowiseai.com/configuration/deployment"),
          source("Flowise databases", "https://docs.flowiseai.com/configuration/databases"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Flowise is rated partial for local-first control because it can run locally or self-hosted with SQLite/Postgres/MySQL, but published chatflows and state are still server-centered rather than device-local sync data.",
        sources: [
          source("Flowise Docker deployment", "https://docs.flowiseai.com/configuration/deployment/docker"),
          source("Flowise databases", "https://docs.flowiseai.com/configuration/databases"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Flowise is rated native for self-hosting because official docs include Docker deployment and environment-variable configuration.",
        sources: [
          source("Flowise Docker deployment", "https://docs.flowiseai.com/configuration/deployment/docker"),
          source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Flowise is rated basic for governance because docs show application auth, chatflow-level API-key access control, workspaces/RBAC for enterprise/self-hosted enterprise, and SSO, but not a broad workflow governance suite by default.",
        sources: [
          source("Flowise app auth", "https://docs.flowiseai.com/configuration/authorization/application"),
          source("Flowise chatflow access control", "https://docs.flowiseai.com/configuration/authorization/chatflow-level"),
          source("Flowise workspaces", "https://docs.flowiseai.com/using-flowise/workspaces"),
          source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Flowise is rated low lock-in because it is open source, deployable across local/cloud/Kubernetes environments, and exposes REST APIs, though flow JSON and node choices still create migration work.",
        sources: [
          source("Flowise deployment", "https://docs.flowiseai.com/configuration/deployment"),
          source("Flowise API reference", "https://docs.flowiseai.com/api-reference"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Flowise remains rated none for sandbox isolation because public docs describe deployment, auth, and node execution settings, but not a hardened portable sandbox for arbitrary untrusted code or tools.",
        sources: [
          source("Flowise environment variables", "https://docs.flowiseai.com/configuration/environment-variables"),
          source("Flowise deployment", "https://docs.flowiseai.com/configuration/deployment"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Flowise is rated partial for concurrent state because it can run hosted/self-hosted flows with persisted chat/session context, but public docs do not state transactional concurrent workflow state semantics.",
        sources: [
          source("Flowise agentflows", "https://docs.flowiseai.com/using-flowise/agentflows"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Pydantic AI": {
    summary:
      "Pydantic AI is a type-safe Python agent framework with tools, dependency injection, structured outputs, and testing/evals ergonomics. It is a code library, not a visual builder, hosted runtime, or governance suite.",
    sources: [
      source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
      source("Pydantic AI agents", "https://pydantic.dev/docs/ai/core-concepts/agent/"),
      source("Pydantic AI durable execution", "https://pydantic.dev/docs/ai/durable-execution/"),
      source("Pydantic AI native tools", "https://pydantic.dev/docs/ai/tools-toolsets/native-tools/"),
    ],
    cells: {
      ...codeFrameworkCells(
        "Pydantic AI",
        "type-safe Python agents",
        [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
          source("Pydantic AI agents", "https://pydantic.dev/docs/ai/core-concepts/agent/"),
          source("Pydantic AI durable execution", "https://pydantic.dev/docs/ai/durable-execution/"),
        ],
        "Pydantic AI is rated partial for data science because it supports structured outputs, tool use, and typed agent apps, but it is not a data-science platform or notebook/runtime by itself.",
      ),
      ai_agents: {
        summary:
          "Pydantic AI is rated native for AI agents because agents, tools, dependency injection, structured outputs, and type-safe model interactions are core documented concepts.",
        sources: [
          source("Pydantic AI agents", "https://pydantic.dev/docs/ai/core-concepts/agent/"),
          source("Pydantic AI tools", "https://pydantic.dev/docs/ai/tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Pydantic AI remains rated none for built-in big-data throughput because it is a Python framework; concurrency, workers, queues, storage, and model-provider rate limits are supplied by the surrounding application.",
        evidence:
          "The testing docs show ordinary Python async execution, and the model docs describe provider configuration rather than a managed high-throughput runtime.",
        sources: [
          source("Pydantic AI testing", "https://pydantic.dev/docs/ai/guides/testing/"),
          source("Pydantic AI models overview", "https://pydantic.dev/docs/ai/models/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Pydantic AI remains rated none for compiled business logic because agents are defined in Python code, YAML/JSON specs, model settings, and tool functions rather than compiled portable workflow artifacts.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Pydantic AI's file-limit cell stays depends because file search, retrieval, artifacts, and uploads are handled by the chosen model provider, vector store, storage backend, or application code.",
        caveat:
          "Native FileSearchTool support is provider-backed; it is not one Pydantic-wide file-size ceiling.",
        sources: [
          source("Pydantic AI native tools", "https://pydantic.dev/docs/ai/tools-toolsets/native-tools/"),
          source("Pydantic AI models overview", "https://pydantic.dev/docs/ai/models/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Pydantic AI remains rated none for file-native workflows because files are inputs to tools, model-provider file search, or application storage, not first-class local project artifacts managed by the runtime.",
        sources: [
          source("Pydantic AI native tools", "https://pydantic.dev/docs/ai/tools-toolsets/native-tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Pydantic AI is rated partial for data science because it supports structured outputs, provider code-execution tools, evals, and typed agent workflows, but it is not a notebook, warehouse, or ML pipeline platform.",
        sources: [
          source("Pydantic AI evals", "https://pydantic.dev/docs/ai/evals/evals/"),
          source("Pydantic AI native tools", "https://pydantic.dev/docs/ai/tools-toolsets/native-tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Pydantic AI is rated partial for replayable execution because it documents durable execution integrations, but those guarantees depend on the chosen external durability backend.",
        sources: [
          source("Pydantic AI durable execution", "https://pydantic.dev/docs/ai/integrations/durable_execution/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      visual_workflow: {
        summary:
          "Pydantic AI is rated none for visual workflow building because it is a code-first Python framework; YAML/JSON agent specs are configuration, not a drag-and-drop workflow canvas.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Pydantic AI remains rated none for UI building because its UI support is event-stream and protocol integration for applications, not a forms/dashboard/page builder.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Pydantic AI remains rated none for full apps because it is embedded inside an application stack; packaging web, desktop, or mobile apps is left to the product team.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Pydantic AI remains rated none for customer-facing delivery because auth, tenant isolation, UI, hosting, and operations must be built outside the framework.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Pydantic AI remains rated none for desktop app delivery because it does not package desktop applications; it can only be used inside separately built desktop software.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Pydantic AI remains rated none for mobile app delivery because it is a Python agent framework, not a mobile runtime or app-packaging system.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Pydantic AI is rated partial for offline use because the framework can run in customer-controlled Python environments, but most model providers, hosted tracing, retrieval, and native tools are network-dependent unless replaced with local services.",
        sources: [
          source("Pydantic AI models overview", "https://pydantic.dev/docs/ai/models/overview/"),
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Pydantic AI is rated partial for local-first architecture because developers can run the framework locally and choose local models/storage, but the framework does not define an offline sync or local data ownership model.",
        sources: [
          source("Pydantic AI models overview", "https://pydantic.dev/docs/ai/models/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Pydantic AI is rated none for governance because the framework does not include enterprise admin, audit, policy, or access-control surfaces by default.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Pydantic AI is rated native for self-hosting because it is an installable Python framework that runs inside customer-controlled applications and infrastructure.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Pydantic AI is rated low lock-in because it is model-agnostic and documents many model providers, but concrete apps can still become tied to provider-specific native tools or storage choices.",
        sources: [
          source("Pydantic AI overview", "https://pydantic.dev/docs/ai/overview/"),
          source("Pydantic AI models overview", "https://pydantic.dev/docs/ai/models/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Pydantic AI is now rated partial for sandbox isolation because it supports provider-native CodeExecutionTool in secure provider environments, but the framework itself does not sandbox arbitrary local tool code.",
        caveat:
          "Provider support varies, and common tools still execute wherever the application runs.",
        sources: [
          source("Pydantic AI native tools", "https://pydantic.dev/docs/ai/tools-toolsets/native-tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Pydantic AI is now rated partial for concurrent state because durable execution integrations are officially documented for Temporal, DBOS, Prefect, and Restate, but state safety depends on the chosen backend.",
        caveat:
          "Without one of those durability systems, transactional multi-user state remains an application responsibility.",
        sources: [
          source("Pydantic AI durable execution", "https://pydantic.dev/docs/ai/integrations/durable_execution/overview/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Agno: {
    summary:
      "Agno is a Python framework for agents and agent teams with tools, memory, storage, knowledge, reasoning, and multi-agent patterns. It is developer-first infrastructure rather than a complete app delivery platform.",
    sources: [
      source("Agno agents", "https://docs.agno.com/agents"),
      source("Agno reasoning", "https://docs.agno.com/agents/reasoning"),
      source("Agno teams", "https://docs.agno.com/teams"),
      source("Agno storage", "https://docs.agno.com/features/storage"),
      source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
    ],
    cells: {
      ...codeFrameworkCells(
        "Agno",
        "Python agent teams and memory-backed agents",
        [
          source("Agno agents", "https://docs.agno.com/agents"),
          source("Agno teams", "https://docs.agno.com/teams"),
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        "Agno is rated partial for data science because it can connect agents to knowledge, tools, and data workflows, but it is not a dedicated analytical runtime.",
      ),
      visual_workflow: {
        summary:
          "Agno is now rated partial for visual workflow building because AgentOS Studio documents a visual Team Builder with drag-and-drop agents and coordination modes, while general workflows remain developer-defined.",
        caveat:
          "This is visual agent/team composition, not a broad no-code business-process canvas.",
        sources: [
          source("Agno Studio Teams", "https://docs.agno.com/agent-os/studio/teams"),
          source("Agno introduction", "https://docs.agno.com/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Agno is rated native for AI agents because agents, teams, tools, memory, storage, reasoning, and multi-agent patterns are first-class documented concepts.",
        sources: [
          source("Agno agents", "https://docs.agno.com/agents"),
          source("Agno teams", "https://docs.agno.com/teams"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Agno remains rated none for compiled business logic because agents, teams, and workflows are Python/runtime constructs served through AgentOS rather than compiled portable workflow artifacts.",
        sources: [
          source("Agno introduction", "https://docs.agno.com/introduction"),
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Agno's file-limit cell stays depends because docs say large media and PDFs should live in object storage and be referenced from sessions or knowledge, not stored under one Agno-wide upload limit.",
        sources: [
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Agno remains rated none for file-native workflows because files are referenced through knowledge, sessions, object storage, or tools rather than owned as local project artifacts by the runtime.",
        sources: [
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Agno is rated partial for data science because docs describe data labeling, extraction, classification, knowledge, traces, evals, and database-backed agent workflows, but Agno is not a notebook or analytical warehouse.",
        sources: [
          source("Agno home", "https://docs.agno.com/"),
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Agno is rated partial for replayability because storage and session persistence are documented, but deterministic workflow replay is not presented as a core guarantee.",
        sources: [
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Agno is rated partial for high-volume operation because it is a lightweight framework intended for scalable agent apps, but scaling depends on the surrounding service architecture.",
        sources: [
          source("Agno introduction", "https://docs.agno.com/introduction"),
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Agno remains rated none for end-user UI building because AgentOS is a runtime/control-plane and Studio builder for agents/teams, not a product for shipping custom app screens.",
        sources: [
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
          source("Agno Studio Teams", "https://docs.agno.com/agent-os/studio/teams"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Agno remains rated none for full apps because it productionizes agents behind APIs, interfaces, and AgentOS rather than packaging complete web, desktop, or mobile applications.",
        sources: [
          source("Agno production overview", "https://docs.agno.com/production/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Agno remains rated none for customer-facing app delivery because docs show exposing agents via APIs and interfaces such as Slack, Discord, MCP, or custom UI, but not a complete external app platform.",
        sources: [
          source("Agno production overview", "https://docs.agno.com/production/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Agno remains rated none for desktop delivery because public docs cover SDK, AgentOS, APIs, and hosted/local services, not desktop app packaging.",
        sources: [
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Agno remains rated none for mobile delivery because the platform does not document native mobile app packaging for Agno-built agents.",
        sources: [
          source("Agno production overview", "https://docs.agno.com/production/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Agno is rated partial for offline/local use because AgentOS can run locally with local databases, but production agent behavior normally depends on model providers, tools, and connected services.",
        sources: [
          source("Agno first agent", "https://docs.agno.com/first-agent"),
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Agno is rated partial for local-first architecture because docs emphasize data ownership in your database and local AgentOS connections, but not end-user offline sync semantics.",
        sources: [
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
          source("Agno storage", "https://docs.agno.com/features/storage"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Agno is now rated basic for governance because AgentOS docs describe RBAC, JWT scopes, request isolation, traces, guardrails, human approvals, and security controls, but not a full enterprise workflow governance suite.",
        sources: [
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
          source("Agno AgentOS security", "https://docs.agno.com/agent-os/security/overview"),
          source("Agno RBAC", "https://docs.agno.com/agent-os/security/rbac"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Agno is rated native for self-hosting because production docs describe deploying AgentOS via Docker, Railway, or AWS in the user's own infrastructure.",
        sources: [
          source("Agno production overview", "https://docs.agno.com/production/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Agno is rated low lock-in because AgentOS stores sessions, memory, knowledge, traces, schedules, approvals, metrics, and evals in user-selected databases, though Agno APIs and schemas still create migration work.",
        sources: [
          source("Agno storage", "https://docs.agno.com/features/storage"),
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Agno is now rated partial for sandbox isolation because security docs call out layered request isolation and sandboxes for tools, but public docs do not define a single hardened default sandbox for all arbitrary code.",
        sources: [
          source("Agno security and auth", "https://docs.agno.com/runtime/security-and-auth"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Agno is now rated partial for concurrent state because AgentOS stores sessions, memory, traces, schedules, approvals, and metrics in databases with request isolation, while transactional app-state correctness still depends on the chosen backend and app design.",
        sources: [
          source("Agno storage", "https://docs.agno.com/features/storage"),
          source("Agno AgentOS", "https://docs.agno.com/agent-os/introduction"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Google ADK": {
    summary:
      "Google Agent Development Kit is a modular agent framework with LLM agents, workflow agents, graph workflows, tooling, evaluation, and Google Cloud deployment paths. It is strongest for teams already comfortable with code and Google Cloud.",
    sources: [
      source("ADK agents", "https://google.github.io/adk-docs/agents/"),
      source("ADK overview", "https://adk.dev/"),
      source("ADK sessions", "https://google.github.io/adk-docs/sessions/"),
      source("ADK deploy", "https://google.github.io/adk-docs/deploy/"),
      source("ADK Visual Builder", "https://adk.dev/visual-builder/"),
    ],
    cells: {
      ...codeFrameworkCells(
        "Google ADK",
        "code-defined agents and workflow agents",
        [
          source("ADK agents", "https://google.github.io/adk-docs/agents/"),
          source("ADK sessions", "https://google.github.io/adk-docs/sessions/"),
          source("ADK deploy", "https://google.github.io/adk-docs/deploy/"),
        ],
        "Google ADK is rated partial for data science because it can build data-aware agents on Google Cloud or local services, but it is not a data-science platform by itself.",
      ),
      visual_workflow: {
        summary:
          "Google ADK is now rated partial for visual workflow building because ADK Visual Builder provides an experimental drag-and-drop interface for agents, tools, callbacks, sequential, loop, and parallel workflow agents.",
        caveat:
          "Visual Builder is marked experimental and generates ADK agent config/code, so it is not yet equivalent to a mature no-code workflow product.",
        sources: [
          source("ADK Visual Builder", "https://adk.dev/visual-builder/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Google ADK is rated native for AI agents because agents, workflow agents, tools, sessions, memory, evaluation, and deployment are core documented ADK concepts.",
        sources: [
          source("ADK agents", "https://google.github.io/adk-docs/agents/"),
          source("ADK tools", "https://google.github.io/adk-docs/tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Google ADK is rated partial for high-volume operation because docs describe scalable deployment through Agent Runtime, Cloud Run, GKE, and Google-managed infrastructure, while throughput depends on the selected hosting and model services.",
        sources: [
          source("ADK deploy", "https://adk.dev/deploy/"),
          source("ADK home", "https://adk.dev/"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Google ADK remains rated none for compiled business logic because agent behavior is defined in SDK code, graph workflows, and generated config rather than compiled portable business-process artifacts.",
        sources: [
          source("ADK technical overview", "https://adk.dev/get-started/about/"),
          source("ADK Visual Builder", "https://adk.dev/visual-builder/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Google ADK's file-limit cell stays depends because artifacts use pluggable artifact services, while Agent Runtime code execution separately documents 100 MB data-file uploads for that sandboxed tool path.",
        caveat:
          "There is no single ADK-wide upload ceiling across artifact services, tool calls, model providers, and deployment targets.",
        sources: [
          source("ADK artifacts", "https://adk.dev/artifacts/"),
          source("ADK Agent Runtime Code Execution", "https://adk.dev/integrations/code-exec-agent-engine/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "Google ADK remains rated none for file-native workflows because artifacts are session/user-scoped binary objects managed by artifact services, not local project files owned by the runtime.",
        sources: [
          source("ADK artifacts", "https://adk.dev/artifacts/"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "Google ADK is rated partial for data science because it supports code execution, artifacts, evaluation, and data-aware tools, but it is not a notebook, warehouse, or ML pipeline platform.",
        sources: [
          source("ADK evaluation", "https://adk.dev/evaluate/"),
          source("ADK Agent Runtime Code Execution", "https://adk.dev/integrations/code-exec-agent-engine/"),
          source("ADK artifacts", "https://adk.dev/artifacts/"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: {
        summary:
          "Google ADK is rated partial for replayability because sessions and state are documented, but ADK does not present deterministic event-history replay as a native runtime guarantee.",
        sources: [
          source("ADK sessions", "https://adk.dev/sessions/"),
          source("ADK evaluation", "https://adk.dev/evaluate/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Google ADK remains rated none for app UI building because ADK Web and Visual Builder are developer tools for agents, not a product for shipping custom forms, dashboards, or customer app screens.",
        sources: [
          source("ADK Web Interface", "https://adk.dev/runtime/web-interface/"),
          source("ADK Visual Builder", "https://adk.dev/visual-builder/"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Google ADK remains rated none for full apps because it builds and deploys agents; full web, mobile, or desktop applications must be built around the ADK runtime separately.",
        sources: [
          source("ADK deploy", "https://adk.dev/deploy/"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Google ADK remains rated none for customer-facing app delivery because docs cover agent APIs, runtime, and deployment rather than packaged customer portals or end-user applications.",
        sources: [
          source("ADK deploy", "https://adk.dev/deploy/"),
          source("ADK Web Interface", "https://adk.dev/runtime/web-interface/"),
        ],
        checkedAt: "2026-05-30",
      },
      desktop: {
        summary:
          "Google ADK remains rated none for desktop delivery because the framework supports agent runtimes and deployment targets, not desktop app packaging.",
        sources: [
          source("ADK home", "https://adk.dev/"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "Google ADK remains rated none for mobile delivery because ADK provides SDKs and runtimes for agents, not native mobile application packaging.",
        sources: [
          source("ADK home", "https://adk.dev/"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: {
        summary:
          "Google ADK remains rated none for offline end-user execution because agent runs generally depend on model services, tools, and deployed runtime services even when development starts locally.",
        sources: [
          source("ADK models FAQ", "https://adk.dev/"),
          source("ADK deploy", "https://adk.dev/deploy/"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Google ADK remains rated none for local-first architecture because docs focus on session services, memory services, artifacts, and deployment rather than offline-first client data ownership and sync.",
        sources: [
          source("ADK sessions", "https://adk.dev/sessions/"),
          source("ADK artifacts", "https://adk.dev/artifacts/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Google ADK is now rated native for self-hosting because it is an open-source framework that can run locally or be deployed by the developer, even though Google Cloud deployment paths are strongly documented.",
        caveat:
          "The agent framework can run outside Google Cloud, but many examples and managed deployment paths are Google-centric.",
        sources: [
          source("ADK overview", "https://adk.dev/"),
          source("ADK deploy", "https://google.github.io/adk-docs/deploy/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Google ADK is now rated low lock-in because the framework is open-source and developer-run, although Google Cloud and Gemini integrations remain the best-supported path.",
        sources: [
          source("ADK overview", "https://adk.dev/"),
          source("ADK deploy", "https://google.github.io/adk-docs/deploy/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Google ADK remains partial for governance because safety docs cover identity, user auth, guardrails, evaluations, tracing, VPC-SC, and managed Google Cloud controls, but the open framework itself is not a broad business governance suite.",
        sources: [
          source("ADK safety", "https://adk.dev/safety/"),
          source("ADK deploy", "https://adk.dev/deploy/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Google ADK is now rated partial for sandbox isolation because ADK documents sandboxed code-execution options through Agent Runtime and GKE, but they require specific Google Cloud setup instead of being a default local boundary.",
        evidence:
          "The Agent Runtime code execution tool creates sandbox environments and the GKE code executor can use gVisor-backed sandbox mode.",
        sources: [
          source("ADK safety", "https://adk.dev/safety/"),
          source("ADK Agent Runtime Code Execution", "https://adk.dev/integrations/code-exec-agent-engine/"),
          source("ADK GKE Code Executor", "https://adk.dev/integrations/gke-code-executor/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Google ADK is now rated partial for concurrent state because sessions, state, memory services, events, and artifact services are explicit primitives, but transactional multi-user app-state safety depends on the selected services and deployment.",
        sources: [
          source("ADK sessions", "https://adk.dev/sessions/"),
          source("ADK artifacts", "https://adk.dev/artifacts/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  LangChain: {
    summary:
      "LangChain is an open-source framework for agents, tools, model integrations, retrieval, middleware, and LangSmith observability. It is a broad SDK ecosystem, not an app runtime with UI, offline, or governance built in by default.",
    sources: [
      source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
      source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
      source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
      source("LangSmith observability", "https://docs.langchain.com/langsmith/observability"),
      source("LangChain product suite", "https://www.langchain.com/"),
    ],
    cells: {
      ...codeFrameworkCells(
        "LangChain",
        "agent harnesses, tools, middleware, and model integrations",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
          source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
          source("LangChain product suite", "https://www.langchain.com/"),
        ],
        "LangChain is rated partial for data science because it supports retrieval, tools, and data-aware LLM apps, but analytics/storage/execution depend on external services and application code.",
      ),
      visual_workflow: note(
        "LangChain remains rated none for visual workflow building because current docs describe code-defined agents, middleware, and frontend patterns rather than a drag-and-drop workflow canvas.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
      ),
      high_volume: note(
        "LangChain remains rated none for built-in big-data throughput because scaling agent workloads, queues, storage, and model-provider limits is handled by LangGraph/LangSmith deployment or the surrounding application.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
      ),
      compiled: note(
        "LangChain remains rated none for compiled business logic because agents are Python/JavaScript code and runtime configuration, not compiled portable workflow artifacts.",
        [
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
      ),
      file_native: note(
        "LangChain remains rated none for file-native workflows because documents, vector stores, tools, and middleware can use files, but files are application inputs rather than first-class local project artifacts managed by LangChain.",
        [
          source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
      ),
      ui_builder: note(
        "LangChain remains rated none for app UI building because the frontend docs provide streaming hooks and UI patterns, not a no-code screen/form/dashboard builder.",
        [
          source("LangChain frontend overview", "https://docs.langchain.com/oss/python/langchain/frontend/overview"),
          source("Agent Chat UI", "https://docs.langchain.com/oss/python/langchain/ui"),
        ],
      ),
      full_apps: note(
        "LangChain remains rated none for full apps because Agent Chat UI and frontend patterns help developers build chat experiences, but teams still own the product shell, data model, auth, hosting, and distribution.",
        [
          source("Agent Chat UI", "https://docs.langchain.com/oss/python/langchain/ui"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
      ),
      customer_facing: note(
        "LangChain is now rated partial for customer-facing delivery because Agent Chat UI and frontend streaming patterns can connect to local or deployed agents, but they are developer templates rather than a complete customer-app platform.",
        [
          source("Agent Chat UI", "https://docs.langchain.com/oss/python/langchain/ui"),
          source("LangChain frontend overview", "https://docs.langchain.com/oss/python/langchain/frontend/overview"),
        ],
      ),
      desktop: note(
        "LangChain remains rated none for desktop delivery because public docs cover libraries, agent backends, and web/frontend integrations, not desktop app packaging.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain frontend overview", "https://docs.langchain.com/oss/python/langchain/frontend/overview"),
        ],
      ),
      mobile: note(
        "LangChain remains rated none for mobile delivery because mobile clients must be built separately on top of the agent or streaming APIs.",
        [
          source("LangChain frontend overview", "https://docs.langchain.com/oss/python/langchain/frontend/overview"),
        ],
      ),
      offline: note(
        "LangChain is rated partial for offline/local use because the framework can run in local applications, but model calls, tracing, hosted deployments, and external retrieval services are commonly network-dependent.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
      ),
      local_first: note(
        "LangChain is rated partial for local-first architecture because developers can keep code, stores, and some model choices local, but end-user sync, device-local ownership, and transactional state are application concerns.",
        [
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
          source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
        ],
      ),
      self_hosted: note(
        "LangChain is rated native for self-hosting because it is an open framework used inside customer-controlled applications, and LangSmith Deployment also documents standalone/self-hosted agent-server paths.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangSmith Deployment", "https://docs.langchain.com/langsmith/deployment"),
        ],
      ),
      lock_in: note(
        "LangChain is rated low lock-in at the framework level because docs emphasize a standard model interface and swappable providers, though LangChain APIs, middleware, traces, and LangSmith deployment choices still create migration work.",
        [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
      ),
      replayable: {
        summary:
          "LangChain is now rated partial for replayability because current agent docs describe persistence and resume via `thread_id` and checkpointers, and the overview says LangChain agents build on LangGraph durability.",
        caveat:
          "This is checkpoint/resume support for agent state, not deterministic business-workflow replay unless the app uses LangGraph-style durable execution correctly.",
        sources: [
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "LangChain is now rated native for AI agents because current docs present agents as a core framework path and describe `create_agent` as a production-ready implementation.",
        caveat:
          "This is code-framework support; LangChain does not provide full app delivery, mobile/desktop distribution, or business-user governance by itself.",
        sources: [
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "LangChain's file-limit cell stays depends because document loading, retrieval, vector stores, model context windows, and file storage are chosen by the application rather than enforced by one LangChain upload limit.",
        sources: [
          source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "LangChain is rated partial for data science because it provides retrieval, tools, structured output, model-provider abstraction, and agent harnesses, but does not include notebooks, model training, warehouses, or analytical storage.",
        sources: [
          source("LangChain retrieval", "https://docs.langchain.com/oss/python/langchain/retrieval"),
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "LangChain remains rated none for governance because the open framework does not provide enterprise admin, policy, approval, or access-control planes; LangSmith adds tracing/evaluation but not full app governance by itself.",
        sources: [
          source("LangSmith observability", "https://docs.langchain.com/langsmith/observability"),
          source("LangChain overview", "https://docs.langchain.com/oss/python/langchain/overview"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "LangChain is now rated partial for sandbox isolation because current agent docs list execution-environment middleware for tools, filesystem, sandboxes, and code execution, but hardened isolation still depends on the selected middleware and deployment.",
        sources: [
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "LangChain remains rated none for concurrent application state because checkpointers can persist per-thread agent state, but transactional multi-user state semantics must be implemented in the surrounding application and stores.",
        sources: [
          source("LangChain agents", "https://docs.langchain.com/oss/python/langchain/agents"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  LlamaIndex: {
    summary:
      "LlamaIndex is a data-centric framework for LLM apps, agents, retrieval, document ingestion, and workflows over private data. It is excellent for RAG and data agents, but does not itself deliver full user-facing apps or distribution.",
    sources: [
      source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
      source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
      source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
      source("LlamaIndex checkpointing", "https://docs.llamaindex.ai/en/stable/examples/workflow/checkpointing_workflows/"),
      source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
    ],
    cells: {
      ...codeFrameworkCells(
        "LlamaIndex",
        "RAG, document indexing, query engines, and agents over private data",
        [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
          source("LlamaIndex multi-agent workflows", "https://docs.llamaindex.ai/en/stable/understanding/agent/multi_agent/"),
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
        ],
        "LlamaIndex is rated native for data science because its core docs focus on document ingestion, indexing, retrieval, query engines, and data-centric LLM applications.",
      ),
      visual_workflow: note(
        "LlamaIndex remains rated none for visual workflow building because current docs describe code-defined workflows, agents, and ingestion/indexing pipelines rather than a native visual workflow canvas.",
        [
          source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
        ],
      ),
      high_volume: note(
        "LlamaIndex remains rated none for built-in big-data throughput because ingestion, indexing, vector storage, model serving, and workflow scaling depend on the chosen stores, services, and host infrastructure.",
        [
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
          source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
        ],
      ),
      compiled: note(
        "LlamaIndex remains rated none for compiled business logic because workflows and agents are Python code and runtime objects, not compiled portable application logic.",
        [
          source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
        ],
      ),
      ui_builder: note(
        "LlamaIndex remains rated none for app UI building because its full-stack examples are developer guides around RAG/agent backends, not a first-party screen, form, or dashboard builder.",
        [
          source("LlamaIndex full-stack app guide", "https://developers.llamaindex.ai/python/framework/understanding/putting_it_all_together/apps/fullstack_app_guide/"),
        ],
      ),
      full_apps: note(
        "LlamaIndex remains rated none for full apps because docs show how to build web apps around LlamaIndex, but the framework itself does not package, host, govern, or distribute complete applications.",
        [
          source("LlamaIndex full-stack app guide", "https://developers.llamaindex.ai/python/framework/understanding/putting_it_all_together/apps/fullstack_app_guide/"),
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
        ],
      ),
      customer_facing: note(
        "LlamaIndex remains rated none for customer-facing delivery because customer exposure requires a separate web app, authentication layer, hosting model, and operations stack built around the framework.",
        [
          source("LlamaIndex full-stack app guide", "https://developers.llamaindex.ai/python/framework/understanding/putting_it_all_together/apps/fullstack_app_guide/"),
        ],
      ),
      desktop: note(
        "LlamaIndex remains rated none for desktop delivery because public docs focus on Python framework components and web app examples, not desktop packaging.",
        [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
          source("LlamaIndex full-stack app guide", "https://developers.llamaindex.ai/python/framework/understanding/putting_it_all_together/apps/fullstack_app_guide/"),
        ],
      ),
      mobile: note(
        "LlamaIndex remains rated none for mobile delivery because mobile clients must be built separately around the framework's APIs or application backend.",
        [
          source("LlamaIndex full-stack app guide", "https://developers.llamaindex.ai/python/framework/understanding/putting_it_all_together/apps/fullstack_app_guide/"),
        ],
      ),
      offline: note(
        "LlamaIndex is rated partial for offline/local use because framework code, local models, and local stores can run under developer control, but hosted LLMs, LlamaCloud/LlamaParse, and external vector stores are often network-dependent.",
        [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
        ],
      ),
      local_first: note(
        "LlamaIndex is rated partial for local-first architecture because indexes and stores can be developer-controlled, but LlamaIndex does not define end-user offline sync, local-first conflict handling, or device-owned app state.",
        [
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
          source("LlamaIndex durable workflows", "https://docs.llamaindex.ai/en/stable/workflows/durable_workflows/"),
        ],
      ),
      self_hosted: note(
        "LlamaIndex is rated native for self-hosting because it is a developer framework that can run in customer-controlled applications and infrastructure.",
        [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
          source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
        ],
      ),
      lock_in: note(
        "LlamaIndex is rated low lock-in at the framework level because applications own their Python code, data stores, and model choices, although LlamaIndex abstractions and managed LlamaCloud services can add migration work.",
        [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
        ],
      ),
      sandbox_isolation: note(
        "LlamaIndex remains rated none for sandbox isolation because tools, data loaders, and code execution boundaries are supplied by the host application or infrastructure rather than a default hardened LlamaIndex sandbox.",
        [
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
          source("LlamaIndex workflows", "https://docs.llamaindex.ai/en/stable/workflows/"),
        ],
      ),
      replayable: {
        summary:
          "LlamaIndex is now rated partial for replayability because workflow docs show checkpointing Workflow runs and rerunning from a checkpoint, while durable workflows require explicit persistence strategies or external resources.",
        caveat:
          "This is workflow checkpointing, not Temporal-style deterministic event-history replay for arbitrary business processes.",
        sources: [
          source("LlamaIndex checkpointing", "https://docs.llamaindex.ai/en/stable/examples/workflow/checkpointing_workflows/"),
          source("LlamaIndex durable workflows", "https://docs.llamaindex.ai/en/stable/workflows/durable_workflows/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "LlamaIndex is now rated native for AI agents because its docs describe prebuilt agents, tool architectures, AgentWorkflow, and multi-agent patterns.",
        caveat:
          "This is framework-level support for developers, not a hosted app platform.",
        sources: [
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
          source("LlamaIndex multi-agent workflows", "https://docs.llamaindex.ai/en/stable/understanding/agent/multi_agent/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "LlamaIndex's file-limit cell stays depends because Documents and Nodes can represent PDFs, API output, databases, text, and beta image support, but storage/indexing limits depend on loaders, parsers, vector stores, models, and host infrastructure.",
        sources: [
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "LlamaIndex remains rated none for file-native app/project handling because Documents and Nodes are ingestion/indexing abstractions, not local project files managed as a complete application runtime.",
        sources: [
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
        ],
        checkedAt: "2026-05-30",
      },
      data_science: {
        summary:
          "LlamaIndex is rated native for data-science/RAG workflows because Documents, Nodes, indexes, query engines, agents, and multi-agent workflows are centered on connecting LLMs to private and structured data.",
        sources: [
          source("LlamaIndex documents and nodes", "https://docs.llamaindex.ai/en/stable/module_guides/loading/documents_and_nodes/"),
          source("LlamaIndex agents", "https://docs.llamaindex.ai/en/stable/use_cases/agents/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "LlamaIndex remains rated none for governance because the open framework does not ship enterprise admin, audit, approvals, or policy controls by default.",
        caveat:
          "Hosted LlamaCloud/LlamaParse services may add account controls, but this row rates the framework used in application code.",
        sources: [
          source("LlamaIndex docs", "https://docs.llamaindex.ai/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "LlamaIndex remains rated none for concurrent application state because workflow checkpointing and data stores do not provide transactional multi-user state semantics for the host app.",
        sources: [
          source("LlamaIndex checkpointing", "https://docs.llamaindex.ai/en/stable/examples/workflow/checkpointing_workflows/"),
          source("LlamaIndex durable workflows", "https://docs.llamaindex.ai/en/stable/workflows/durable_workflows/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Haystack: {
    summary:
      "Haystack is deepset's open-source framework for composable NLP, RAG, search, pipelines, document stores, agents, and tools, with a Haystack Enterprise Platform for visual pipeline work, deployment, and governance. It remains AI-pipeline infrastructure rather than a general app platform.",
    sources: [
      source("Haystack introduction", "https://docs.haystack.deepset.ai/docs/intro"),
      source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
      source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
      source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
      source("Haystack ecosystem", "https://haystack.deepset.ai/"),
      source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
    ],
    cells: {
      ...codeFrameworkCells(
        "Haystack",
        "RAG/search pipelines, document stores, agents, and tools",
        [
          source("Haystack introduction", "https://docs.haystack.deepset.ai/docs/intro"),
          source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
        ],
        "Haystack is rated native for data science because its documented core is composable NLP, search, retrieval, RAG pipelines, document stores, and evaluation-oriented components.",
      ),
      visual_workflow: note(
        "Haystack is now rated partial for visual workflow building because the Enterprise Platform advertises visual, code-aligned pipeline design, while the open-source framework remains code/YAML driven.",
        [
          source("Haystack ecosystem", "https://haystack.deepset.ai/"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      replayable: {
        summary:
          "Haystack remains rated none for replayable execution because pipeline docs describe branching, loops, async execution, and serialization, but not deterministic replay or checkpoint-resume semantics.",
        sources: [
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Haystack is now rated partial for high-volume execution because the framework supports scalable RAG/search pipelines, branching, async/concurrent processing, and Kubernetes-ready deployments, while large-scale serving still depends on infrastructure and stores.",
        sources: [
          source("Haystack ecosystem", "https://haystack.deepset.ai/"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: note(
        "Haystack remains rated none for compiled business logic because pipelines serialize to YAML/dictionaries and custom components are Python code, not compiled portable workflow artifacts.",
        [
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      file_size: {
        summary:
          "Haystack's file-limit cell stays depends because documents are converted into Haystack Document objects and written to chosen document stores; limits come from parsers, stores, models, and application infrastructure.",
        sources: [
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: note(
        "Haystack remains rated none for file-native app/project handling because files become Documents in converters, pipelines, or document stores, not local project artifacts managed by a complete application runtime.",
        [
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
        ],
      ),
      ai_agents: {
        summary:
          "Haystack is now rated native for AI agents because Haystack documents an Agent component that uses chat LLMs and tools to solve complex queries iteratively.",
        caveat:
          "This is developer-framework support, not a complete app runtime or governed workflow product.",
        sources: [
          source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "Haystack remains rated none for app UI building because the visual Enterprise Platform surface is for AI pipelines, testing, and deployment, not arbitrary forms, dashboards, or application screens.",
        [
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      full_apps: note(
        "Haystack is now rated partial for full apps because the Enterprise Platform says it can build, test, deploy, and monitor AI agents and applications, but delivery is still pipeline/API centered rather than a general app runtime.",
        [
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      customer_facing: note(
        "Haystack is now rated partial for customer-facing delivery because the Enterprise Platform supports shareable prototypes and REST API deployment, while teams still build the final customer UI themselves.",
        [
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      desktop: note(
        "Haystack remains rated none for desktop delivery because public docs describe a Python framework and web/cloud/on-prem enterprise platform, not desktop application packaging.",
        [
          source("Haystack introduction", "https://docs.haystack.deepset.ai/docs/intro"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      mobile: note(
        "Haystack remains rated none for mobile delivery because mobile clients must be built separately around Haystack pipelines or deployed APIs.",
        [
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      offline: note(
        "Haystack is rated partial for offline/local use because the open-source framework can run in customer-controlled environments, but hosted models, managed platform features, and external document stores are often network-dependent.",
        [
          source("Haystack introduction", "https://docs.haystack.deepset.ai/docs/intro"),
          source("Haystack ecosystem", "https://haystack.deepset.ai/"),
        ],
      ),
      local_first: note(
        "Haystack is rated partial for local-first architecture because code, pipelines, and stores can be customer-controlled, but Haystack does not define end-user device-local sync or local-first app state semantics.",
        [
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
        ],
      ),
      data_science: {
        summary:
          "Haystack is rated native for data-science/RAG workflows because Document Stores, retrievers, indexing/query pipelines, agents, loops, branching, and async pipelines are first-party concepts.",
        sources: [
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Haystack is now rated enterprise for governance because the Enterprise Platform advertises secure access controls, auditability, governed deployments, and RBAC with organization/workspace roles and granular permissions.",
        sources: [
          source("Haystack introduction", "https://docs.haystack.deepset.ai/docs/intro"),
          source("Haystack ecosystem", "https://haystack.deepset.ai/"),
          source("Haystack Enterprise RBAC", "https://www.deepset.ai/blog/haystack-platform-rbac-ai-security"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: note(
        "Haystack is rated native for self-hosting because the open-source framework runs under customer control and deepset advertises cloud, VPC, on-prem, and Kubernetes-ready deployment paths.",
        [
          source("Haystack ecosystem", "https://haystack.deepset.ai/"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      lock_in: note(
        "Haystack is rated low lock-in because pipelines are serializable, cloud-agnostic, and exportable as Python/YAML, though Enterprise Platform features still create migration work.",
        [
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      sandbox_isolation: note(
        "Haystack remains rated none for sandbox isolation because docs show tools, custom components, and secure deployment options, but not a default hardened sandbox for arbitrary untrusted code or agent tools.",
        [
          source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
          source("Haystack Enterprise Platform", "https://www.deepset.ai/products-and-services/haystack-enterprise-platform"),
        ],
      ),
      concurrent_state: {
        summary:
          "Haystack is now rated partial for concurrent state because Agent supports runtime state schemas and pipelines can branch or run concurrent flows, while transactional shared app state remains the host application's responsibility.",
        sources: [
          source("Haystack Agent", "https://docs.haystack.deepset.ai/docs/agent"),
          source("Haystack pipelines", "https://docs.haystack.deepset.ai/docs/pipelines"),
          source("Haystack document store", "https://docs.haystack.deepset.ai/docs/document-store"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Make.com": {
    summary:
          "Make is a hosted visual iPaaS for scenarios, app integrations, data stores, and AI-assisted automation. It is approachable and broad, but cloud-only and oriented around integration scenarios rather than app delivery.",
    sources: [
      source("Make help center", "https://www.make.com/en/help"),
      source("Make AI Agents", "https://www.make.com/en/ai-agents"),
      source("Make scenario replay", "https://help.make.com/scenario-run-replay"),
      source("Make file size pricing", "https://www.make.com/en/pricing"),
      source("Make working with files", "https://help.make.com/working-with-files"),
      source("Make app concepts", "https://www.make.com/en/help/app/app.html"),
    ],
    cells: {
      ...hostedAutomationCells("Make.com", "visual app integration and AI-assisted scenarios", [
        source("Make help center", "https://www.make.com/en/help"),
        source("Make AI Agents", "https://www.make.com/en/ai-agents"),
        source("Make scenario replay", "https://help.make.com/scenario-run-replay"),
        source("Make pricing", "https://www.make.com/en/pricing"),
      ]),
      visual_workflow: note(
        "Make.com is rated native for visual workflow building because Make scenarios are built as visual modules and routes, with notes, templates, and prior-run trigger data for testing and debugging.",
        [
          source("Make scenarios", "https://help.make.com/scenarios"),
          source("Make app concepts", "https://www.make.com/en/help/app/app.html"),
        ],
        "This visual builder is for hosted automation scenarios, not a full application UI/runtime.",
      ),
      replayable: {
        summary:
          "Make.com is now rated partial for replayable execution because Make documents scenario run replay for backfills and error recovery, but not deterministic event-history replay of business logic.",
        caveat:
          "Replay is tied to scenario history/run data and the hosted Make execution model.",
        sources: [
          source("Make scenario replay", "https://help.make.com/scenario-run-replay"),
          source("Scenario run replay announcement", "https://help.make.com/scenario-run-replay-and-naming-capabilities-now-available"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Make.com is now rated 1 GB because Make's current pricing/help materials list plan-specific maximum file sizes up to 1,000 MB on Enterprise.",
        evidence:
          "The public plan table lists 5 MB, 100 MB, 250 MB, 500 MB, and 1,000 MB file-size tiers; the previous 250 MB value represented only one plan tier.",
        caveat:
          "The actual limit depends on plan, so lower tiers remain substantially smaller.",
        sources: [
          source("Make pricing", "https://www.make.com/en/pricing"),
          source("Make working with files", "https://help.make.com/working-with-files"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Make.com is now rated native for AI agents because Make publicly offers first-party AI Agents that can use apps and scenarios inside the Make platform.",
        caveat:
          "This is native inside Make's hosted platform; it is not a portable local agent runtime.",
        sources: [
          source("Make AI Agents", "https://www.make.com/en/ai-agents"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Make.com is now rated partial for customer-facing delivery because Make AI Agents can be triggered through chat messages, emails, forms, and webhooks, but Make still does not package full customer apps.",
        sources: [
          source("Make AI Agents New", "https://help.make.com/make-ai-agent"),
          source("Create AI agents for triggers", "https://help.make.com/create-ai-agents-for-different-triggers"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Make.com remains rated none for self-hosting because public materials present Make as a managed cloud automation platform.",
        sources: [
          source("Make help center", "https://www.make.com/en/help"),
          source("Make pricing", "https://www.make.com/en/pricing"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: note(
        "Make.com is rated partial for sandbox isolation because Make Code documents a fully isolated code sandbox for custom code modules, while the overall scenario runtime remains Make-hosted and not a portable arbitrary-tool sandbox.",
        [
          source("Make Code app", "https://apps.make.com/code"),
          source("Make scenarios", "https://help.make.com/scenarios"),
        ],
      ),
    },
  },
  Workato: {
    summary:
      "Workato is an enterprise iPaaS with recipes, connectors, governance, on-prem agents, and AI features. It is strong for governed integration automation, but remains a hosted enterprise automation platform.",
    sources: [
      source("Workato docs", "https://docs.workato.com/"),
      source("Workato Agentic", "https://www.workato.com/agentic"),
      source("Workato platform limits", "https://docs.workato.com/en/limits.html"),
      source("Workato on-prem agent", "https://docs.workato.com/on-prem.html"),
      source("Workato recipe jobs", "https://docs.workato.com/recipes/jobs"),
      source("Workato Workflow apps", "https://docs.workato.com/en/workflow-apps.html"),
      source("Workato RBAC", "https://docs.workato.com/user-accounts-and-teams/role-based-access/"),
    ],
    cells: {
      ...hostedAutomationCells("Workato", "enterprise recipes, connectors, and workflow apps", [
        source("Workato docs", "https://docs.workato.com/"),
        source("Workato platform limits", "https://docs.workato.com/en/limits.html"),
        source("Workato Agentic", "https://www.workato.com/agentic"),
      ]),
      visual_workflow: note(
        "Workato is rated native for visual workflow building because Workato recipes are built in a recipe editor and Workato documents workflow, API, data-pipeline, app-event, and knowledge-base recipe types.",
        [
          source("Workato recipes", "https://docs.workato.com/en/recipes"),
          source("Workato docs", "https://docs.workato.com/"),
        ],
        "Workato also has Workflow apps, but this cell is primarily about recipe automation rather than full app distribution.",
      ),
      replayable: {
        summary:
          "Workato is rated partial for replayability because recipe jobs retain trigger-event data and can be rerun from job history or via RecipeOps, but reruns use the latest recipe version rather than deterministic event-history replay.",
        sources: [
          source("Workato recipe jobs", "https://docs.workato.com/recipes/jobs"),
          source("RecipeOps rerun jobs", "https://docs.workato.com/connectors/recipeops/actions/rerun-jobs.html"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Workato is rated partial for high-volume execution because platform limits document webhook rates, recipe concurrency, queues, long actions, connector timeouts, and data-orchestration limits rather than unbounded throughput.",
        caveat:
          "Enterprise customers can request some limit increases, and heavy processing often belongs in connected data platforms or bulk connector actions.",
        sources: [
          source("Workato platform limits", "https://docs.workato.com/en/limits.html"),
          source("Workato tasks", "https://docs.workato.com/recipes/tasks"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Workato is now rated 10 GB because Workato's platform limits list a 10 GB maximum file size for FileStorage, even though many other Workato payload paths are smaller.",
        evidence:
          "The same limits page also lists 50 MB trigger payloads, 5 MB API payloads, 5 GB API attachments, 500 MB files on standard Pages, and 100 MB on public Pages.",
        caveat:
          "This cell uses Workato FileStorage's maximum; recipe/API/message payloads should still be treated as path-specific.",
        sources: [
          source("Workato platform limits", "https://docs.workato.com/en/limits.html"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Workato is now rated native for AI agents because Workato publicly positions Agentic as a platform for creating and managing enterprise AI agents on top of Workato automation.",
        caveat:
          "The agent capability is part of Workato's enterprise hosted platform and recipe/connectivity model.",
        sources: [
          source("Workato Agentic", "https://www.workato.com/agentic"),
          source("Workato docs", "https://docs.workato.com/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: {
        summary:
          "Workato is rated partial for UI building because Workflow apps provide a no-code app environment with a drag-and-drop page builder, forms, dashboards, app portal, and data tables, but the UI remains Workato portal/app scoped.",
        sources: [
          source("Workato Workflow apps", "https://docs.workato.com/en/workflow-apps.html"),
          source("Workato create page", "https://docs.workato.com/workflow-apps/pages-create.html"),
        ],
        checkedAt: "2026-05-30",
      },
      full_apps: {
        summary:
          "Workato is now rated partial for full apps because Workflow apps can create brandable business applications with data tables, pages, request workflows, and a portal, but not portable desktop/mobile/customer apps outside Workato.",
        sources: [
          source("Workato Workflow apps", "https://docs.workato.com/en/workflow-apps.html"),
          source("Workato Workflow apps getting started", "https://docs.workato.com/en/workflow-apps/getting-started.html"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Workato is rated partial for customer-facing delivery because Embedded connection widgets/APIs and public Workflow app pages can expose integration or form experiences, but Workato does not package full customer applications.",
        sources: [
          source("Workato Embedded Connection Widget", "https://docs.workato.com/oem/embedded-connections.html"),
          source("Workato Workflow apps limits", "https://docs.workato.com/en/workflow-apps/limits.html"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Workato is rated enterprise for governance because docs describe workspace/environment/project RBAC, custom roles, collaborator groups, environments, lifecycle management, data retention, and RecipeOps/job reporting controls.",
        sources: [
          source("Workato RBAC", "https://docs.workato.com/user-accounts-and-teams/role-based-access/"),
          source("Workato environments", "https://docs.workato.com/features/environments.html"),
          source("RecipeOps by Workato", "https://docs.workato.com/en/connectors/recipeops.html"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Workato is rated partial for self-hosting because it offers on-prem agents for private connectivity, while the automation control plane remains Workato-hosted.",
        sources: [
          source("Workato on-prem agent", "https://docs.workato.com/on-prem.html"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Workato is rated partial for concurrent state because it manages recipe jobs, queues, paused jobs, reruns, and workflow app records, but transactional business-state safety still depends on recipe design and connected systems.",
        sources: [
          source("Workato recipe jobs", "https://docs.workato.com/recipes/jobs"),
          source("Workato platform limits", "https://docs.workato.com/en/limits.html"),
          source("Workato Workflow apps", "https://docs.workato.com/en/workflow-apps.html"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: note(
        "Workato is rated partial for sandbox isolation because connectors and on-prem agents run under Workato-managed limits, RBAC, allow-listed SDK methods, and private-network connectivity, but Workato does not document a portable hardened sandbox for arbitrary untrusted code.",
        [
          source("Workato Connector SDK quickstart", "https://docs.workato.com/en/developing-connectors/sdk/quickstart.html"),
          source("Workato Connector SDK limits", "https://docs.workato.com/en/developing-connectors/sdk/limits.html"),
          source("Workato on-prem connectivity", "https://docs.workato.com/en/on-prem.html"),
        ],
      ),
    },
  },
  Pipedream: {
    summary:
      "Pipedream is a developer-oriented integration platform with event sources, workflows, managed code steps, and AI actions. It is fast for API automation, but not an app UI/distribution platform.",
    sources: [
      source("Pipedream docs", "https://pipedream.com/docs/"),
      source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
      source("Pipedream AI actions", "https://pipedream.com/docs/connect/ai/"),
      source("Pipedream events", "https://pipedream.com/docs/workflows/events/"),
      source("Pipedream Connect", "https://pipedream.com/docs/connect"),
      source("Pipedream workspaces", "https://pipedream.com/docs/workspaces/"),
    ],
    cells: {
      ...hostedAutomationCells("Pipedream", "developer-oriented API workflows and managed code steps", [
        source("Pipedream docs", "https://pipedream.com/docs/"),
        source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
        source("Pipedream AI actions", "https://pipedream.com/docs/connect/ai/"),
      ]),
      visual_workflow: note(
        "Pipedream is rated native for visual workflow building because its workflow builder lets users create trigger-driven workflows, add prebuilt actions or custom code steps, configure steps, and inspect execution details without managing servers.",
        [
          source("Pipedream workflows", "https://pipedream.com/docs/workflows/building-workflows"),
          source("Pipedream workflow quickstart", "https://pipedream.com/docs/workflows/quickstart"),
        ],
        "The builder is developer-oriented and linear step-based, not a no-code app-screen builder.",
      ),
      replayable: {
        summary:
          "Pipedream is rated partial for replayability because workflow event context includes replay metadata and the UI supports replaying events, but this is event rerun/debug support rather than deterministic workflow replay.",
        sources: [
          source("Pipedream events", "https://pipedream.com/docs/workflows/events/"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Pipedream remains rated none for high-volume throughput because workflow limits document HTTP QPS, memory, disk, execution-time, log/export, and event-retention caps; paid plans remove some credit caps but not all platform limits.",
        sources: [
          source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
        ],
        checkedAt: "2026-05-30",
      },
      compiled: {
        summary:
          "Pipedream remains rated none for compiled business logic because workflows are hosted triggers, actions, and code steps rather than compiled portable application logic.",
        sources: [
          source("Pipedream code steps", "https://pipedream.com/docs/workflows/building-workflows/code"),
        ],
        checkedAt: "2026-05-30",
      },
      file_size: {
        summary:
          "Pipedream is now rated 5 TB for large HTTP payloads because its limits page says the large-body and large-file upload interfaces support uploads up to 5 TB.",
        caveat:
          "Default HTTP request bodies are still much smaller, and workflow memory, disk, logs, timeouts, and event exports impose separate practical limits.",
        sources: [
          source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Pipedream remains rated partial for AI agents because Connect can expose prebuilt tools and actions to applications or AI agents, but Pipedream itself is not a first-party autonomous agent runtime.",
        sources: [
          source("Pipedream Connect components", "https://pipedream.com/docs/connect/components/"),
          source("Pipedream AI actions", "https://pipedream.com/docs/connect/ai/"),
        ],
        checkedAt: "2026-05-30",
      },
      customer_facing: {
        summary:
          "Pipedream is now rated partial for customer-facing delivery because Pipedream Connect is documented for embedding API integrations, triggers, and actions into a product or AI agent for end users.",
        caveat:
          "This delivers embedded integration/tooling surfaces, not complete customer-facing applications.",
        sources: [
          source("Pipedream Connect", "https://pipedream.com/docs/connect"),
          source("Pipedream Connect components", "https://pipedream.com/docs/connect/components/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Pipedream is rated basic for governance because workspace docs include Owner/Admin/Member roles, project-level permissions, required 2FA, SSO, and SCIM, but not a broad enterprise workflow governance suite.",
        sources: [
          source("Pipedream workspaces", "https://pipedream.com/docs/workspaces/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "Pipedream is now rated none for self-hosting because public docs present workflows, Connect, events, and managed code steps as Pipedream-hosted services rather than a customer-run runtime.",
        caveat:
          "Pipedream Connect can be embedded into another product, but that is not self-hosting the workflow runtime.",
        sources: [
          source("Pipedream docs", "https://pipedream.com/docs/"),
          source("Pipedream Connect", "https://pipedream.com/docs/connect/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Pipedream is rated partial for sandbox isolation because code steps execute in Pipedream's managed workflow runtime, but the docs do not expose a portable hardened sandbox model for arbitrary untrusted code.",
        sources: [
          source("Pipedream code steps", "https://pipedream.com/docs/workflows/building-workflows/code"),
          source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Pipedream is rated partial for concurrent state because every event triggers a separate workflow execution with event context and trace IDs, but transactional shared business state must be handled by external services or Pipedream data stores.",
        sources: [
          source("Pipedream events", "https://pipedream.com/docs/workflows/events/"),
          source("Pipedream workflow limits", "https://pipedream.com/docs/workflows/limits"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  OpenClaw: {
    summary:
      "OpenClaw is represented as an open autonomous/coding-agent style tool with a gateway, local agent workspace, messaging channels, tools, sandboxing, and mobile/desktop control surfaces. Ratings stay focused on agent-local execution rather than enterprise app delivery.",
    sources: [
      source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
      source("OpenClaw agent runtime", "https://openclawlab.com/en/docs/concepts/agent/"),
      source("OpenClaw sandbox", "https://openclawlab.com/en/docs/concepts/sandbox/"),
      source("OpenClaw session concurrency", "https://openclawlab.com/en/docs/deep-dive/framework-focus/session-concurrency-framework/"),
    ],
    cells: {
      ...codingAgentCells("OpenClaw", [
        source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
        source("OpenClaw agent runtime", "https://openclawlab.com/en/docs/concepts/agent/"),
        source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
      ]),
      visual_workflow: note(
        "OpenClaw is rated none for visual workflow building because its documented surfaces are agent runtime, gateway, workspace, tools, sandbox, and platform clients rather than a visual process-builder canvas.",
        [
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
        ],
      ),
      ai_agents: {
        summary:
          "OpenClaw is rated native for AI agents because the current docs describe a gateway architecture, an agent runtime, agent workspace files, tools, memory, channels, and model-provider configuration.",
        caveat:
          "The rating is for autonomous agent operation, not for general business app delivery.",
        sources: [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw agent runtime", "https://openclawlab.com/en/docs/concepts/agent/"),
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "OpenClaw remains rated none for replayable execution because the docs describe session stores, command queues, and agent workspaces, but not deterministic replay of completed tool executions or business workflows.",
        [
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
          source("OpenClaw session concurrency", "https://openclawlab.com/en/docs/deep-dive/framework-focus/session-concurrency-framework/"),
        ],
      ),
      high_volume: note(
        "OpenClaw is rated none for big-data throughput because the documented runtime is a personal agent/gateway workspace with sessions and tools, not a batch data pipeline or fleet-scale workflow engine.",
        [
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
          source("OpenClaw gateway agent config", "https://docs.openclaw.ai/gateway/config-agents"),
        ],
      ),
      compiled: note(
        "OpenClaw is rated none for compiled business logic because its value is agent runtime/tool orchestration and workspace files, not compiling durable business workflows into deployable application logic.",
        [
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
        ],
      ),
      file_size: note(
        "OpenClaw's file-limit cell stays depends because workspace files, inbound media, sandbox bind mounts, model context, and tool policy are configured in the gateway/runtime rather than governed by one upload cap.",
        [
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
          source("OpenClaw gateway agent config", "https://docs.openclaw.ai/gateway/config-agents"),
        ],
      ),
      data_science: {
        summary:
          "OpenClaw remains rated none for data-science workflows because its docs focus on personal/coding agents, gateway channels, workspace files, and tools rather than notebooks, ML training, analytical pipelines, or governed datasets.",
        caveat:
          "An agent could call data tools through custom skills, but that is extensibility rather than a native data-science platform.",
        sources: [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw tools", "https://openclawlab.com/en/docs/tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "OpenClaw is rated none for end-user UI building because its public surfaces are agent gateways, messaging/control clients, tools, and workspace files rather than a no-code app-screen builder.",
        [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
        ],
      ),
      full_apps: note(
        "OpenClaw is rated none for shipping full applications because it can operate tools and edit artifacts, but it does not package, host, or distribute complete business applications as a product capability.",
        [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw tools", "https://openclawlab.com/en/docs/tools/"),
        ],
      ),
      customer_facing: note(
        "OpenClaw is rated none for customer-facing apps because Android, iOS, macOS, and messaging surfaces are ways to control or talk to the agent, not customer app deployment channels.",
        [
          source("OpenClaw Android app", "https://openclawlab.com/en/docs/platforms/android/"),
          source("OpenClaw iOS app", "https://openclawlab.com/en/docs/platforms/ios/"),
          source("OpenClaw macOS app", "https://openclawlab.com/en/docs/platforms/macos/"),
        ],
      ),
      desktop: {
        summary:
          "OpenClaw is rated partial for desktop because public docs include macOS and desktop-adjacent gateway/control surfaces, but not a packaged runtime for shipping custom desktop apps.",
        sources: [
          source("OpenClaw macOS app", "https://openclawlab.com/en/docs/platforms/macos/"),
          source("OpenClaw gateway on macOS", "https://openclawlab.com/en/docs/platforms/mac/gateway/"),
        ],
        checkedAt: "2026-05-30",
      },
      mobile: {
        summary:
          "OpenClaw is now rated partial for mobile because public docs include Android and iOS app pages for controlling or interacting with the agent, but those clients are not a mobile app builder for custom business apps.",
        sources: [
          source("OpenClaw Android app", "https://openclawlab.com/en/docs/platforms/android/"),
          source("OpenClaw iOS app", "https://openclawlab.com/en/docs/platforms/ios/"),
        ],
        checkedAt: "2026-05-30",
      },
      offline: note(
        "OpenClaw is rated partial for offline work because gateway and workspace operations can run under local control, but model-provider calls, messaging channels, and many tools still depend on network services.",
        [
          source("OpenClaw agent runtime", "https://docs.openclaw.ai/agent"),
          source("OpenClaw gateway agent config", "https://docs.openclaw.ai/gateway/config-agents"),
        ],
      ),
      local_first: {
        summary:
          "OpenClaw is rated native for local-first behavior because the docs describe a local gateway/agent workspace model where agent context, tools, and sessions are controlled from the user's own environment.",
        caveat:
          "Model inference can still use external providers such as Claude, OpenAI, or hosted model gateways.",
        sources: [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
        ],
        checkedAt: "2026-05-30",
      },
      file_native: {
        summary:
          "OpenClaw is rated native for file-native work because agent workspace files define identity, tools, skills, memory, and runtime context, and the tool model includes file operations.",
        sources: [
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
        ],
        checkedAt: "2026-05-30",
      },
      self_hosted: {
        summary:
          "OpenClaw is rated native for self-hosting/local control because docs describe local and remote gateway deployment, Docker, macOS, Linux, Android, iOS, and other platform paths rather than only a managed SaaS surface.",
        sources: [
          source("OpenClaw docs", "https://openclawlab.com/en/docs/"),
          source("OpenClaw Docker", "https://openclawlab.com/en/docs/install/docker/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "OpenClaw is rated low lock-in because its artifacts are local workspaces, source files, and open agent configuration, although prompts, session history, and model-provider behavior may not migrate cleanly.",
        [
          source("OpenClaw agent workspace", "https://openclawlab.com/en/docs/concepts/agent-workspace/"),
          source("OpenClaw repository", "https://github.com/openclaw/openclaw"),
        ],
      ),
      sandbox_isolation: {
        summary:
          "OpenClaw is now rated partial for sandbox isolation because current docs include Sandbox and Sandbox CLI surfaces, but agent safety still depends on configured tool policy, approvals, and deployment choices.",
        sources: [
          source("OpenClaw sandbox", "https://openclawlab.com/en/docs/concepts/sandbox/"),
          source("OpenClaw Sandbox CLI", "https://openclawlab.com/en/docs/cli/sandbox/"),
          source("OpenClaw tool policy and approvals", "https://openclawlab.com/en/docs/deep-dive/framework-focus/tool-policy-exec-approvals/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "OpenClaw is rated none for governance because the available OpenClaw materials emphasize local/gateway agent operation, sandboxing, and tool policy rather than enterprise RBAC, audit, approval, or admin controls.",
        sources: [
          source("OpenClaw repository", "https://github.com/openclaw/openclaw"),
          source("OpenClaw tool policy and approvals", "https://openclawlab.com/en/docs/deep-dive/framework-focus/tool-policy-exec-approvals/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "OpenClaw is now rated partial for concurrent state because public docs describe session keys, lanes, queues, active runs, and session/global two-level queueing, but not transactional application-state semantics.",
        sources: [
          source("OpenClaw session concurrency", "https://openclawlab.com/en/docs/deep-dive/framework-focus/session-concurrency-framework/"),
          source("OpenClaw command queue", "https://openclawlab.com/en/docs/concepts/command-queue/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Hermes Agent": {
    summary:
      "Hermes Agent is represented as an autonomous/coding-agent style tool. Public evidence is limited, so ratings focus on agent-local execution and treat enterprise governance, app delivery, and runtime guarantees conservatively.",
    sources: [
      source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
      source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
      source("Hermes Agent profiles", "https://hermes-agent.nousresearch.com/docs/user-guide/profiles/"),
      source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
    ],
    cells: {
      ...codingAgentCells("Hermes Agent", [
        source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
        source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
        source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
      ]),
      visual_workflow: note(
        "Hermes Agent is rated none for visual workflow building because its documented interaction surfaces are CLI/TUI, messaging gateways, toolsets, skills, profiles, and automation hooks rather than a visual process-builder canvas.",
        [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
        ],
      ),
      replayable: {
        summary:
          "Hermes Agent is rated partial for replayability because its CLI docs show session resume, conversation history, context compression, and command history, but not deterministic event-history replay.",
        sources: [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("Hermes Agent docs", "https://hermes-agent.nousresearch.com/docs/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Hermes Agent is rated native for AI agents because its public CLI docs and repository present it as an autonomous agent tool.",
        sources: [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: note(
        "Hermes Agent is rated none for big-data throughput because batch processing and background sessions are agent/evaluation conveniences, not a governed high-volume data or workflow execution engine.",
        [
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
        ],
      ),
      compiled: note(
        "Hermes Agent is rated none for compiled business logic because it runs terminal tools, Python snippets, skills, hooks, and MCP integrations; it does not compile portable business workflows into deployable logic.",
        [
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
          source("Hermes code execution", "https://hermes-agent.nousresearch.com/docs/user-guide/features/code-execution/"),
        ],
      ),
      file_size: note(
        "Hermes Agent's file-limit cell stays depends because context references, deliverable attachments, terminal/code execution output caps, and gateway media handling each have separate constraints rather than one product-wide upload limit.",
        [
          source("Hermes context references", "https://hermes-agent.nousresearch.com/docs/user-guide/features/context-references/"),
          source("Hermes code execution", "https://hermes-agent.nousresearch.com/docs/user-guide/features/code-execution/"),
          source("Hermes deliverable mode", "https://hermes-agent.nousresearch.com/docs/user-guide/features/deliverable-mode"),
        ],
      ),
      ui_builder: note(
        "Hermes Agent is rated none for end-user UI building because the CLI/TUI and messaging integrations are agent interfaces, not a no-code app-screen builder for business users.",
        [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("Hermes Agent docs", "https://hermes-agent.nousresearch.com/docs/"),
        ],
      ),
      full_apps: note(
        "Hermes Agent is rated none for shipping full applications because it can generate files, run tools, and expose an API server, but application hosting, packaging, and delivery are external implementation work.",
        [
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
          source("Hermes deliverable mode", "https://hermes-agent.nousresearch.com/docs/user-guide/features/deliverable-mode"),
        ],
      ),
      customer_facing: note(
        "Hermes Agent is rated none for customer-facing apps because messaging gateways and deliverable attachments expose the agent's output, not a product surface for deploying customer applications.",
        [
          source("Hermes messaging gateway", "https://hermes-agent.nousresearch.com/docs/user-guide/messaging/"),
          source("Hermes deliverable mode", "https://hermes-agent.nousresearch.com/docs/user-guide/features/deliverable-mode"),
        ],
      ),
      desktop: note(
        "Hermes Agent is rated partial for desktop because it runs as a local CLI/TUI and can integrate with IDEs through ACP, but it does not package custom desktop applications for end users.",
        [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
        ],
      ),
      offline: note(
        "Hermes Agent is rated partial for offline use because terminal/file work can happen locally, but model routing, web tools, gateways, browser automation, and many integrations depend on remote providers.",
        [
          source("Hermes Agent docs", "https://hermes-agent.nousresearch.com/docs/"),
          source("Hermes Tool Gateway", "https://hermes-agent.nousresearch.com/docs/user-guide/features/tool-gateway"),
        ],
      ),
      local_first: {
        summary:
          "Hermes Agent is rated native for local-first behavior because it is used as a local CLI agent against the user's workspace.",
        caveat:
          "Model calls may still depend on external model providers.",
        sources: [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Hermes Agent is rated low lock-in because the tool is open source and works from local project files, profiles, skills, and configuration, though provider routing and session history can still be product-specific.",
        [
          source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
          source("Hermes Agent profiles", "https://hermes-agent.nousresearch.com/docs/user-guide/profiles/"),
        ],
      ),
      sandbox_isolation: {
        summary:
          "Hermes Agent is rated partial for sandbox isolation because its feature docs describe sandboxed RPC code execution, while broader shell/tool safety still depends on the local CLI environment and user approvals.",
        sources: [
          source("Hermes Agent features", "https://hermes-agent.nousresearch.com/docs/user-guide/features/overview/"),
          source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Hermes Agent is rated none for governance because its public docs emphasize local/gateway agent configuration, toolsets, profiles, plugins, and provider routing rather than enterprise RBAC, audit, approval, or admin controls.",
        sources: [
          source("NousResearch hermes-agent", "https://github.com/NousResearch/hermes-agent"),
          source("Hermes Agent profiles", "https://hermes-agent.nousresearch.com/docs/user-guide/profiles/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Hermes Agent is rated partial for concurrent state because the CLI exposes background tasks and profiles for separate agents, but it does not provide transactional multi-user application state.",
        sources: [
          source("Hermes Agent CLI docs", "https://hermes-agent.nousresearch.com/docs/user-guide/cli/"),
          source("Hermes Agent profiles", "https://hermes-agent.nousresearch.com/docs/user-guide/profiles/"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  OpenHands: {
    summary:
      "OpenHands is an open-source software-development agent platform for coding tasks, local workspaces, and sandboxed execution patterns. It is agent/developer tooling, not a governed business app platform.",
    sources: [
      source("OpenHands docs", "https://docs.all-hands.dev/"),
      source("OpenHands runtime", "https://docs.all-hands.dev/openhands/usage/architecture/runtime"),
      source("OpenHands runtimes", "https://docs.all-hands.dev/usage/runtimes"),
      source("OpenHands GitHub", "https://github.com/All-Hands-AI/OpenHands"),
    ],
    cells: {
      ...codingAgentCells("OpenHands", [
        source("OpenHands docs", "https://docs.all-hands.dev/"),
        source("OpenHands runtime", "https://docs.all-hands.dev/openhands/usage/architecture/runtime"),
        source("OpenHands GitHub", "https://github.com/All-Hands-AI/OpenHands"),
      ]),
      visual_workflow: note(
        "OpenHands is rated none for visual workflow building because its documented surfaces are CLI, web GUI, IDE integration, SDK agents, and sandboxes rather than a visual process-builder canvas.",
        [
          source("OpenHands CLI quick start", "https://docs.openhands.dev/openhands/usage/cli/quick-start"),
          source("OpenHands IDE integration", "https://docs.openhands.dev/openhands/usage/run-openhands/acp"),
          source("OpenHands SDK getting started", "https://docs.openhands.dev/sdk/getting-started"),
        ],
      ),
      ai_agents: {
        summary:
          "OpenHands is rated native for AI agents because it is explicitly built as a software-development agent platform.",
        sources: [
          source("OpenHands docs", "https://docs.all-hands.dev/"),
          source("OpenHands GitHub", "https://github.com/All-Hands-AI/OpenHands"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "OpenHands is rated partial for replayability because the CLI saves conversation history and supports resuming conversations, but the docs do not describe deterministic replay of a completed coding run.",
        [
          source("OpenHands CLI installation", "https://docs.openhands.dev/openhands/usage/cli/installation"),
          source("OpenHands CLI quick start", "https://docs.openhands.dev/openhands/usage/cli/quick-start"),
        ],
      ),
      high_volume: note(
        "OpenHands is rated none for big-data throughput because its SDK and CLI are built for software-development agents that edit files, run commands, and browse the web, not fleet-scale business workflow or data-pipeline execution.",
        [
          source("OpenHands SDK getting started", "https://docs.openhands.dev/sdk/getting-started"),
          source("OpenHands sandbox overview", "https://docs.openhands.dev/usage/runtimes/overview"),
        ],
      ),
      compiled: note(
        "OpenHands is rated none for compiled business logic because it helps agents write or modify code in a sandbox, but it does not compile its own workflow definitions into portable business logic.",
        [
          source("OpenHands SDK getting started", "https://docs.openhands.dev/sdk/getting-started"),
          source("OpenHands Docker sandbox", "https://docs.openhands.dev/sdk/guides/agent-server/docker-sandbox"),
        ],
      ),
      file_size: note(
        "OpenHands' file-limit cell stays depends because repository size, sandbox volumes, workspace backends, model context, and hosted workspace constraints are deployment choices rather than one OpenHands upload ceiling.",
        [
          source("OpenHands CLI installation", "https://docs.openhands.dev/openhands/usage/cli/installation"),
          source("OpenHands sandbox overview", "https://docs.openhands.dev/usage/runtimes/overview"),
          source("OpenHands Docker sandbox", "https://docs.openhands.dev/sdk/guides/agent-server/docker-sandbox"),
        ],
      ),
      ui_builder: note(
        "OpenHands is rated none for end-user UI building because its web GUI, CLI, and IDE integrations are developer control surfaces for an agent, not a no-code app-screen builder.",
        [
          source("OpenHands CLI quick start", "https://docs.openhands.dev/openhands/usage/cli/quick-start"),
          source("OpenHands IDE integration", "https://docs.openhands.dev/openhands/usage/run-openhands/acp"),
        ],
      ),
      full_apps: note(
        "OpenHands is rated none for shipping full applications because it can help modify a repository, while packaging, hosting, mobile/desktop delivery, and production operations remain the responsibility of the generated project.",
        [
          source("OpenHands SDK getting started", "https://docs.openhands.dev/sdk/getting-started"),
          source("OpenHands CLI quick start", "https://docs.openhands.dev/openhands/usage/cli/quick-start"),
        ],
      ),
      customer_facing: note(
        "OpenHands is rated none for customer-facing delivery because its documented product surface is an agent for editing and running software-development work, not an app-hosting, portal, or end-user distribution product.",
        [
          source("OpenHands SDK getting started", "https://docs.openhands.dev/sdk/getting-started"),
          source("OpenHands docs", "https://docs.openhands.dev/"),
        ],
      ),
      desktop: note(
        "OpenHands is rated none for desktop application delivery because its supported surfaces are CLI, web GUI, and IDE integrations, not packaging custom native desktop apps for end users.",
        [
          source("OpenHands CLI quick start", "https://docs.openhands.dev/openhands/usage/cli/quick-start"),
          source("OpenHands IDE integration", "https://docs.openhands.dev/openhands/usage/run-openhands/acp"),
        ],
      ),
      offline: note(
        "OpenHands is rated partial for offline use because local CLI/GUI and Docker sandbox work can run under user control, but LLM access and many integrations generally require remote providers.",
        [
          source("OpenHands quick start", "https://docs.openhands.dev/usage/installation"),
          source("OpenHands sandbox overview", "https://docs.openhands.dev/usage/runtimes/overview"),
        ],
      ),
      self_hosted: {
        summary:
          "OpenHands is rated native for self-hosting because it is open source and public docs/repository support running it outside a vendor-hosted service.",
        sources: [
          source("OpenHands GitHub", "https://github.com/All-Hands-AI/OpenHands"),
          source("OpenHands docs", "https://docs.all-hands.dev/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: note(
        "OpenHands is rated none for governance because its docs focus on local/cloud coding-agent execution, CLI/IDE use, and sandboxes rather than enterprise RBAC, approvals, audit trails, or policy administration.",
        [
          source("OpenHands docs", "https://docs.openhands.dev/"),
          source("OpenHands sandbox overview", "https://docs.openhands.dev/usage/runtimes/overview"),
        ],
      ),
      lock_in: note(
        "OpenHands is rated low lock-in because it is open-source, self-hostable, and works on ordinary repositories, while conversations, prompts, sandbox setup, and model-provider behavior may still be tool-specific.",
        [
          source("OpenHands GitHub", "https://github.com/All-Hands-AI/OpenHands"),
          source("OpenHands CLI installation", "https://docs.openhands.dev/openhands/usage/cli/installation"),
        ],
      ),
      sandbox_isolation: {
        summary:
          "OpenHands is now rated native for sandbox isolation because the Docker Runtime is documented as the default runtime and is explicitly designed to execute agent actions in an isolated Docker environment.",
        caveat:
          "OpenHands also documents Local Runtime for controlled environments without Docker, so deployments can weaken isolation if configured that way.",
        sources: [
          source("OpenHands runtime architecture", "https://docs.all-hands.dev/openhands/usage/architecture/runtime"),
          source("OpenHands runtime overview", "https://docs.all-hands.dev/usage/runtimes"),
          source("OpenHands local runtime", "https://docs.all-hands.dev/modules/usage/runtimes/local"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "OpenHands remains rated none for concurrent state because public docs focus on coding-agent runtime/workspace execution, not transactional multi-user application state.",
        sources: [
          source("OpenHands runtime architecture", "https://docs.all-hands.dev/openhands/usage/architecture/runtime"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Goose: {
    summary:
      "Goose is Block's open-source AI agent for local developer workflows with CLI/desktop surfaces, extensions, and model-provider flexibility. It is local coding-agent infrastructure, not business app delivery.",
    sources: [
      source("Goose docs", "https://block.github.io/goose/"),
      source("Goose desktop", "https://block.github.io/goose/docs/getting-started/installation/"),
      source("Goose extensions", "https://block.github.io/goose/docs/guides/using-extensions/"),
      source("Goose GitHub", "https://github.com/block/goose"),
    ],
    cells: {
      ...codingAgentCells("Goose", [
        source("Goose docs", "https://block.github.io/goose/"),
        source("Goose installation", "https://block.github.io/goose/docs/getting-started/installation/"),
        source("Goose GitHub", "https://github.com/block/goose"),
      ]),
      visual_workflow: note(
        "Goose is rated none for visual workflow building because its documented surfaces are Desktop, CLI, API, extensions, recipes, and subagents rather than a visual process-builder canvas.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose recipes", "https://block.github.io/goose/docs/tutorials/recipes-tutorial"),
          source("Goose extensions", "https://block.github.io/goose/docs/getting-started/using-extensions"),
        ],
      ),
      desktop: {
        summary:
          "Goose is rated partial for desktop because Goose provides local CLI/desktop-style developer surfaces, but not a packaged customer desktop app runtime.",
        sources: [
          source("Goose installation", "https://block.github.io/goose/docs/getting-started/installation/"),
          source("Goose GitHub", "https://github.com/block/goose"),
        ],
        checkedAt: "2026-05-30",
      },
      replayable: note(
        "Goose is rated partial for replayability because sessions, recipes, and CLI providers support persistence and repeatable prompts, but Goose does not provide deterministic replay of completed agent/tool execution.",
        [
          source("Goose recipes", "https://block.github.io/goose/docs/tutorials/recipes-tutorial"),
          source("Goose CLI providers", "https://block.github.io/goose/docs/guides/cli-providers"),
        ],
      ),
      high_volume: note(
        "Goose is rated none for big-data throughput because it is a local general-purpose AI agent with extensions and subagents, not a fleet-scale data-processing or workflow-execution platform.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose extensions", "https://block.github.io/goose/docs/getting-started/using-extensions"),
        ],
      ),
      compiled: note(
        "Goose is rated none for compiled business logic because Goose itself is a compiled local agent, but it does not compile user business workflows into portable deployable logic.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose recipes", "https://block.github.io/goose/docs/tutorials/recipes-tutorial"),
        ],
      ),
      file_size: note(
        "Goose's file-limit cell stays depends because local workspace size, recipe inputs, extension payloads, model context, and provider limits all shape practical file handling rather than one Goose upload cap.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose extensions", "https://block.github.io/goose/docs/getting-started/using-extensions"),
          source("Goose CLI providers", "https://block.github.io/goose/docs/guides/cli-providers"),
        ],
      ),
      ai_agents: {
        summary:
          "Goose is rated native for AI agents because Block documents Goose as an open-source AI agent that can operate through a local CLI/desktop surface, use extensions, and work with developer tools.",
        caveat:
          "This is a local developer agent, not a business workflow or app-delivery platform.",
        sources: [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose extensions", "https://block.github.io/goose/docs/guides/using-extensions/"),
          source("Goose GitHub", "https://github.com/block/goose"),
        ],
        checkedAt: "2026-05-30",
      },
      ui_builder: note(
        "Goose is rated none for end-user UI building because MCP-UI extensions can render interactive components inside Goose Desktop, but Goose does not provide a no-code app-screen builder or app runtime.",
        [
          source("Goose MCP-UI extensions", "https://block.github.io/goose/docs/guides/interactive-chat/mcp-ui/"),
          source("Goose docs", "https://block.github.io/goose/"),
        ],
      ),
      full_apps: note(
        "Goose is rated none for shipping full applications because it can help build code and run recipes, while app packaging, hosting, and distribution remain outside Goose.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose recipes", "https://block.github.io/goose/docs/tutorials/recipes-tutorial"),
        ],
      ),
      customer_facing: note(
        "Goose is rated none for customer-facing delivery because Desktop, CLI, API, and extensions are agent surfaces; customer exposure comes from whatever software Goose helps create.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose extensions", "https://block.github.io/goose/docs/getting-started/using-extensions"),
        ],
      ),
      offline: note(
        "Goose is rated partial for offline use because the agent, recipes, and local tools run on the user's machine, but most model providers and many extensions still require network access.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose CLI providers", "https://block.github.io/goose/docs/guides/cli-providers"),
        ],
      ),
      local_first: {
        summary:
          "Goose is rated native for local-first behavior because it runs against local developer workspaces and can use local tools/extensions.",
        caveat:
          "Model inference can still depend on external model providers.",
        sources: [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose extensions", "https://block.github.io/goose/docs/guides/using-extensions/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: note(
        "Goose is rated low lock-in because it is open source, uses ordinary local files and recipes, supports MCP extensions, and works with many model providers, even though sessions and extension behavior can still be Goose-specific.",
        [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose recipes", "https://block.github.io/goose/docs/tutorials/recipes-tutorial"),
          source("Goose CLI providers", "https://block.github.io/goose/docs/guides/cli-providers"),
        ],
      ),
      sandbox_isolation: {
        summary:
          "Goose is rated partial for sandbox isolation because it can be configured with tools/extensions and user-controlled execution, but public docs do not make a hardened sandbox the default execution model.",
        sources: [
          source("Goose extensions", "https://block.github.io/goose/docs/guides/using-extensions/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Goose is rated none for governance because the open-source local agent docs do not provide enterprise admin, audit, policy, or access-control controls by default.",
        sources: [
          source("Goose docs", "https://block.github.io/goose/"),
          source("Goose GitHub", "https://github.com/block/goose"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Gemini CLI": {
    summary:
      "Gemini CLI is Google's open-source AI agent in the terminal with tool access, MCP support, and Google model integration. It is developer-agent tooling, not an app/workflow platform for business users.",
    sources: [
      source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
      source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
      source("Gemini CLI checkpointing", "https://google-gemini.github.io/gemini-cli/docs/checkpointing/"),
      source("Gemini CLI sandboxing", "https://google-gemini.github.io/gemini-cli/docs/sandboxing/"),
      source("Gemini CLI enterprise", "https://google-gemini.github.io/gemini-cli/docs/enterprise/"),
      source("Gemini CLI GitHub", "https://github.com/google-gemini/gemini-cli"),
    ],
    cells: {
      ...codingAgentCells("Gemini CLI", [
        source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
        source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        source("Gemini CLI checkpointing", "https://google-gemini.github.io/gemini-cli/docs/checkpointing/"),
        source("Gemini CLI sandboxing", "https://google-gemini.github.io/gemini-cli/docs/sandboxing/"),
        source("Gemini CLI enterprise", "https://google-gemini.github.io/gemini-cli/docs/enterprise/"),
        source("Gemini CLI GitHub", "https://github.com/google-gemini/gemini-cli"),
      ]),
      visual_workflow: note(
        "Gemini CLI is rated none for visual workflow building because its documented surface is a terminal agent with slash commands, tools, checkpointing, sandboxing, and enterprise configuration rather than a visual process canvas.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
      ),
      replayable: {
        summary:
          "Gemini CLI is now rated partial for replayability because the docs include checkpointing to save project state before tool changes and restore snapshots with `/restore`, but this is not deterministic workflow replay.",
        sources: [
          source("Gemini CLI checkpointing", "https://google-gemini.github.io/gemini-cli/docs/checkpointing/"),
          source("Gemini CLI commands", "https://google-gemini.github.io/gemini-cli/docs/cli/commands/"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Gemini CLI is rated native for AI agents because it is an agentic terminal tool with built-in tools for interacting with codebases and local files.",
        sources: [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: note(
        "Gemini CLI is rated none for big-data throughput because the docs describe an interactive/headless terminal agent with local tools, not a distributed workflow or data-processing runtime.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
      ),
      compiled: note(
        "Gemini CLI is rated none for compiled business logic because it can inspect and edit code through tools, but it does not compile its own workflow definitions into deployable business logic.",
        [
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
          source("Gemini CLI file-system tools", "https://google-gemini.github.io/gemini-cli/docs/tools/file-system.html"),
        ],
      ),
      file_size: note(
        "Gemini CLI's file-limit cell stays depends because local file-system tools, glob reads, model context, checkpoint snapshots, sandbox configuration, and Gemini API limits shape practical handling rather than one CLI upload ceiling.",
        [
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
          source("Gemini CLI file-system tools", "https://google-gemini.github.io/gemini-cli/docs/tools/file-system.html"),
          source("Gemini CLI checkpointing", "https://google-gemini.github.io/gemini-cli/docs/checkpointing/"),
        ],
      ),
      ui_builder: note(
        "Gemini CLI is rated none for end-user UI building because its documented surface is a CLI with commands, tools, checkpointing, and enterprise configuration rather than a no-code app-screen builder.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
      ),
      full_apps: note(
        "Gemini CLI is rated none for shipping full applications because it can help create or modify application code, while hosting, packaging, distribution, and runtime operations remain outside the CLI.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
      ),
      customer_facing: note(
        "Gemini CLI is rated none for customer-facing delivery because customers interact with the software the CLI helps build, not with Gemini CLI as an app-hosting or portal product.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI GitHub", "https://github.com/google-gemini/gemini-cli"),
        ],
      ),
      desktop: note(
        "Gemini CLI is rated partial for desktop because it runs as a local terminal tool and can operate on local projects, but it does not package native desktop applications for end users.",
        [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/docs/cli/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
      ),
      offline: note(
        "Gemini CLI is rated none for offline execution because the core architecture communicates with the Gemini API for model inference, even though file-system tools run against local projects.",
        [
          source("Gemini CLI architecture", "https://google-gemini.github.io/gemini-cli/docs/architecture.html"),
          source("Gemini CLI core", "https://google-gemini.github.io/gemini-cli/docs/core/"),
        ],
      ),
      self_hosted: {
        summary:
          "Gemini CLI is now rated native for self-hosting/local execution because it is an open-source CLI that runs in the user's local environment, even though model calls use remote providers.",
        sources: [
          source("Gemini CLI GitHub", "https://github.com/google-gemini/gemini-cli"),
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
        ],
        checkedAt: "2026-05-30",
      },
      local_first: {
        summary:
          "Gemini CLI is now rated partial for local-first behavior because the agent runs locally and works with local files, but model inference and account integration are remote.",
        sources: [
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Gemini CLI is rated partial for sandbox isolation because sandboxing is documented, but users still need to configure and trust the selected sandbox/tool permissions for local command execution.",
        sources: [
          source("Gemini CLI sandboxing", "https://google-gemini.github.io/gemini-cli/docs/sandboxing/"),
          source("Gemini CLI tools", "https://google-gemini.github.io/gemini-cli/docs/tools/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Gemini CLI is now rated basic for governance because official docs describe enterprise configuration and OpenTelemetry observability, but not a full enterprise admin, approval, or policy-control plane.",
        sources: [
          source("Gemini CLI enterprise", "https://google-gemini.github.io/gemini-cli/docs/enterprise/"),
          source("Gemini CLI telemetry", "https://google-gemini.github.io/gemini-cli/docs/telemetry/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Gemini CLI is still rated high lock-in at the product level because the official experience is centered on Google's Gemini models and account/API ecosystem, despite the CLI itself being open source.",
        sources: [
          source("Gemini CLI docs", "https://google-gemini.github.io/gemini-cli/"),
          source("Gemini CLI GitHub", "https://github.com/google-gemini/gemini-cli"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  "Internal dev teams": {
    summary:
      "Internal dev teams can build almost anything with enough budget and time, but every capability depends on implementation quality, operating model, and maintenance capacity.",
    sources: [
      source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
      source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
    ],
    cells: {
      ...customBuildCells("Internal dev teams", "internal product engineering", [
        source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
      ]),
      replayable: {
        summary:
          "Internal dev teams are rated depends for replayability because durable execution, retry, and audit replay only exist if the team explicitly builds or adopts them.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Internal dev teams are rated depends for high-volume throughput because owned teams can build queues, workers, object storage, backpressure, and data platforms, but those capabilities are architecture and operations work rather than a default product feature.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Internal dev teams are rated depends for AI agents because memory, tool permissions, evaluation, monitoring, and human approval controls have to be selected, implemented, and operated by the team.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Internal dev teams are rated depends for governance because access control, audit, policy, and compliance controls depend on the architecture, SDLC, and operating model the team implements.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Internal dev teams are rated depends for sandbox isolation because secure code/tool execution requires deliberate isolation design, threat modeling, and implementation.",
        sources: [
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Internal dev teams are rated depends for concurrent state because transactional safety depends on the chosen database, queueing model, locking strategy, and implementation quality.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Agencies: {
    summary:
      "Agencies can deliver custom apps and integrations, but long-term portability, governance, and lock-in depend on contract terms, architecture choices, and who owns the source and operations.",
    sources: [
      source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
      source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
    ],
    cells: {
      ...customBuildCells("Agencies", "external delivery contracts and agency-operated implementation", [
        source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
      ]),
      replayable: {
        summary:
          "Agencies are rated depends for replayability because the contract can require durable execution, run history, retries, or audit replay, but those guarantees only exist if the agency chooses and implements the right runtime.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Agencies are rated depends for high-volume throughput because an agency can deliver scalable queues, workers, and storage, but capacity and reliability depend on architecture scope, handoff, and operations.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Agencies are rated depends for AI agents because agent behavior, tool boundaries, approvals, evaluation, and monitoring have to be specified in the brief and maintained after delivery.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Agencies are rated high lock-in because the real migration risk is often contract, source-code ownership, implementation knowledge, and operational handoff rather than only the technology stack.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Agencies are rated depends for governance because controls depend on the delivered architecture, client requirements, agency process, and post-handoff operations.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Agencies are rated depends for sandbox isolation because secure agent/tool execution must be specified, implemented, and tested as part of the custom build.",
        sources: [
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Agencies are rated depends for concurrent state because the delivered system may or may not include transactional workflow state, idempotency, and safe concurrent-write handling.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
  Contractors: {
    summary:
      "Contractors can implement custom capabilities, but continuity, governance, security, and concurrent-state guarantees depend on project scope and code ownership.",
    sources: [
      source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
      source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
    ],
    cells: {
      ...customBuildCells("Contractors", "contracted implementation by a small external team or individual", [
        source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
      ]),
      replayable: {
        summary:
          "Contractors are rated depends for replayability because a contractor may add job history, retries, or workflow replay, but that is only present when it is explicitly scoped, built, tested, and documented.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      high_volume: {
        summary:
          "Contractors are rated depends for high-volume throughput because scalable queues, background workers, storage, and observability can be built, but they are risky to leave implicit in a small-scope contract.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      ai_agents: {
        summary:
          "Contractors are rated depends for AI agents because safe tool access, permissions, evals, monitoring, and fallback behavior must be designed and maintained rather than assumed from custom-code delivery.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      lock_in: {
        summary:
          "Contractors are rated high lock-in because continuity and portability can depend on a small number of people, undocumented decisions, and who owns operational knowledge after delivery.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
      governance: {
        summary:
          "Contractors are rated depends for governance because admin controls, audit, SDLC, and compliance are only present if they are explicitly scoped and implemented.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      sandbox_isolation: {
        summary:
          "Contractors are rated depends for sandbox isolation because isolated execution has to be architected and validated as part of the custom system.",
        sources: [
          source("OWASP ASVS", "https://owasp.org/www-project-application-security-verification-standard/"),
        ],
        checkedAt: "2026-05-30",
      },
      concurrent_state: {
        summary:
          "Contractors are rated depends for concurrent state because safe concurrent writes require explicit implementation choices and testing.",
        sources: [
          source("NIST secure software development", "https://csrc.nist.gov/pubs/sp/800/218/final"),
        ],
        checkedAt: "2026-05-30",
      },
    },
  },
};

const supportMeaning: Record<string, string> = {
  native: "Native means the product publicly documents this as a first-class capability.",
  partial: "Partial means the product has adjacent support, but with scope, deployment, or product-fit limits.",
  none: "None means the reviewed public docs do not show this as a supported product capability.",
  high: "High means the comparison treats this as a material risk or limitation.",
  low: "Low means the comparison treats this as a comparatively low risk.",
  saas: "SaaS means the capability depends on the vendor-hosted service.",
  basic: "Basic means the product has some governance or control surface but not a broad enterprise control plane.",
  enterprise: "Enterprise means the capability is primarily documented as an enterprise-grade or admin/governance feature.",
  js: "JS means the product mainly exposes this through JavaScript or Node.js code paths.",
  unlimited: "Unlimited means no practical product-level cap was identified in the reviewed public sources.",
  depends: "Depends means the answer is implementation-specific rather than a fixed product capability.",
};

const supportRationale: Record<string, string> = {
  native: "the reviewed sources show this as a first-party product capability.",
  partial: "the reviewed sources show adjacent support, but with scope, deployment, or product-fit limits.",
  none: "the reviewed public sources do not show this as a supported product capability.",
  high: "the comparison treats this as a material product or migration risk.",
  low: "the comparison treats this as a comparatively low product or migration risk.",
  saas: "the capability depends on the vendor-hosted service.",
  basic: "the reviewed sources show some controls, but not a broad enterprise control plane.",
  enterprise: "the reviewed sources position this as an enterprise-grade admin, security, or governance capability.",
  js: "the product mainly exposes this through JavaScript or Node.js code paths.",
  unlimited: "no practical product-level cap was identified in the reviewed public sources.",
  depends: "the real answer depends on deployment, configuration, integration path, or implementation choices.",
};

const capabilityMeaning: Record<string, string> = {
  visual_workflow: "This cell looks for a visual workflow/canvas builder rather than only code-defined pipelines.",
  replayable: "This cell looks for durable replay, run history, retries, or reconstruction of workflow execution.",
  high_volume: "This cell looks for scale characteristics suitable for large workloads, queues, datasets, or enterprise automation.",
  compiled: "This cell looks for compiled or native business logic rather than interpreted scripts or hosted configuration.",
  file_size: "This cell looks at file/payload handling limits and whether large documents or datasets are a normal use case.",
  file_native: "This cell looks for file-native workflows where files are first-class local or project artifacts.",
  data_science: "This cell looks for data, analytics, ML, RAG, or pipeline use cases beyond simple task automation.",
  ai_agents: "This cell looks for first-class AI agents, tools, memory, or agent orchestration.",
  ui_builder: "This cell looks for custom UI construction such as forms, dashboards, and app screens.",
  full_apps: "This cell looks for shipping complete apps, not only dashboards, bots, workflows, or connectors.",
  customer_facing: "This cell looks for safe delivery to external users or customers rather than only internal operators.",
  desktop: "This cell looks for desktop app/runtime support, not just browser access.",
  mobile: "This cell looks for mobile app/runtime support.",
  offline: "This cell looks for offline execution or offline-capable user experiences.",
  local_first: "This cell looks for an architecture where data and projects can primarily live locally or under the customer's direct control.",
  governance: "This cell looks for admin, audit, security, policy, access-control, and compliance controls.",
  self_hosted: "This cell looks for self-hosting, on-prem, VPC, hybrid, or customer-controlled deployment.",
  lock_in: "This cell looks at practical migration risk around data, workflow definitions, runtime, and vendor platform coupling.",
  sandbox_isolation: "This cell looks for isolation around tool/code execution and untrusted automation.",
  concurrent_state: "This cell looks for transactional or otherwise safe handling of concurrent workflow/application state.",
};

const formatSupportValue = (support: Support): string => {
  if (typeof support === "string" && support.startsWith("limited-")) {
    return `limited (${support.replace("limited-", "")})`;
  }
  return supportMeaning[support] ? support : String(support);
};

const getSupportNote = (comp: Competitor, cap: string, support: Support): SupportNote => {
  const explicitNote = comp.notes?.[cap];
  if (explicitNote) return explicitNote;

  const research = competitorResearch[comp.name];
  const researchedCell = research?.cells?.[cap];
  if (researchedCell) {
    return {
      ...researchedCell,
      sources: researchedCell.sources ?? research.sources,
      checkedAt: researchedCell.checkedAt ?? "2026-05-30",
    };
  }

  const supportText = formatSupportValue(support);
  const supportExplanation = typeof support === "string" && support.startsWith("limited-")
    ? "public materials or matrix research identify a concrete file/payload cap for this product."
    : supportRationale[support] ?? "the current matrix research supports this rating.";

  return {
    summary: `${comp.name} gets a ${supportText} rating for ${t(`compare.cap.${cap}`)} because ${supportExplanation}`,
    evidence: `${research?.summary ?? "This row is based on public product documentation and the current comparison model."} ${capabilityMeaning[cap] ?? ""}`.trim(),
    caveat:
      "Public documentation does not always expose private enterprise limits or custom deployment terms; this note reflects reviewed public sources.",
    sources: research?.sources ?? [],
    checkedAt: "2026-05-30",
  };
};

const categories: Category[] = [
  {
    name: t("compare.category.execution"),
    desc: t("compare.category.execution.desc"),
    competitors: [
      {
        name: "Zapier",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-100mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "saas",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
        notes: {
          file_size: {
            summary:
              "Zapier is better represented as a 100 MB practical file limit for Zaps and Agents, not the 25 MB form-upload limit that was previously in this matrix.",
            evidence:
              "Zapier's file-sending docs say files larger than 100 MB may timeout and Zapier has a 150 MB file hydration limit. Zapier Agents knowledge files are documented up to 100 MB. Zapier Forms separately documents 5 MB on Free and 25 MB on paid plans for form uploads, which is narrower than the general Zap/Agent file case.",
            caveat:
              "Individual connected apps can impose their own stricter limits, so this remains a practical platform-level comparison rather than a guarantee for every Zap action.",
            sources: [
              source("Send files in Zaps", "https://help.zapier.com/hc/en-us/articles/8496288813453-Send-files-in-Zaps"),
              source("Zapier Agents knowledge files", "https://help.zapier.com/hc/en-us/articles/24569690575117-Add-your-own-data-to-an-agent"),
              source("Zapier Forms file uploads", "https://help.zapier.com/hc/en-us/articles/32816445802893-Use-the-file-upload-field-type-in-Zapier-Interfaces-forms"),
            ],
            checkedAt: "2026-05-30",
          },
          replayable: {
            summary:
              "Zapier is now rated partial for replayability because Zap history supports replaying unsuccessful runs and held runs can be replayed after task or flood-protection limits clear.",
            caveat:
              "This is operational replay/retry inside Zapier history, not deterministic workflow replay from an event log.",
            sources: [
              source("Zap limits", "https://help.zapier.com/hc/en-us/articles/8496181445261-Zap-limits"),
              source("Zap history", "https://help.zapier.com/hc/en-us/articles/8496291148685-View-and-manage-your-Zap-history"),
            ],
            checkedAt: "2026-05-30",
          },
          ai_agents: {
            summary:
              "Zapier is now rated native for AI agents because Zapier Agents is a first-party product with knowledge sources, actions, and hosted agent workflows inside Zapier.",
            caveat:
              "The agent runtime is native to Zapier's hosted platform, not portable or local-first.",
            sources: [
              source("Zapier Agents knowledge files", "https://help.zapier.com/hc/en-us/articles/24569690575117-Add-your-own-data-to-an-agent"),
              source("Zapier Agents", "https://zapier.com/agents"),
            ],
            checkedAt: "2026-05-30",
          },
          ui_builder: {
            summary:
              "Zapier is now rated partial for UI building because Zapier Interfaces supports interactive pages, forms, tables, and app-like experiences linked to Zaps and Tables, but it is not a full general app UI runtime.",
            sources: [
              source("Zapier Interfaces", "https://help.zapier.com/hc/en-us/articles/14490267815949-Create-interactive-pages-and-apps-with-Zapier-Interfaces-Beta-"),
              source("Zapier Tables", "https://help.zapier.com/hc/en-us/articles/8496297232781-Create-and-use-Zapier-Tables"),
            ],
            checkedAt: "2026-05-30",
          },
          customer_facing: {
            summary:
              "Zapier is now rated partial for customer-facing delivery because Interfaces and Forms can collect external input and connect it to automations, but Zapier does not package complete customer applications.",
            sources: [
              source("Zapier Interfaces", "https://help.zapier.com/hc/en-us/articles/14490267815949-Create-interactive-pages-and-apps-with-Zapier-Interfaces-Beta-"),
              source("Zapier Forms file uploads", "https://help.zapier.com/hc/en-us/articles/32816445802893-Use-the-file-upload-field-type-in-Zapier-Interfaces-forms"),
            ],
            checkedAt: "2026-05-30",
          },
        },
      },
      {
        name: "n8n",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "js",
          file_size: "limited-200mb",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
      {
        name: "Node-RED",
        capabilities: {
          visual_workflow: "native",
          replayable: "none",
          high_volume: "none",
          compiled: "js",
          file_size: "depends",
          ai_agents: "none",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "none",
        },
      },
    ],
  },
  {
    name: t("compare.category.lowcode"),
    desc: t("compare.category.lowcode.desc"),
    competitors: [
      {
        name: "Retool",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-40mb",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "none",
          mobile: "native",
          offline: "partial",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "enterprise",
          self_hosted: "native",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Power Apps",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-10gb",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "partial",
          mobile: "native",
          offline: "native",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "enterprise",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Superblocks",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-50mb",
          ai_agents: "partial",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Appsmith",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "enterprise",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.bi_analytics"),
    desc: t("compare.category.bi_analytics.desc"),
    competitors: [
      {
        name: "Tableau",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "partial",
          ui_builder: "native",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "partial",
          mobile: "partial",
          offline: "partial",
          local_first: "none",
          file_native: "partial",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Power BI",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-10gb",
          ai_agents: "partial",
          ui_builder: "native",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "partial",
          mobile: "partial",
          offline: "partial",
          local_first: "none",
          file_native: "partial",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Looker",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "partial",
          ui_builder: "native",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.orchestration"),
    desc: t("compare.category.orchestration.desc"),
    competitors: [
      {
        name: "Airflow",
        capabilities: {
          visual_workflow: "none",
          replayable: "native",
          high_volume: "native",
          compiled: "native",
          file_size: "depends",
          ai_agents: "none",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "native",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Temporal",
        capabilities: {
          visual_workflow: "none",
          replayable: "native",
          high_volume: "native",
          compiled: "native",
          file_size: "limited-2mb",
          ai_agents: "none",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "native",
          governance: "enterprise",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "native",
        },
      },
    ],
  },
  {
    name: t("compare.category.enterprise_data"),
    desc: t("compare.category.enterprise_data.desc"),
    competitors: [
      {
        name: "Ontology data platform",
        examples: t("compare.competitor.ontology.examples"),
        exampleNote: t("compare.competitor.ontology.note"),
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "native",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "partial",
          customer_facing: "none",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "native",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "native",
        },
      },
      {
        name: "ERP process platform",
        examples: t("compare.competitor.erp.examples"),
        exampleNote: t("compare.competitor.erp.note"),
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-50mb",
          ai_agents: "partial",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "native",
        },
      },
    ],
  },
  {
    name: t("compare.category.enterprise_automation"),
    desc: t("compare.category.enterprise_automation.desc"),
    competitors: [
      {
        name: "ServiceNow",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-1024mb",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Salesforce",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-2gb",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "none",
          mobile: "native",
          offline: "partial",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "native",
        },
      },
      {
        name: "Regrello",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.rpa"),
    desc: t("compare.category.rpa.desc"),
    competitors: [
      {
        name: "UiPath",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-10mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "partial",
          customer_facing: "partial",
          desktop: "partial",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Automation Anywhere",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-50mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Blue Prism",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "partial",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.compliance_grc"),
    desc: t("compare.category.compliance_grc.desc"),
    competitors: [
      {
        name: "ServiceNow GRC",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-1024mb",
          ai_agents: "native",
          ui_builder: "native",
          full_apps: "partial",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Archer",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "none",
          ui_builder: "partial",
          full_apps: "partial",
          customer_facing: "none",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "OneTrust",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "partial",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.agent_runtimes"),
    desc: t("compare.category.agent_runtimes.desc"),
    competitors: [
      {
        name: "CrewAI",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "native",
          data_science: "partial",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
      {
        name: "AutoGen",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "LangGraph",
        capabilities: {
          visual_workflow: "partial",
          replayable: "native",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "partial",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
      {
        name: "Dify",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-15mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Langdock",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-256mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "partial",
          mobile: "partial",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
        notes: {
          visual_workflow: {
            summary:
              "Langdock documents a visual drag-and-drop workflow builder with triggers, agent nodes, action nodes, conditions, loops, HTTP requests, and file-search nodes.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock workflows introduction",
                url: "https://docs.langdock.com/product/workflows/introduction",
              },
            ],
            checkedAt: "2026-05-30",
          },
          file_size: {
            summary:
              "Langdock supports large office documents, but the limits are format-specific and action execution has a separate total-file cap.",
            evidence:
              "Docs list 256 MB for PDFs/DOCX/PPTX, 30 MB for spreadsheets, 20 MB for images, 200 MB for audio, and a 100 MB total-file limit for custom action execution.",
            sources: [
              {
                label: "Supported file types",
                url: "https://docs.langdock.com/resources/faq/supported-file-types",
              },
              {
                label: "File support for actions",
                url: "https://docs.langdock.com/resources/integrations/file-support-for-actions",
              },
            ],
            checkedAt: "2026-05-30",
          },
          ai_agents: {
            summary:
              "Agents are a first-class Langdock product with saved instructions, knowledge, actions, sharing, analytics, Slack/Teams/API exposure, and MCP support.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock Agent MCP server",
                url: "https://docs.langdock.com/resources/integrations/langdock-agent-mcp-server",
              },
            ],
            checkedAt: "2026-05-30",
          },
          ui_builder: {
            summary:
              "Langdock has visual builders for agents and AI workflows, but public docs do not describe a general-purpose app UI builder for custom forms, dashboards, or arbitrary application screens.",
            caveat:
              "This is a product-scope judgment, not a Langdock-published limitation.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
            ],
            checkedAt: "2026-05-30",
          },
          full_apps: {
            summary:
              "Langdock offers its own web, desktop PWA, mobile apps, agents, APIs, and workflows, but the docs reviewed do not describe exporting or shipping full standalone customer applications built on Langdock.",
            caveat:
              "Rated against Flow-Like's app-shipping capability, not against Langdock's ability to expose agents via Slack, Teams, or API.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
            ],
            checkedAt: "2026-05-30",
          },
          customer_facing: {
            summary:
              "Langdock agents can be exposed outside the core app through Slack, Microsoft Teams, and API/MCP-style access, but the docs reviewed do not show full customer-facing app delivery.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock Agent MCP server",
                url: "https://docs.langdock.com/resources/integrations/langdock-agent-mcp-server",
              },
            ],
            checkedAt: "2026-05-30",
          },
          desktop: {
            summary:
              "Langdock documents desktop use through an installable progressive web app rather than a native desktop application runtime.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock app shortcuts",
                url: "https://docs.langdock.com/resources/tricks-and-shortcuts",
              },
            ],
            checkedAt: "2026-05-30",
          },
          mobile: {
            summary:
              "Langdock offers dedicated iOS and Android apps. Public mobile copy says the mobile app includes Chat and Agents; workflows and workspace settings are directed to desktop.",
            sources: [
              {
                label: "Langdock mobile app",
                url: "https://langdock.com/mobile",
              },
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
            ],
            checkedAt: "2026-05-30",
          },
          offline: {
            summary:
              "Langdock is documented as a web/PWA/mobile platform with hosted, own-cloud, or on-prem deployment options, but its mobile and feature materials do not describe offline workflow execution.",
            caveat:
              "Rated as not supported unless Langdock publishes an offline runtime for workflows or agents.",
            sources: [
              {
                label: "Langdock mobile app",
                url: "https://langdock.com/mobile",
              },
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
            ],
            checkedAt: "2026-05-30",
          },
          local_first: {
            summary:
              "Langdock emphasizes enterprise deployment and connected app access; its feature and mobile materials do not describe a local-first project or data architecture where user data primarily lives on-device.",
            caveat:
              "This is an architectural inference from public docs.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock mobile app",
                url: "https://langdock.com/mobile",
              },
            ],
            checkedAt: "2026-05-30",
          },
          governance: {
            summary:
              "Langdock documents enterprise workspace administration, SSO/SCIM, granular integration/action permissions, usage analytics, custom retention, and SOC 2 Type II / ISO 27001 references.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock legal and compliance links",
                url: "https://docs.langdock.com/administration/legal-compliance",
              },
            ],
            checkedAt: "2026-05-30",
          },
          self_hosted: {
            summary:
              "Langdock advertises deployment options beyond the managed service, including own-cloud and on-prem deployment.",
            caveat:
              "Rated partial because this appears to be an enterprise/dedicated deployment option, not an open self-host package.",
            sources: [
              {
                label: "Langdock mobile app",
                url: "https://langdock.com/mobile",
              },
              {
                label: "Langdock pricing",
                url: "https://www.langdock.com/en/pricing",
              },
            ],
            checkedAt: "2026-05-30",
          },
          lock_in: {
            summary:
              "Langdock is model-agnostic and supports BYOK/BYOM patterns, but agents, workflows, workspace governance, and deployment are still centered on the Langdock platform.",
            caveat:
              "Rated high for platform/workflow lock-in, not model-provider lock-in.",
            sources: [
              {
                label: "Langdock feature overview",
                url: "https://docs.langdock.com/resources/feature-overview",
              },
              {
                label: "Langdock homepage",
                url: "https://www.langdock.com/",
              },
            ],
            checkedAt: "2026-05-30",
          },
          sandbox_isolation: {
            summary:
              "Langdock documents custom action and trigger code running in a secure sandbox with restricted libraries, but that sandbox is tied to Langdock actions rather than a portable isolation primitive for arbitrary tools.",
            sources: [
              {
                label: "File support for actions",
                url: "https://docs.langdock.com/resources/integrations/file-support-for-actions",
              },
            ],
            checkedAt: "2026-05-30",
          },
          concurrent_state: {
            summary:
              "Langdock provides hosted workflow execution and monitoring, but the workflow docs do not state transactional concurrent-write semantics for workflow or agent state.",
            caveat:
              "Rated partial because hosted products usually provide operational isolation, while the specific transactional guarantee is not documented in the reviewed public sources.",
            sources: [
              {
                label: "Langdock workflows product page",
                url: "https://www.langdock.com/products/workflows",
              },
              {
                label: "Langdock workflows introduction",
                url: "https://docs.langdock.com/product/workflows/introduction",
              },
            ],
            checkedAt: "2026-05-30",
          },
        },
      },
      {
        name: "Flowise",
        capabilities: {
          visual_workflow: "native",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-50mb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "partial",
          file_native: "none",
          data_science: "none",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
      {
        name: "Pydantic AI",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Agno",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "partial",
          governance: "basic",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Google ADK",
        capabilities: {
          visual_workflow: "partial",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "partial",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.llm_frameworks"),
    desc: t("compare.category.llm_frameworks.desc"),
    competitors: [
      {
        name: "LangChain",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "none",
        },
      },
      {
        name: "LlamaIndex",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "native",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "none",
        },
      },
      {
        name: "Haystack",
        capabilities: {
          visual_workflow: "partial",
          replayable: "none",
          high_volume: "partial",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "partial",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "none",
          data_science: "native",
          governance: "enterprise",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "none",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.ai_ipaas"),
    desc: t("compare.category.ai_ipaas.desc"),
    competitors: [
      {
        name: "Make.com",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-1gb",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "basic",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Workato",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "partial",
          compiled: "none",
          file_size: "limited-10gb",
          ai_agents: "native",
          ui_builder: "partial",
          full_apps: "partial",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "partial",
          governance: "enterprise",
          self_hosted: "partial",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Pipedream",
        capabilities: {
          visual_workflow: "native",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "limited-5tb",
          ai_agents: "partial",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "partial",
          desktop: "none",
          mobile: "none",
          offline: "none",
          local_first: "none",
          file_native: "none",
          data_science: "none",
          governance: "basic",
          self_hosted: "none",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
    ],
  },
  {
    name: t("compare.category.coding_agents"),
    desc: t("compare.category.coding_agents.desc"),
    competitors: [
      {
        name: "OpenClaw",
        warning: t("compare.warning.openclaw"),
        capabilities: {
          visual_workflow: "none",
          replayable: "none",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "partial",
          mobile: "partial",
          offline: "partial",
          local_first: "native",
          file_native: "native",
          data_science: "none",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "Hermes Agent",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "partial",
          mobile: "none",
          offline: "partial",
          local_first: "native",
          file_native: "native",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "partial",
        },
      },
      {
        name: "OpenHands",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "none",
          mobile: "none",
          offline: "partial",
          local_first: "partial",
          file_native: "native",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "native",
          concurrent_state: "none",
        },
      },
      {
        name: "Goose",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "partial",
          mobile: "none",
          offline: "partial",
          local_first: "native",
          file_native: "native",
          data_science: "partial",
          governance: "none",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "partial",
          concurrent_state: "none",
        },
      },
      {
        name: "Gemini CLI",
        capabilities: {
          visual_workflow: "none",
          replayable: "partial",
          high_volume: "none",
          compiled: "none",
          file_size: "depends",
          ai_agents: "native",
          ui_builder: "none",
          full_apps: "none",
          customer_facing: "none",
          desktop: "partial",
          mobile: "none",
          offline: "none",
          local_first: "partial",
          file_native: "native",
          data_science: "partial",
          governance: "basic",
          self_hosted: "native",
          lock_in: "high",
          sandbox_isolation: "partial",
          concurrent_state: "none",
        },
      },
    ],
  },
  {
    name: t("compare.category.custom_development"),
    desc: t("compare.category.custom_development.desc"),
    competitors: [
      {
        name: "Internal dev teams",
        capabilities: {
          visual_workflow: "none",
          replayable: "depends",
          high_volume: "depends",
          compiled: "native",
          file_size: "depends",
          ai_agents: "depends",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "depends",
          mobile: "depends",
          offline: "depends",
          local_first: "depends",
          file_native: "depends",
          data_science: "depends",
          governance: "depends",
          self_hosted: "native",
          lock_in: "low",
          sandbox_isolation: "depends",
          concurrent_state: "depends",
        },
      },
      {
        name: "Agencies",
        capabilities: {
          visual_workflow: "none",
          replayable: "depends",
          high_volume: "depends",
          compiled: "native",
          file_size: "depends",
          ai_agents: "depends",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "depends",
          mobile: "depends",
          offline: "depends",
          local_first: "depends",
          file_native: "depends",
          data_science: "depends",
          governance: "depends",
          self_hosted: "depends",
          lock_in: "high",
          sandbox_isolation: "depends",
          concurrent_state: "depends",
        },
      },
      {
        name: "Contractors",
        capabilities: {
          visual_workflow: "none",
          replayable: "depends",
          high_volume: "depends",
          compiled: "native",
          file_size: "depends",
          ai_agents: "depends",
          ui_builder: "native",
          full_apps: "native",
          customer_facing: "native",
          desktop: "depends",
          mobile: "depends",
          offline: "depends",
          local_first: "depends",
          file_native: "depends",
          data_science: "depends",
          governance: "depends",
          self_hosted: "depends",
          lock_in: "high",
          sandbox_isolation: "depends",
          concurrent_state: "depends",
        },
      },
    ],
  },
];

const renderSupport = (support: Support): { icon: string; color: string; label?: string } => {
  if (support === "native") return { icon: "✓", color: "emerald" };
  if (support === "partial") return { icon: "◐", color: "amber" };
  if (support === "none") return { icon: "✗", color: "red" };
  if (support === "high") return { icon: "⚠", color: "red", label: t("compare.support.high") };
  if (support === "low") return { icon: "○", color: "emerald", label: t("compare.support.low") };
  if (support === "saas") return { icon: "◐", color: "amber", label: "SaaS" };
  if (support === "basic") return { icon: "◐", color: "amber", label: t("compare.support.basic") };
  if (support === "enterprise") return { icon: "◐", color: "amber", label: "Enterprise" };
  if (support === "js") return { icon: "◐", color: "amber", label: "JS" };
  if (support === "unlimited") return { icon: "∞", color: "emerald", label: t("compare.support.unlimited") };
  if (support === "depends") return { icon: "?", color: "muted", label: t("compare.support.depends") };
  if (typeof support === "string" && support.startsWith("limited-")) {
    const size = support.replace("limited-", "").toUpperCase();
    return { icon: "⚠", color: "red", label: size };
  }
  return { icon: support, color: "muted", label: support };
};

const supportScore = (support: Support): number => {
  if (support === "native" || support === "unlimited" || support === "low") return 1;
  if (support === "partial" || support === "js" || support === "enterprise") return 0.55;
  if (support === "saas" || support === "basic" || support === "depends") return 0.4;
  if (support === "none" || support === "high") return 0;
  if (typeof support === "string" && support.startsWith("limited-")) return 0.2;
  return 0.3;
};

// Per-category axis spec. Each category gets the two dimensions that actually
// differentiate its competitors, so the bubble plot tells a relevant story.
interface AxisSpec {
  caps: string[];
  labelKey: string;
  lowKey: string;
  highKey: string;
}

interface CategorySpec {
  x: AxisSpec;
  y: AxisSpec;
}

// Each category is plotted against THE two pain points its competitors share.
// Flow-Like lands top-right because it solves both — that's the story.
const categorySpecs: Record<string, CategorySpec> = {
  // SaaS workflow tools die at scale; open ones aren't production-grade.
  [t("compare.category.execution")]: {
    x: {
      caps: ["replayable", "high_volume", "compiled"],
      labelKey: "compare.matrix.plot.axis.production_scale",
      lowKey: "compare.matrix.plot.axis.production_scale.low",
      highKey: "compare.matrix.plot.axis.production_scale.high",
    },
    y: {
      caps: ["self_hosted", "lock_in"],
      labelKey: "compare.matrix.plot.axis.openness",
      lowKey: "compare.matrix.plot.axis.openness.low",
      highKey: "compare.matrix.plot.axis.openness.high",
    },
  },
  // Low-code is browser-locked internal tools — never reaches customers/mobile/offline.
  [t("compare.category.lowcode")]: {
    x: {
      caps: ["mobile", "offline", "desktop"],
      labelKey: "compare.matrix.plot.axis.customer_distribution",
      lowKey: "compare.matrix.plot.axis.customer_distribution.low",
      highKey: "compare.matrix.plot.axis.customer_distribution.high",
    },
    y: {
      caps: ["self_hosted", "lock_in"],
      labelKey: "compare.matrix.plot.axis.openness",
      lowKey: "compare.matrix.plot.axis.openness.low",
      highKey: "compare.matrix.plot.axis.openness.high",
    },
  },
  // BI tools explain what happened, but rarely own the operational workflow.
  [t("compare.category.bi_analytics")]: {
    x: {
      caps: ["file_size", "file_native", "data_science"],
      labelKey: "compare.matrix.plot.axis.data_headroom",
      lowKey: "compare.matrix.plot.axis.data_headroom.low",
      highKey: "compare.matrix.plot.axis.data_headroom.high",
    },
    y: {
      caps: ["visual_workflow", "full_apps", "customer_facing"],
      labelKey: "compare.matrix.plot.axis.business_ux",
      lowKey: "compare.matrix.plot.axis.business_ux.low",
      highKey: "compare.matrix.plot.axis.business_ux.high",
    },
  },
  // Engineer-only runtimes with no business UX and no AI-native primitives.
  [t("compare.category.orchestration")]: {
    x: {
      caps: ["visual_workflow", "ui_builder", "full_apps"],
      labelKey: "compare.matrix.plot.axis.business_ux",
      lowKey: "compare.matrix.plot.axis.business_ux.low",
      highKey: "compare.matrix.plot.axis.business_ux.high",
    },
    y: {
      caps: ["ai_agents", "data_science"],
      labelKey: "compare.matrix.plot.axis.ai_native",
      lowKey: "compare.matrix.plot.axis.ai_native.low",
      highKey: "compare.matrix.plot.axis.ai_native.high",
    },
  },
  // Enterprise data: powerful but internal-only AND locked behind 6-figure contracts.
  [t("compare.category.enterprise_data")]: {
    x: {
      caps: ["self_hosted", "lock_in"],
      labelKey: "compare.matrix.plot.axis.portability",
      lowKey: "compare.matrix.plot.axis.portability.low",
      highKey: "compare.matrix.plot.axis.portability.high",
    },
    y: {
      caps: ["customer_facing", "mobile", "offline"],
      labelKey: "compare.matrix.plot.axis.customer_reach",
      lowKey: "compare.matrix.plot.axis.customer_reach.low",
      highKey: "compare.matrix.plot.axis.customer_reach.high",
    },
  },
  // Legacy RPA stacks bolt "AI" on top of brittle screen-scraping, all SaaS-locked.
  [t("compare.category.enterprise_automation")]: {
    x: {
      caps: ["ai_agents", "data_science", "replayable"],
      labelKey: "compare.matrix.plot.axis.ai_native",
      lowKey: "compare.matrix.plot.axis.ai_native.low",
      highKey: "compare.matrix.plot.axis.ai_native.high",
    },
    y: {
      caps: ["self_hosted", "lock_in"],
      labelKey: "compare.matrix.plot.axis.portability",
      lowKey: "compare.matrix.plot.axis.portability.low",
      highKey: "compare.matrix.plot.axis.portability.high",
    },
  },
  // RPA automates screens, not durable, typed business systems.
  [t("compare.category.rpa")]: {
    x: {
      caps: ["replayable", "high_volume", "compiled"],
      labelKey: "compare.matrix.plot.axis.production_scale",
      lowKey: "compare.matrix.plot.axis.production_scale.low",
      highKey: "compare.matrix.plot.axis.production_scale.high",
    },
    y: {
      caps: ["ai_agents", "ui_builder", "customer_facing"],
      labelKey: "compare.matrix.plot.axis.ai_reach",
      lowKey: "compare.matrix.plot.axis.ai_reach.low",
      highKey: "compare.matrix.plot.axis.ai_reach.high",
    },
  },
  // GRC systems govern evidence, but they rarely run the underlying work.
  [t("compare.category.compliance_grc")]: {
    x: {
      caps: ["governance", "sandbox_isolation", "concurrent_state"],
      labelKey: "compare.matrix.plot.axis.agent_safety",
      lowKey: "compare.matrix.plot.axis.agent_safety.low",
      highKey: "compare.matrix.plot.axis.agent_safety.high",
    },
    y: {
      caps: ["visual_workflow", "replayable", "high_volume"],
      labelKey: "compare.matrix.plot.axis.production_readiness",
      lowKey: "compare.matrix.plot.axis.production_readiness.low",
      highKey: "compare.matrix.plot.axis.production_readiness.high",
    },
  },
  // Agent runtimes: zero sandboxing + file-based concurrent state = production death trap.
  [t("compare.category.agent_runtimes")]: {
    x: {
      caps: ["sandbox_isolation", "concurrent_state", "governance"],
      labelKey: "compare.matrix.plot.axis.agent_safety",
      lowKey: "compare.matrix.plot.axis.agent_safety.low",
      highKey: "compare.matrix.plot.axis.agent_safety.high",
    },
    y: {
      caps: ["high_volume", "replayable", "compiled"],
      labelKey: "compare.matrix.plot.axis.production_readiness",
      lowKey: "compare.matrix.plot.axis.production_readiness.low",
      highKey: "compare.matrix.plot.axis.production_readiness.high",
    },
  },
  // LLM frameworks are code-only SDK building blocks — no UX, no deployment story.
  [t("compare.category.llm_frameworks")]: {
    x: {
      caps: ["visual_workflow", "ui_builder", "full_apps"],
      labelKey: "compare.matrix.plot.axis.business_ux",
      lowKey: "compare.matrix.plot.axis.business_ux.low",
      highKey: "compare.matrix.plot.axis.business_ux.high",
    },
    y: {
      caps: ["high_volume", "compiled", "replayable"],
      labelKey: "compare.matrix.plot.axis.production_scale",
      lowKey: "compare.matrix.plot.axis.production_scale.low",
      highKey: "compare.matrix.plot.axis.production_scale.high",
    },
  },
  // AI-augmented iPaaS: cloud-only connector glue with thin AI wrappers, no real portability.
  [t("compare.category.ai_ipaas")]: {
    x: {
      caps: ["ai_agents", "data_science", "replayable"],
      labelKey: "compare.matrix.plot.axis.ai_native",
      lowKey: "compare.matrix.plot.axis.ai_native.low",
      highKey: "compare.matrix.plot.axis.ai_native.high",
    },
    y: {
      caps: ["self_hosted", "lock_in"],
      labelKey: "compare.matrix.plot.axis.portability",
      lowKey: "compare.matrix.plot.axis.portability.low",
      highKey: "compare.matrix.plot.axis.portability.high",
    },
  },
  // Personal AI agents: host-OS exposure + file-based state vs. Flow-Like's WASM sandbox + transactions.
  [t("compare.category.coding_agents")]: {
    x: {
      caps: ["sandbox_isolation", "concurrent_state", "governance"],
      labelKey: "compare.matrix.plot.axis.agent_safety",
      lowKey: "compare.matrix.plot.axis.agent_safety.low",
      highKey: "compare.matrix.plot.axis.agent_safety.high",
    },
    y: {
      caps: ["visual_workflow", "ui_builder", "full_apps"],
      labelKey: "compare.matrix.plot.axis.business_ux",
      lowKey: "compare.matrix.plot.axis.business_ux.low",
      highKey: "compare.matrix.plot.axis.business_ux.high",
    },
  },
  // Custom builds can do anything, but every capability has to be rebuilt and maintained.
  [t("compare.category.custom_development")]: {
    x: {
      caps: ["replayable", "high_volume", "compiled"],
      labelKey: "compare.matrix.plot.axis.production_scale",
      lowKey: "compare.matrix.plot.axis.production_scale.low",
      highKey: "compare.matrix.plot.axis.production_scale.high",
    },
    y: {
      caps: ["governance", "sandbox_isolation", "concurrent_state"],
      labelKey: "compare.matrix.plot.axis.agent_safety",
      lowKey: "compare.matrix.plot.axis.agent_safety.low",
      highKey: "compare.matrix.plot.axis.agent_safety.high",
    },
  },
};

const axisScore = (caps: Record<string, Support>, axis: AxisSpec): number => {
  if (axis.caps.length === 0) return 0;
  const sum = axis.caps.reduce((acc, k) => acc + supportScore(caps[k] ?? "none"), 0);
  return sum / axis.caps.length;
};

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

const competitorPosition = (
  caps: Record<string, Support>,
  spec: CategorySpec,
  index = 0,
  count = 1,
) => {
  // Map 0..1 to 10..78 to keep bubbles inside the chart frame.
  const baseX = 10 + axisScore(caps, spec.x) * 68;
  const baseY = 10 + axisScore(caps, spec.y) * 68;
  // Deterministic jitter so identical-cap competitors fan out instead of stacking.
  const spread = count > 1 ? 5 : 0;
  const jitterX = count > 1 ? (index - (count - 1) / 2) * spread : 0;
  const jitterY = count > 1 ? (((index * 7) % Math.max(count, 3)) - (count - 1) / 2) * (spread * 0.6) : 0;
  return {
    x: clamp(baseX + jitterX, 6, 88),
    y: clamp(baseY + jitterY, 6, 88),
  };
};

const categoryAccent: Record<string, string> = {
  [t("compare.category.execution")]: "amber",
  [t("compare.category.lowcode")]: "blue",
  [t("compare.category.bi_analytics")]: "lime",
  [t("compare.category.orchestration")]: "purple",
  [t("compare.category.enterprise_data")]: "fuchsia",
  [t("compare.category.enterprise_automation")]: "rose",
  [t("compare.category.rpa")]: "orange",
  [t("compare.category.compliance_grc")]: "emerald",
  [t("compare.category.agent_runtimes")]: "teal",
  [t("compare.category.llm_frameworks")]: "violet",
  [t("compare.category.ai_ipaas")]: "cyan",
  [t("compare.category.coding_agents")]: "sky",
  [t("compare.category.custom_development")]: "slate",
};

  return {
    capabilityGroups,
    flowLikeCapabilities,
    categories,
    renderSupport,
    getSupportNote,
    categorySpecs,
    competitorPosition,
    categoryAccent,
  };
}
