import {
	IAppCategory,
	IAppExecutionMode,
	IAppStatus,
	IAppType,
	IAppVisibility,
} from "../../../lib/schema/app/app";
import {
	PackageStatus,
	type WasmPackageCategory,
} from "../../../lib/schema/wasm";
import type {
	ExploreAnnouncement,
	ExploreCollection,
	ExploreCollectionSummary,
	ExploreRail,
	ExploreRailKey,
	ExploreResolvedItem,
	ExploreSearchResponse,
	ExploreView,
	ExploreViewer,
	ResolvedExplore,
} from "./explore-types";

type AppItem = Extract<ExploreResolvedItem, { kind: "app" }>;
type PackageItem = Extract<ExploreResolvedItem, { kind: "package" }>;

const FIXTURE_EPOCH_SECS = 1_790_000_000;

function systemTime(daysAgo: number) {
	return {
		secs_since_epoch: FIXTURE_EPOCH_SECS - daysAgo * 86_400,
		nanos_since_epoch: 0,
	};
}

function app(
	id: string,
	name: string,
	category: IAppCategory,
	appType: IAppType,
	rating: [average: number, count: number] | null,
	price: number,
	description: string,
	daysAgo: number,
): AppItem {
	const [average, ratingCount] = rating ?? [null, 0];
	return {
		kind: "app",
		app: {
			id,
			authors: [],
			bits: [],
			boards: [],
			events: [],
			page_ids: [],
			templates: [],
			widget_ids: [],
			created_at: systemTime(daysAgo),
			updated_at: systemTime(Math.max(daysAgo - 1, 0)),
			download_count: Math.round(10_000 / (daysAgo + 1)),
			interactions_count: 0,
			execution_mode: IAppExecutionMode.Any,
			status: IAppStatus.Active,
			visibility: IAppVisibility.Public,
			primary_category: category,
			app_type: appType,
			price,
			avg_rating: average,
			rating_count: ratingCount,
			rating_sum: Math.round((average ?? 0) * ratingCount),
		},
		metadata: {
			name,
			description,
			tags: [],
			preview_media: [],
			created_at: systemTime(daysAgo),
			updated_at: systemTime(Math.max(daysAgo - 1, 0)),
		},
	};
}

function pkg(
	id: string,
	name: string,
	options: {
		verified: boolean;
		version: string;
		installs: number;
		rating?: [average: number, count: number];
		price?: number;
		capabilities: string[];
		category: WasmPackageCategory;
		description: string;
	},
): PackageItem {
	return {
		kind: "package",
		package: {
			id,
			name,
			description: options.description,
			latestVersion: options.version,
			downloadCount: options.installs,
			status: PackageStatus.Active,
			keywords: [],
			verified: options.verified,
			price: options.price ?? 0,
			visibility: "public",
			primaryCategory: options.category,
			avgRating: options.rating?.[0] ?? null,
			ratingCount: options.rating?.[1] ?? 0,
			capabilities: options.capabilities,
		},
	};
}

export const FIXTURE_APPS = {
	copilot: app(
		"app-customer-support-copilot",
		"Customer Support Copilot",
		IAppCategory.Business,
		IAppType.Agent,
		[4.8, 126],
		0,
		"Triages tickets, drafts replies and keeps a live graph of customers, accounts and issues.",
		120,
	),
	invoice: app(
		"app-invoice-autopilot",
		"Invoice Autopilot",
		IAppCategory.Finance,
		IAppType.DataPipeline,
		[4.6, 58],
		1200,
		"Reads invoices from mail and PDFs, extracts line items, books them into your ledger.",
		90,
	),
	research: app(
		"app-research-assistant",
		"Research Assistant",
		IAppCategory.Productivity,
		IAppType.Agent,
		[4.7, 212],
		0,
		"Chat with your documents and the web; cites every claim.",
		200,
	),
	sales: app(
		"app-sales-pulse",
		"Sales Pulse",
		IAppCategory.Business,
		IAppType.Analytics,
		[4.5, 41],
		800,
		"Pipeline, revenue and churn in one live dashboard.",
		60,
	),
	modelLab: app(
		"app-model-lab",
		"Model Lab",
		IAppCategory.Education,
		IAppType.DataFocus,
		[4.3, 19],
		0,
		"Train and compare small models on your own tables.",
		4,
	),
	pageStudio: app(
		"app-page-studio",
		"Page Studio",
		IAppCategory.Productivity,
		IAppType.CustomInterface,
		[4.4, 33],
		0,
		"Build internal tools with drag-and-drop pages.",
		5,
	),
	selfHost: app(
		"app-self-host-kit",
		"Self-Host Kit",
		IAppCategory.Utilities,
		IAppType.Form,
		null,
		0,
		"Deploy Flow-Like on your own infrastructure in minutes.",
		1,
	),
	bounded: app(
		"app-bounded-agent-starter",
		"Bounded Agent Starter",
		IAppCategory.Education,
		IAppType.Agent,
		[4.9, 8],
		0,
		"A safe agent loop template with budgets and guardrails.",
		2,
	),
} satisfies Record<string, AppItem>;

export const FIXTURE_PACKAGES = {
	codeInterpreter: pkg("code-interpreter", "Code Interpreter", {
		verified: true,
		version: "0.1.2",
		installs: 5400,
		capabilities: ["net.http", "models", "storage.user", "variables", "cache"],
		category: "DATA_TRANSFORMATION",
		description:
			"Sandboxed Python execution via WASM Component Model. Two nodes: inline eval and project-based runs.",
	}),
	typst: pkg("typst-documents", "Typst Documents", {
		verified: true,
		version: "0.2.0",
		installs: 3400,
		capabilities: [],
		category: "DOCUMENT_PROCESSING",
		description:
			"Compile Typst source text into PDF or SVG documents, with FlowLike-backed assets and Typst Universe packages.",
	}),
	pdfTools: pkg("pdf-tools", "PDF Tools", {
		verified: true,
		version: "0.1.0",
		installs: 2000,
		capabilities: [],
		category: "DOCUMENT_PROCESSING",
		description: "PDF tools for Flow-Like workflows",
	}),
	youtube: pkg("youtube-tools", "YouTube Tools", {
		verified: true,
		version: "0.3.0",
		installs: 550,
		capabilities: ["net.http"],
		category: "MEDIA_CONTENT",
		description:
			"Comprehensive YouTube tools for Flow-Like: transcripts, metadata, search, channels, playlists, and more",
	}),
	privacyBlur: pkg("privacy-blur", "Privacy Blur", {
		verified: false,
		version: "0.4.0",
		installs: 1100,
		rating: [4.6, 14],
		price: 499,
		capabilities: ["models"],
		category: "AI_ML",
		description: "Detect faces. Blur sensitive regions. Write the result.",
	}),
	invoiceExtraction: pkg("invoice-extraction", "Invoice Extraction", {
		verified: true,
		version: "1.2.0",
		installs: 890,
		rating: [4.4, 22],
		price: 900,
		capabilities: ["models", "storage.user"],
		category: "DOCUMENT_PROCESSING",
		description: "Pull totals, dates and line items out of invoice PDFs.",
	}),
	feedReader: pkg("feed-reader", "Feed Reader", {
		verified: false,
		version: "0.2.1",
		installs: 310,
		rating: [4.1, 9],
		capabilities: ["net.http"],
		category: "INTEGRATION_CONNECTORS",
		description: "Poll RSS/Atom feeds and emit new items as events.",
	}),
	voiceActivity: pkg("voice-activity-detection", "Voice Activity Detection", {
		verified: false,
		version: "0.1.0",
		installs: 120,
		capabilities: [],
		category: "AI_ML",
		description: "Find speech segments in audio streams.",
	}),
} satisfies Record<string, PackageItem>;

const A = FIXTURE_APPS;
const P = FIXTURE_PACKAGES;

const INVOICES_ID = "collection-invoices";
const INVOICES_TITLE = "Automate your invoices";
const INVOICES_BLURB = "One app to run it, three packages to build your own.";
const INVOICE_ITEMS: ExploreResolvedItem[] = [
	A.invoice,
	P.invoiceExtraction,
	P.pdfTools,
	P.typst,
];

type Projection = "all" | "apps" | "packages";

function visible(
	items: readonly ExploreResolvedItem[],
	projection: Projection,
): ExploreResolvedItem[] {
	if (projection === "apps") {
		return items.filter((item) => item.kind !== "package");
	}
	if (projection === "packages") {
		return items.filter((item) => item.kind !== "app");
	}
	return [...items];
}

function kindCounts(items: readonly ExploreResolvedItem[]) {
	return {
		apps: items.filter((item) => item.kind === "app").length,
		packages: items.filter((item) => item.kind === "package").length,
	};
}

function invoicesSummary(projection: Projection): ExploreCollectionSummary {
	const preview = visible(INVOICE_ITEMS, projection);
	return {
		id: INVOICES_ID,
		title: INVOICES_TITLE,
		blurb: INVOICES_BLURB,
		...kindCounts(preview),
		preview,
	};
}

function collection(
	placementId: string,
	title: string,
	blurb: string,
	items: ExploreResolvedItem[],
): ExploreCollection {
	return {
		placementId,
		title,
		blurb,
		source: "hand",
		items,
		...kindCounts(items),
	};
}

function rail(
	placementId: string,
	key: ExploreRailKey,
	items: ExploreResolvedItem[],
): ExploreRail {
	return { placementId, rail: key, title: null, items };
}

const ANNOUNCEMENT: ExploreAnnouncement = {
	placementId: "announcement-widgets",
	dismissKey: "announcement-widgets:3f2a9c1b7e04",
	tone: "launch",
	title: "Packages can now ship widgets",
	body: "Drop live UI from a node package straight into your app pages.",
	ctaLabel: "See what's new",
	ctaHref: "/store/explore/search?type=packages",
	imageUrl: null,
	dismissible: true,
};

const CATEGORY_COUNTS: [IAppCategory, number, number][] = [
	[IAppCategory.Business, 14, 6],
	[IAppCategory.Finance, 7, 5],
	[IAppCategory.Productivity, 21, 9],
	[IAppCategory.Education, 9, 3],
];

function projectedView(projection: Projection): ExploreView {
	const withApps = projection !== "packages";
	const withPackages = projection !== "apps";
	const invoices = collection(
		INVOICES_ID,
		INVOICES_TITLE,
		INVOICES_BLURB,
		visible(INVOICE_ITEMS, projection),
	);
	const agents = collection(
		"collection-agents",
		"Agents that stay in bounds",
		"Agents with budgets, guardrails and a clear paper trail.",
		[A.bounded, A.research, A.copilot],
	);
	const slides = [
		{
			item: A.copilot,
			headline: "Support that remembers every customer",
			subline:
				"Triages tickets, drafts replies and keeps a live graph of customers, accounts and issues.",
			accent: "auto" as const,
		},
		{
			item: {
				kind: "collection" as const,
				collection: invoicesSummary(projection),
			},
			headline: INVOICES_TITLE,
			subline: INVOICES_BLURB,
			accent: "category:Finance" as const,
		},
		{
			item: P.codeInterpreter,
			headline: "Run real Python inside your flows",
			subline:
				"Sandboxed Python execution via WASM Component Model — inline eval or whole projects.",
		},
	].filter((slide) =>
		slide.item.kind === "collection"
			? slide.item.collection.preview.length >= 2
			: visible([slide.item], projection).length > 0,
	);
	const newApps = withApps ? 9 : 0;
	const newPackages = withPackages ? 15 : 0;
	const trending = visible(
		[
			A.copilot,
			A.research,
			A.invoice,
			A.sales,
			P.codeInterpreter,
			P.youtube,
			P.typst,
			P.pdfTools,
		],
		projection,
	);
	const topPaid = visible(
		[A.invoice, A.sales, P.invoiceExtraction, P.privacyBlur],
		projection,
	);
	const builders = withPackages
		? rail("rail-for-builders", "for_builders", [
				P.pdfTools,
				P.privacyBlur,
				P.feedReader,
				P.voiceActivity,
			])
		: rail("rail-new", "new", [
				A.selfHost,
				A.bounded,
				A.modelLab,
				A.pageStudio,
			]);
	return {
		grid: {
			hero: { placementId: "spotlight-main", rotationSeconds: 8, slides },
			notice: ANNOUNCEMENT,
			feature: withPackages
				? {
						placementId: "feature-typst",
						eyebrow: null,
						slide: { item: P.typst },
					}
				: {
						placementId: "feature-invoice-autopilot",
						eyebrow: null,
						slide: { item: A.invoice },
					},
			collection: withPackages ? invoices : agents,
			stat: {
				...rail("rail-new-count", "new_count", []),
				stat: {
					total: newApps + newPackages,
					apps: newApps,
					packages: newPackages,
					freeApps: withApps ? 7 : 0,
					paidApps: withApps ? 2 : 0,
					verifiedPackages: withPackages ? 6 : 0,
				},
			},
			categories: {
				...rail("rail-by-category", "by_category", []),
				categories: CATEGORY_COUNTS.map(([appCategory, apps, packages]) => ({
					appCategory,
					apps: withApps ? apps : 0,
					packages: withPackages ? packages : 0,
				})),
			},
		},
		rows: [
			{ kind: "rail", rail: rail("rail-trending", "trending", trending) },
			...(projection === "packages"
				? []
				: [
						{
							kind: "rail" as const,
							rail: rail("rail-suites", "suites", []),
						},
					]),
			{ kind: "rail", rail: rail("rail-top-paid", "top_paid", topPaid) },
			{ kind: "rail", rail: builders },
		],
	};
}

function viewer(dev: boolean): ExploreViewer {
	return { dev, signedIn: true, platform: "desktop", language: "en" };
}

/** Sample landing page from the design brief: dev → all three views, otherwise the apps-only `all` view. */
export function exploreFixture({
	dev = true,
}: { dev?: boolean } = {}): ResolvedExplore {
	return {
		revision: "fixture",
		generatedAt: new Date(FIXTURE_EPOCH_SECS * 1000).toISOString(),
		viewer: viewer(dev),
		typeCounts: { apps: 184, packages: dev ? 128 : 0 },
		views: dev
			? {
					all: projectedView("all"),
					apps: projectedView("apps"),
					packages: projectedView("packages"),
				}
			: { all: projectedView("apps"), apps: null, packages: null },
	};
}

/** Sample Browse response for the query "invoice". */
export function exploreSearchFixture({
	dev = true,
}: { dev?: boolean } = {}): ExploreSearchResponse {
	const projection: Projection = dev ? "all" : "apps";
	const packageHits = dev
		? [
				{ package: P.invoiceExtraction.package, matchedVia: "name" as const },
				{
					package: P.pdfTools.package,
					matchedVia: "collection" as const,
					collectionTitle: INVOICES_TITLE,
				},
				{
					package: P.typst.package,
					matchedVia: "collection" as const,
					collectionTitle: INVOICES_TITLE,
				},
			]
		: [];
	return {
		query: "invoice",
		viewer: viewer(dev),
		collections: [
			collection(
				INVOICES_ID,
				INVOICES_TITLE,
				INVOICES_BLURB,
				visible(INVOICE_ITEMS, projection),
			),
		],
		apps: { items: [A.invoice], total: 1, hasMore: false },
		packages: {
			items: packageHits,
			total: packageHits.length,
			hasMore: false,
		},
		related: dev ? [] : [A.sales, A.research, A.pageStudio],
		facets: {
			types: { apps: 1, packages: packageHits.length, collections: 1 },
			categories: dev
				? [
						{ value: "app:Finance", kind: "app", count: 1 },
						{ value: "package:DOCUMENT_PROCESSING", kind: "package", count: 3 },
					]
				: [{ value: "app:Finance", kind: "app", count: 1 }],
			price: { free: dev ? 2 : 0, paid: dev ? 2 : 1 },
			verified: dev ? 3 : 0,
			permissions: dev
				? { none: 2, network: 0, models: 1, storage: 1 }
				: { none: 0, network: 0, models: 0, storage: 0 },
			packagesCapped: false,
		},
	};
}
