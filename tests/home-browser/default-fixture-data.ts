import {
	type IApp,
	IAppCategory,
	IAppExecutionMode,
	IAppStatus,
	IAppType,
	IAppVisibility,
} from "../../packages/ui/lib/schema/app/app";
import type { IBit, IMetadata } from "../../packages/ui/lib/schema/bit/bit";
import type { IExecutionUsageRecord } from "../../packages/ui/lib/schema/usage/tracking";
import type { PackageSummary } from "../../packages/ui/lib/schema/wasm";
import type { INotification } from "../../packages/ui/state/backend-state/types";

// These examples belong only to the local browser fixture. Artwork comes from
// existing product documentation so the page is reviewed with real media files.
const art = [
	new URL(
		"../../apps/website/public/images/flowpilot-whole-app.jpg",
		import.meta.url,
	).href,
	new URL("../../apps/docs/src/assets/DataStudioOverview.webp", import.meta.url)
		.href,
	new URL("../../apps/docs/src/assets/OntologyModel.webp", import.meta.url)
		.href,
	new URL("../../apps/docs/src/assets/FlowWithLayers.webp", import.meta.url)
		.href,
	new URL("../../apps/docs/src/assets/NodeCatalog.webp", import.meta.url).href,
	new URL("../../apps/docs/src/assets/PagesOverview.webp", import.meta.url)
		.href,
];
const logo = new URL(
	"../../apps/docs/src/assets/app-logo.webp",
	import.meta.url,
).href;
const now = Date.now();
const time = (days: number) => ({
	secs_since_epoch: Math.floor((now - days * 86_400_000) / 1000),
	nanos_since_epoch: 0,
});
const entries = [
	[
		"Knowledge Chat",
		"Ask your documents. Get answers with sources.",
		IAppCategory.Productivity,
		IAppType.Agent,
	],
	[
		"Invoice OCR",
		"Turn incoming invoices into records your team can use.",
		IAppCategory.Business,
		IAppType.DataPipeline,
	],
	[
		"Sheet Sync",
		"Keep customer records and spreadsheets up to date.",
		IAppCategory.Business,
		IAppType.DataFocus,
	],
	[
		"Webhook Relay",
		"Connect the tools you use with a reliable handoff.",
		IAppCategory.Utilities,
		IAppType.DataPipeline,
	],
	[
		"Toolbox",
		"Useful building blocks for your next automation.",
		IAppCategory.Utilities,
		IAppType.CustomInterface,
	],
	[
		"Field Notes",
		"Collect ideas and turn them into the next useful step.",
		IAppCategory.Productivity,
		IAppType.Form,
	],
] as const;

export const defaultFixtureApps: [IApp, IMetadata][] = entries.map(
	([name, description, category, appType], index) => [
		{
			id: `default-fixture-app-${index}`,
			authors: ["Flow-Like fixture team"],
			bits: [],
			boards: [],
			events: [],
			page_ids: [],
			widget_ids: [],
			templates: [],
			status: IAppStatus.Active,
			app_type: appType,
			visibility: IAppVisibility.Public,
			execution_mode: IAppExecutionMode.Any,
			primary_category: category,
			created_at: time(index + 1),
			updated_at: time(index / 8),
			download_count: [2480, 1860, 1240, 980, 750, 340][index],
			interactions_count: [9400, 7200, 3400, 2160, 1850, 600][index],
			rating_count: [28, 16, 21, 12, 8, 6][index],
			rating_sum: [138, 76, 103, 58, 39, 29][index],
			avg_rating: [4.9, 4.8, 4.9, 4.8, 4.9, 4.8][index],
		},
		{
			name,
			description,
			use_case: description,
			tags:
				index === 0
					? ["documents", "ai", "knowledge"]
					: ["automation", category.toLowerCase()],
			icon: logo,
			thumbnail: art[index],
			preview_media: [],
			created_at: time(index + 1),
			updated_at: time(index / 8),
		},
	],
);

export const defaultFixturePackages: PackageSummary[] = [
	[
		"Document toolkit",
		"Extract text, tables and structured fields from documents.",
		"documents",
	],
	[
		"Postgres connectors",
		"Read, write and sync your team's database records.",
		"database",
	],
	[
		"Message channels",
		"Bring events from your tools into a flow.",
		"communication",
	],
].map(([name, description, category], index) => ({
	id: `default-fixture-package-${index}`,
	name,
	description,
	latestVersion: ["1.4.0", "2.1.0", "1.2.3"][index],
	downloadCount: [8400, 6100, 3200][index],
	status: "active",
	keywords: [category],
	verified: true,
	price: 0,
	visibility: "public",
	primaryCategory: [
		"DOCUMENT_PROCESSING",
		"INTEGRATION_CONNECTORS",
		"COMMUNICATION",
	][index],
	metadata: { name, description, icon: logo, thumbnail: art[index + 1] },
	avgRating: [4.9, 4.8, 4.7][index],
	ratingCount: [18, 12, 9][index],
	capabilities: ["net.http", "storage.user", "oauth", "models"],
})) as PackageSummary[];

export const defaultFixtureModels = [
	{
		id: "s14lujkm2gut2mwg0zo3imxv",
		hash: "fixture-model-hash",
		hub: "api.flow-like.com",
		authors: [],
		dependencies: [],
		dependency_tree_hash: "fixture",
		created: "2026-09-05",
		updated: "2026-09-05",
		type: "Llm",
		size: 0,
		parameters: {
			context_length: 128000,
			provider: { provider_name: "openai", params: {} },
		},
		meta: {
			en: {
				name: "GLM 5",
				description: "A hosted model for working through documents and ideas.",
				icon: logo,
				tags: ["reasoning", "hosted"],
				preview_media: [],
				created_at: time(2),
				updated_at: time(1),
			},
		},
	},
] as unknown as IBit[];

export const defaultFixtureHistory: IExecutionUsageRecord[] = Array.from(
	{ length: 72 },
	(_, index) => ({
		id: `default-fixture-run-${index}`,
		app_id: defaultFixtureApps[index % 4][0].id,
		created_at: new Date(now - (index + 1) * 110 * 60_000).toISOString(),
		status: index % 19 === 0 ? "Error" : index % 13 === 0 ? "Warn" : "Info",
		microseconds: 230000 + (index % 9) * 42000,
		board_id: "fixture-board",
		node_id: "fixture-node",
		version: "1",
		instance: null,
		technical_user_id: null,
	}),
);

export const defaultFixtureNotifications: INotification[] = [
	{
		id: "default-fixture-notification",
		user_id: "fixture-user",
		title: "Invoice OCR needs a look",
		description:
			"One document could not be read. Review the recorded run to continue.",
		link: "/notifications",
		notification_type: "WORKFLOW",
		read: false,
		created_at: new Date(now - 25 * 60_000).toISOString(),
	},
];
