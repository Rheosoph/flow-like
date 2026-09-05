import {
	type IApp,
	IAppCategory,
	IAppExecutionMode,
	IAppStatus,
	IAppType,
	IAppVisibility,
} from "../../packages/ui/lib/schema/app/app";
import type { IMetadata } from "../../packages/ui/lib/schema/bit/bit-pack";
import {
	PackageStatus,
	type PackageSummary,
	type WasmPackageCategory,
} from "../../packages/ui/lib/schema/wasm";
import type { IGroup } from "../../packages/ui/state/backend-state/types";

export type AppEntry = [IApp, IMetadata];
const at = { secs_since_epoch: 1788566400, nanos_since_epoch: 0 };
const categories = [
	IAppCategory.Productivity,
	IAppCategory.Business,
	IAppCategory.Utilities,
	IAppCategory.Education,
	IAppCategory.Communication,
	IAppCategory.Finance,
	IAppCategory.Travel,
	IAppCategory.Photography,
];
const colors = [
	"#b25a34",
	"#5568a9",
	"#39816e",
	"#97724e",
	"#8d6098",
	"#527c9b",
	"#4b8677",
	"#b76669",
];
const catalog = [
	[
		"Knowledge Chat",
		"Find answers in your team’s documents with links to the source.",
	],
	[
		"Invoice Desk",
		"Extract invoice details and route each payment for review.",
	],
	[
		"Webhook Relay",
		"Connect incoming events to the tools your team already uses.",
	],
	[
		"Study Companion",
		"Turn your course notes into practice questions and review sessions.",
	],
	[
		"Meeting Notes",
		"Keep decisions, context, and action items together after every call.",
	],
	["Expense Review", "Check receipts and prepare your monthly expense report."],
	[
		"Trip Planner",
		"Bring bookings, travel times, and daily plans into one itinerary.",
	],
	["Photo Organizer", "Sort your photo library by date, project, and subject."],
	[
		"Focus Board",
		"Plan the work that matters today and track it to completion.",
	],
	[
		"Customer Brief",
		"Prepare for customer conversations with account context in one view.",
	],
	[
		"CSV Cleaner",
		"Find duplicate rows and normalize columns before importing your data.",
	],
	[
		"Language Practice",
		"Build a daily vocabulary routine with examples you can remember.",
	],
	[
		"Team Digest",
		"Collect useful project updates into a weekly team briefing.",
	],
	[
		"Budget Planner",
		"Compare planned spending with actual costs throughout the month.",
	],
	[
		"City Guide",
		"Save local recommendations and organize stops by neighborhood.",
	],
	[
		"Image Resizer",
		"Prepare consistent image sizes for your website and social posts.",
	],
	["Reading Queue", "Save articles with notes and pick up where you left off."],
	[
		"Proposal Builder",
		"Draft a project proposal from your scope, timeline, and pricing.",
	],
	[
		"File Sorter",
		"Organize incoming files using rules your whole team can follow.",
	],
	[
		"Lesson Planner",
		"Prepare lessons with learning objectives, materials, and exercises.",
	],
	["Inbox Triage", "Group new messages by topic and identify the next action."],
	["Cash Flow", "Track expected payments and see upcoming cash requirements."],
	[
		"Packing List",
		"Build a packing checklist around your destination and trip length.",
	],
	[
		"Contact Sheet",
		"Create a visual overview of images for review and selection.",
	],
	[
		"Research Notebook",
		"Collect source material and connect your notes across projects.",
	],
	[
		"Lead Organizer",
		"Keep new leads, follow-up dates, and account notes together.",
	],
	["JSON Inspector", "Explore structured data and find the fields you need."],
	[
		"Flashcard Studio",
		"Build focused study sets from your notes and learning materials.",
	],
	[
		"Announcement Drafts",
		"Prepare clear project announcements for different teams.",
	],
	[
		"Subscription Tracker",
		"See recurring costs and plan ahead for renewal dates.",
	],
	[
		"Route Notebook",
		"Keep route details and practical notes for your next journey.",
	],
	[
		"Asset Library",
		"Find approved images and keep usage notes alongside each asset.",
	],
	["Task Capture", "Turn quick notes into tasks you can schedule and finish."],
	[
		"Support Insights",
		"Review recurring support topics and identify documentation gaps.",
	],
	[
		"PDF Toolkit",
		"Collect, split, and arrange pages for everyday document work.",
	],
	[
		"Course Tracker",
		"Track progress across courses and plan your next learning session.",
	],
	[
		"Translation Desk",
		"Review translated messages with the original text in view.",
	],
	[
		"Revenue Report",
		"Review revenue by product, period, and customer segment.",
	],
	[
		"Travel Journal",
		"Keep places, photos, and observations from your travels together.",
	],
	[
		"Color Library",
		"Collect palettes from project images and share the selected colors.",
	],
	[
		"Habit Notes",
		"Track daily routines and reflect on what helped you follow through.",
	],
	[
		"Project Intake",
		"Collect the information your team needs before starting new work.",
	],
	[
		"Link Checker",
		"Check saved URLs and identify broken links before publication.",
	],
	[
		"Practice Log",
		"Record focused practice sessions and see your progress over time.",
	],
	[
		"Feedback Inbox",
		"Collect feedback, group related requests, and share next steps.",
	],
	[
		"Purchase Review",
		"Compare purchase requests with budgets and approval rules.",
	],
	[
		"Weekend Finder",
		"Plan short trips around the time and activities you have in mind.",
	],
	[
		"Shot Planner",
		"Prepare a shoot with reference images, locations, and a shot list.",
	],
	[
		"Personal Dashboard",
		"Bring your priorities and useful daily information into one view.",
	],
	[
		"Process Library",
		"Document repeatable work so teammates can find the next step.",
	],
	[
		"Data Converter",
		"Convert tabular data into the format your next tool expects.",
	],
	[
		"Workshop Kit",
		"Prepare exercises, timing, and materials for a working session.",
	],
	[
		"Community Updates",
		"Prepare a digest of useful conversations and upcoming events.",
	],
	[
		"Donation Ledger",
		"Track contributions and organize records for reporting.",
	],
	[
		"Booking Binder",
		"Keep confirmations and practical details ready when you travel.",
	],
	[
		"Gallery Review",
		"Share a photo selection and collect feedback on each image.",
	],
	[
		"Weekly Review",
		"Review finished work and choose priorities for the week ahead.",
	],
	[
		"Vendor Directory",
		"Keep supplier contacts, agreements, and renewal notes together.",
	],
	[
		"Archive Search",
		"Search across archived files and recover useful project material.",
	],
	[
		"Reading Tutor",
		"Break a reading assignment into questions and short exercises.",
	],
	[
		"Interview Notes",
		"Capture interview themes and connect observations to evidence.",
	],
	[
		"Savings Goals",
		"Track progress toward planned purchases and financial milestones.",
	],
	["Local Explorer", "Create a personal map of places to visit close to home."],
	[
		"Export Studio",
		"Prepare image exports with consistent names and size presets.",
	],
];

function icon(name: string, index: number) {
	const letters = name
		.split(" ")
		.slice(0, 2)
		.map((word) => word[0])
		.join("");
	return `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><rect width="128" height="128" rx="28" fill="${colors[index % colors.length]}"/><circle cx="102" cy="22" r="44" fill="white" opacity=".08"/><text x="64" y="79" text-anchor="middle" font-family="Arial,sans-serif" font-size="43" font-weight="700" fill="white">${letters}</text></svg>`)}`;
}

export const apps: AppEntry[] = catalog.map(([name, description], index) => {
	const ratingCount = index % 7 === 0 ? 0 : 12 + index * 3;
	const average = 4.1 + (index % 9) / 10;
	return [
		{
			id: `explore-app-${index + 1}`,
			authors: ["Community Studio"],
			bits: [],
			boards: [],
			events: [],
			page_ids: [],
			widget_ids: [],
			templates: [],
			status: IAppStatus.Active,
			visibility: IAppVisibility.Public,
			execution_mode: IAppExecutionMode.Any,
			app_type: Object.values(IAppType)[index % Object.values(IAppType).length],
			primary_category: categories[index % categories.length],
			created_at: {
				...at,
				secs_since_epoch: at.secs_since_epoch - index * 86400,
			},
			updated_at: {
				...at,
				secs_since_epoch: at.secs_since_epoch - ((index * 7) % 64) * 3600,
			},
			download_count: 12000 - index * 173,
			interactions_count: 1000 - index * 10,
			rating_count: ratingCount,
			rating_sum: Math.round(ratingCount * average),
			avg_rating: ratingCount ? average : null,
			price: index % 11 === 3 ? 490 : 0,
			allow_forking: true,
			version: "1.2.0",
		},
		{
			name,
			description,
			use_case: description,
			icon: icon(name, index),
			tags: [
				categories[index % categories.length].toLowerCase(),
				index % 2 ? "workflow" : "assistant",
			],
			created_at: at,
			updated_at: at,
			preview_media: [],
		},
	];
});

export const groups: IGroup[] = [
	{
		name: "Team Operations",
		use_case: "Keep everyday work moving",
		description:
			"Connected tools for meeting notes, project intake, and team updates.",
		indices: [4, 41, 12],
	},
	{
		name: "Research Workspace",
		use_case: "From sources to useful answers",
		description:
			"Collect source material, ask questions, and prepare your next briefing.",
		indices: [0, 24, 16, 60],
	},
].map((group, index) => ({
	id: `explore-suite-${index + 1}`,
	owner_app_id: apps[group.indices[0]][0].id,
	status: "ACTIVE",
	visibility: "PUBLIC",
	name: group.name,
	use_case: group.use_case,
	description: group.description,
	icon: icon(group.name, index + 2),
	tags: ["teamwork"],
	member_count: group.indices.length,
	members: group.indices.map((appIndex, position) => ({
		id: `suite-${index}-member-${position}`,
		app_id: apps[appIndex][0].id,
		kind: position === 0 ? "PRIMARY" : "MEMBER",
		status: "ACTIVE",
		position,
		app_name: apps[appIndex][1].name,
		app_description: apps[appIndex][1].description,
		app_icon: apps[appIndex][1].icon,
	})),
	created_at: at.secs_since_epoch,
	updated_at: at.secs_since_epoch,
}));

const packageCatalog: [string, string, WasmPackageCategory][] = [
	[
		"Document Toolkit",
		"Extract text, split pages, and assemble documents in your workflows.",
		"DOCUMENT_PROCESSING",
	],
	[
		"HTTP Connectors",
		"Call external APIs with reusable request and response nodes.",
		"INTEGRATION_CONNECTORS",
	],
	[
		"Table Transform",
		"Filter, join, and reshape tables before the next workflow step.",
		"DATA_TRANSFORMATION",
	],
	[
		"Language Tools",
		"Classify text and prepare structured prompts for language models.",
		"AI_ML",
	],
	[
		"Schedule Utilities",
		"Build recurring schedules and calculate the next execution time.",
		"WORKFLOW_AUTOMATION",
	],
	[
		"Chart Helpers",
		"Prepare series, labels, and grouped data for reporting views.",
		"ANALYTICS_REPORTING",
	],
	[
		"CSV Nodes",
		"Read and write CSV files with consistent column handling.",
		"DATA_TRANSFORMATION",
	],
	[
		"Email Connector",
		"Compose messages and process incoming email events.",
		"COMMUNICATION",
	],
	[
		"Image Metadata",
		"Read image dimensions and extract embedded metadata.",
		"MEDIA_CONTENT",
	],
	[
		"JSON Toolkit",
		"Query JSON values, validate structures, and merge objects.",
		"DATA_TRANSFORMATION",
	],
	[
		"PDF Extraction",
		"Extract document text and page information for later processing.",
		"DOCUMENT_PROCESSING",
	],
	[
		"Webhook Helpers",
		"Validate incoming payloads and prepare responses for integrations.",
		"INTEGRATION_CONNECTORS",
	],
	[
		"Date and Time",
		"Parse dates, convert time zones, and calculate elapsed time.",
		"WORKFLOW_AUTOMATION",
	],
	[
		"Text Embeddings",
		"Create embedding vectors for retrieval and similarity workflows.",
		"AI_ML",
	],
	[
		"Spreadsheet Bridge",
		"Read worksheet values and prepare updates from workflow data.",
		"INTEGRATION_CONNECTORS",
	],
	[
		"Markdown Tools",
		"Parse headings and convert structured notes into Markdown.",
		"DOCUMENT_PROCESSING",
	],
	[
		"Metrics Aggregator",
		"Calculate grouped metrics and compare reporting periods.",
		"ANALYTICS_REPORTING",
	],
	[
		"Archive Utilities",
		"Inspect archive contents and organize extracted files.",
		"DATA_TRANSFORMATION",
	],
	[
		"Receipt Parser",
		"Extract receipt fields for a review and reconciliation workflow.",
		"FINANCE_BILLING",
	],
	[
		"Message Templates",
		"Render reusable message templates with workflow variables.",
		"COMMUNICATION",
	],
	[
		"Storage Connector",
		"List objects and transfer files between storage locations.",
		"INTEGRATION_CONNECTORS",
	],
	[
		"Prompt Library",
		"Build reusable prompt templates with typed input values.",
		"AI_ML",
	],
	[
		"Workflow Assertions",
		"Check intermediate values and return useful validation failures.",
		"WORKFLOW_AUTOMATION",
	],
];

export const packages: PackageSummary[] = packageCatalog.map(
	([name, description, category], index) => ({
		id: `explore-package-${index + 1}`,
		name: name.toLowerCase().replaceAll(" ", "-"),
		description,
		latestVersion: `${1 + (index % 3)}.${index % 6}.0`,
		downloadCount: 18600 - index * 631,
		status: PackageStatus.Active,
		keywords: [
			category.toLowerCase().replaceAll("_", " "),
			index % 2 ? "connector" : "utilities",
		],
		verified: index % 3 !== 1,
		price: index % 7 === 4 ? 990 : 0,
		visibility: "public",
		primaryCategory: category,
		avgRating: index % 5 === 0 ? null : 4.1 + (index % 9) / 10,
		ratingCount: index % 5 === 0 ? 0 : 14 + index * 2,
		capabilities:
			category === "INTEGRATION_CONNECTORS"
				? ["net.http", "oauth"]
				: category === "AI_ML"
					? ["models", "streaming"]
					: ["storage.node", "cache"],
		metadata: { lang: "en", name, description, icon: icon(name, index) },
	}),
);
