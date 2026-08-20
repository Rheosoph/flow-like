"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Activity,
	ArrowLeft,
	Ban,
	Boxes,
	ChevronLeft,
	ChevronRight,
	CircleHelp,
	Download,
	FileText,
	Gauge,
	Mail,
	Pencil,
	Plus,
	RefreshCw,
	ScrollText,
	Search,
	ShieldAlert,
	ShieldCheck,
	ShieldQuestion,
	ShieldX,
	Sparkles,
	UserCheck,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	ConformityRecommendations,
	type Recommendation,
} from "../../ai-act/conformity-recommendations";
import {
	Accordion,
	AccordionContent,
	AccordionItem,
	AccordionTrigger,
	Alert,
	AlertDescription,
	AlertTitle,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Progress,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
	Switch,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui";

// ---------------------------------------------------------------------------
// Types (camelCase JSON from the admin AI Act API).
// ---------------------------------------------------------------------------

type RiskCategory =
	| "PROHIBITED"
	| "HIGH"
	| "LIMITED"
	| "MINIMAL"
	| "UNDETERMINED";

type ConformityBand = "green" | "amber" | "red";

interface InventoryItem {
	appId: string;
	appName?: string | null;
	riskCategory: RiskCategory;
	status: string;
	conformityScore?: number | null;
	conformityBand?: string | null;
	modelCount: number;
	unvettedModelCount: number;
	driftCount: number;
	worstScore?: number | null;
	securityScore?: number | null;
	privacyScore?: number | null;
	governanceScore?: number | null;
	boardCount: number;
	updatedAt: string;
}

interface InventoryResponse {
	items: InventoryItem[];
	total: number;
	page: number;
	limit: number;
	hasMore: boolean;
}

interface ExportRow {
	appId: string;
	appName?: string | null;
	riskCategory: RiskCategory;
	status: string;
	conformityScore?: number | null;
	conformityBand?: string | null;
	updatedAt: string;
}

interface ModelObservationItem {
	id: string;
	modelId: string;
	provider?: string | null;
	source: string;
	posture: string;
	hosted: boolean;
	openLicence: boolean;
	systemicRisk: boolean;
	vetted: boolean;
	dynamicSelector: boolean;
	driftFlagged: boolean;
	firstSeenAt: string;
	lastSeenAt: string;
}

interface ResponsiblePerson {
	userId?: string | null;
	name?: string | null;
	email?: string | null;
	username?: string | null;
	avatar?: string | null;
	description?: string | null;
}

interface AssessmentSummary {
	riskCategory: string;
	status: string;
	version?: number | null;
	conformityScore?: number | null;
	conformityBand?: string | null;
	responsibleName?: string | null;
	responsibleEmail?: string | null;
	responsiblePerson?: ResponsiblePerson | null;
	reviewedByName?: string | null;
	transparencyObligations?: unknown;
	submittedAt?: string | null;
	reviewedAt?: string | null;
	reviewNote?: string | null;
	updatedAt: string;
}

interface QuestionOption {
	value: string;
	label: string;
	help?: string | null;
}

interface Question {
	key: string;
	label: string;
	kind: "select" | "multi" | "yesno" | "text" | "contact";
	options?: QuestionOption[];
	help?: string | null;
	required?: boolean;
}

interface Screen {
	id: string;
	title: string;
	description?: string;
	questions: Question[];
	highRiskOnly?: boolean;
}

interface QuestionnaireSchema {
	version: number;
	screens: Screen[];
}

interface Classification {
	riskCategory: RiskCategory;
	conformityScore: number | null;
	conformityBand: ConformityBand | null;
	transparencyObligations: string[];
	blocked: boolean;
	rationale: string[];
}

interface InventoryDetailResponse {
	appId: string;
	appName?: string | null;
	assessment?: AssessmentSummary | null;
	models: ModelObservationItem[];
	schema: QuestionnaireSchema;
	signals: Record<string, unknown>;
	answers: Record<string, unknown>;
	classification: Classification;
	recommendations: Recommendation[];
	hasAssessment: boolean;
}

interface RegistryItem {
	id: string;
	provider: string;
	modelId: string;
	posture: string;
	hosted: boolean;
	openLicence: boolean;
	systemicRisk: boolean;
	vetted: boolean;
	note?: string | null;
	updatedAt: string;
	observed: boolean;
	registered: boolean;
	needsRating: boolean;
	observedCount: number;
}

interface FlaggedPattern {
	node: string;
	category: string;
	score: number;
	count?: number;
}

interface BoardScoreItem {
	boardId: string;
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
	worstScore: number;
	nodeCount: number;
	scoredNodeCount: number;
	flaggedPatterns: FlaggedPattern[];
	computedAt: string;
	updatedAt: string;
}

interface AppScoreDetailResponse {
	appId: string;
	appName?: string | null;
	boards: BoardScoreItem[];
}

const SCORE_CATEGORIES = [
	"security",
	"privacy",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

type ScoreCategory = (typeof SCORE_CATEGORIES)[number];

const PAGE_SIZE = 25;

// ---------------------------------------------------------------------------
// EU AI Act metadata (Regulation (EU) 2024/1689).
// ---------------------------------------------------------------------------

const RISK_META: Record<
	RiskCategory,
	{
		label: string;
		article: string;
		description: string;
		badge: string;
		icon: LucideIcon;
	}
> = {
	PROHIBITED: {
		label: "Prohibited",
		article: "Art. 5",
		description:
			"Banned AI practice. The system may not be placed on the EU market and publication is blocked.",
		badge: "bg-red-600 text-white",
		icon: ShieldX,
	},
	HIGH: {
		label: "High Risk",
		article: "Annex III",
		description:
			"Subject to the full conformity regime: risk management, data governance, logging, technical documentation and human oversight.",
		badge: "bg-orange-500 text-white",
		icon: ShieldAlert,
	},
	LIMITED: {
		label: "Limited Risk",
		article: "Art. 50",
		description:
			"Transparency obligations apply — users must be told they are interacting with, or consuming output from, an AI system.",
		badge: "bg-yellow-500 text-black",
		icon: ShieldQuestion,
	},
	MINIMAL: {
		label: "Minimal Risk",
		article: "—",
		description:
			"No mandatory obligations beyond voluntary codes of conduct. Most general-purpose applications fall here.",
		badge: "bg-emerald-500 text-white",
		icon: ShieldCheck,
	},
	UNDETERMINED: {
		label: "Undetermined",
		article: "—",
		description:
			"Not enough information to classify. Complete the conformity questionnaire to obtain a determination.",
		badge: "bg-slate-500 text-white",
		icon: CircleHelp,
	},
};

const TRANSPARENCY_META: Record<
	string,
	{ article: string; label: string; description: string }
> = {
	disclose_ai_interaction: {
		article: "Art. 50(1)",
		label: "Disclose AI interaction",
		description: "Inform people they are interacting with an AI system.",
	},
	label_generated_content: {
		article: "Art. 50(2)",
		label: "Label AI-generated content",
		description:
			"Mark synthetic audio, image, video or text as artificially generated.",
	},
	disclose_emotion_biometric: {
		article: "Art. 50(3)",
		label: "Disclose emotion / biometric use",
		description:
			"Notify people exposed to emotion-recognition or biometric categorisation.",
	},
	human_oversight: {
		article: "Art. 14",
		label: "Human oversight",
		description: "Ensure effective human oversight of the high-risk system.",
	},
	technical_documentation: {
		article: "Art. 11 / 12",
		label: "Technical documentation & logging",
		description:
			"Maintain technical documentation and automatic event logs of operation.",
	},
};

const POSTURE_LABEL: Record<string, string> = {
	UNKNOWN: "Unknown",
	HOSTED: "Hosted GPAI",
	OPEN_LICENCE: "Open licence",
	CLOSED: "Closed",
	SYSTEMIC: "Systemic risk",
};

const BAND_META: Record<
	ConformityBand,
	{ label: string; text: string; bar: string }
> = {
	green: {
		label: "Conformant",
		text: "text-emerald-600",
		bar: "bg-emerald-500",
	},
	amber: {
		label: "Needs attention",
		text: "text-amber-600",
		bar: "bg-amber-500",
	},
	red: { label: "At risk", text: "text-red-600", bar: "bg-red-500" },
};

const RISK_FILTER_OPTIONS: { value: string; label: string }[] = [
	{ value: "all", label: "All risk levels" },
	{ value: "PROHIBITED", label: "Prohibited" },
	{ value: "HIGH", label: "High" },
	{ value: "LIMITED", label: "Limited" },
	{ value: "MINIMAL", label: "Minimal" },
	{ value: "UNDETERMINED", label: "Undetermined" },
];

const STATUS_FILTER_OPTIONS: { value: string; label: string }[] = [
	{ value: "all", label: "All statuses" },
	{ value: "UNASSESSED", label: "Unassessed" },
	{ value: "DRAFT", label: "Draft" },
	{ value: "SUBMITTED", label: "Submitted" },
	{ value: "APPROVED", label: "Approved" },
	{ value: "REJECTED", label: "Rejected" },
	{ value: "BLOCKED", label: "Blocked" },
];

// ---------------------------------------------------------------------------
// Small presentational helpers.
// ---------------------------------------------------------------------------

function riskMeta(risk: RiskCategory) {
	return RISK_META[risk] ?? RISK_META.UNDETERMINED;
}

function RiskBadge({
	risk,
	withTooltip = false,
}: {
	risk: RiskCategory;
	withTooltip?: boolean;
}) {
	const meta = riskMeta(risk);
	const Icon = meta.icon;
	const badge = (
		<Badge className={`${meta.badge} gap-1`}>
			<Icon className="h-3 w-3" />
			{meta.label}
		</Badge>
	);
	if (!withTooltip) return badge;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span>{badge}</span>
			</TooltipTrigger>
			<TooltipContent className="max-w-xs">
				<p className="text-xs font-medium">
					{meta.article !== "—" ? `${meta.article} — ` : ""}
					{meta.label}
				</p>
				<p className="text-xs text-muted-foreground">{meta.description}</p>
			</TooltipContent>
		</Tooltip>
	);
}

function StatusBadge({ status }: { status: string }) {
	const { t } = useTranslation("admin");
	const normalized = status.toUpperCase();
	const tone =
		normalized === "APPROVED"
			? `bg-emerald-500/15 text-emerald-600 border-emerald-500/30`
			: normalized === "SUBMITTED"
				? `bg-blue-500/15 text-blue-600 border-blue-500/30`
				: normalized === "REJECTED" || normalized === "BLOCKED"
					? `bg-red-500/15 text-red-600 border-red-500/30`
					: `bg-muted text-muted-foreground border-border`;
	return (
		<Badge variant="outline" className={tone}>
			{normalized}
		</Badge>
	);
}

function bandTextColor(band?: string | null): string {
	if (band === "green" || band === "amber" || band === "red") {
		return BAND_META[band].text;
	}
	return "text-muted-foreground";
}

function formatAnswer(question: Question | undefined, value: unknown): string {
	if (value === undefined || value === null || value === "") return "—";

	const labelFor = (raw: string) =>
		question?.options?.find((o) => o.value === raw)?.label ?? raw;

	if (Array.isArray(value)) {
		if (value.length === 0) return "—";
		return value.map((v) => labelFor(String(v))).join(", ");
	}

	if (question?.kind === "yesno") {
		const raw = String(value).toLowerCase();
		if (raw === "yes" || raw === "true") return "Yes";
		if (raw === "no" || raw === "false") return "No";
	}

	return labelFor(String(value));
}

function triggerDownload(filename: string, content: string, type: string) {
	const blob = new Blob([content], { type });
	const url = URL.createObjectURL(blob);
	const a = document.createElement("a");
	a.href = url;
	a.download = filename;
	document.body.appendChild(a);
	a.click();
	document.body.removeChild(a);
	URL.revokeObjectURL(url);
}

function scoreBg(value: number): string {
	if (value >= 7) return "bg-emerald-500";
	if (value >= 4) return "bg-yellow-500";
	return "bg-red-500";
}

function scoreText(value: number): string {
	if (value >= 7) return "text-emerald-600 dark:text-emerald-400";
	if (value >= 4) return "text-yellow-600 dark:text-yellow-400";
	return "text-red-600 dark:text-red-400";
}

function ScoreChip({ value }: { value: number }) {
	return (
		<span
			className={`inline-flex h-6 w-6 items-center justify-center rounded-md text-xs font-semibold text-white ${scoreBg(value)}`}
		>
			{value}
		</span>
	);
}

function PageShell({ children }: { children: ReactNode }) {
	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">{children}</div>
			</div>
		</main>
	);
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export function AdminAiActInventoryPage({
	initialAppId,
	initialTab,
	initialRegistryProvider,
	initialRegistryModelId,
	onAppChange,
	onRegistryModelOpen,
}: Readonly<{
	initialAppId?: string | null;
	initialTab?: string | null;
	initialRegistryProvider?: string | null;
	initialRegistryModelId?: string | null;
	onAppChange?: (appId: string | null) => void;
	onRegistryModelOpen?: (provider: string, modelId: string) => void;
}>) {
	const { t } = useTranslation("admin");
	const [selectedAppId, setSelectedAppId] = useState<string | null>(
		initialAppId ?? null,
	);

	useEffect(() => {
		setSelectedAppId(initialAppId ?? null);
	}, [initialAppId]);

	const selectApp = useCallback(
		(appId: string | null) => {
			setSelectedAppId(appId);
			onAppChange?.(appId);
		},
		[onAppChange],
	);

	if (selectedAppId) {
		return (
			<TooltipProvider>
				<PageShell>
					<InventoryDetail
						appId={selectedAppId}
						onBack={() => selectApp(null)}
						onRegistryModelOpen={onRegistryModelOpen}
					/>
				</PageShell>
			</TooltipProvider>
		);
	}

	return (
		<TooltipProvider>
			<PageShell>
				<div className="space-y-1">
					<h2 className="text-xl font-semibold flex items-center gap-2">
						<ShieldAlert className="h-5 w-5 text-primary" />
						{t("aiInventoryAmpGovernance", "AI Inventory & Governance")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t(
							"singleRegisterOfAiConformityAndSecurityPostureUnderTheEuAiActRegulationEu20241689TrackRiskClassificationTransparencyObligationsSecurityAmpQualityScoresAndTheModelsAttachedToEveryPublishedApplication",
							"Single register of AI conformity and security posture under the EU AI Act (Regulation (EU) 2024/1689). Track risk classification, transparency obligations, security & quality scores and the models attached to every published application.",
						)}
					</p>
				</div>

				<Tabs
					defaultValue={initialTab === "registry" ? "registry" : "inventory"}
				>
					<TabsList>
						<TabsTrigger value="inventory">
							{t("conformityInventory", "Conformity Inventory")}
						</TabsTrigger>
						<TabsTrigger value="registry">
							{t("gpaiModelRegistry", "GPAI Model Registry")}
						</TabsTrigger>
					</TabsList>
					<TabsContent value="inventory" className="mt-4">
						<InventoryTab onSelectApp={selectApp} />
					</TabsContent>
					<TabsContent value="registry" className="mt-4">
						<RegistryTab
							initialProvider={initialRegistryProvider}
							initialModelId={initialRegistryModelId}
						/>
					</TabsContent>
				</Tabs>
			</PageShell>
		</TooltipProvider>
	);
}

// ---------------------------------------------------------------------------
// KPI summary
// ---------------------------------------------------------------------------

function StatCard({
	icon: Icon,
	label,
	value,
	hint,
	tone = "text-foreground",
	loading,
}: {
	icon: LucideIcon;
	label: string;
	value: string | number;
	hint?: string;
	tone?: string;
	loading?: boolean;
}) {
	return (
		<Card>
			<CardContent className="p-4">
				<div className="flex items-center justify-between">
					<span className="text-xs font-medium text-muted-foreground">
						{label}
					</span>
					<Icon className="h-4 w-4 text-muted-foreground" />
				</div>
				{loading ? (
					<Skeleton className="mt-2 h-7 w-16" />
				) : (
					<p className={`mt-1 text-2xl font-semibold ${tone}`}>{value}</p>
				)}
				{hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}
			</CardContent>
		</Card>
	);
}

function InventoryStats() {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const stats = useQuery({
		queryKey: ["admin", "ai-act", "inventory", "stats"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ExportRow[]>(
				profile.data,
				"admin/ai-act/inventory/export?format=json",
			);
		},
		enabled: !!profile.data,
	});

	const summary = useMemo(() => {
		const rows = stats.data ?? [];
		const assessed = rows.filter(
			(row) => row.status.toUpperCase() !== "UNASSESSED",
		);
		const byRisk: Record<RiskCategory, number> = {
			PROHIBITED: 0,
			HIGH: 0,
			LIMITED: 0,
			MINIMAL: 0,
			UNDETERMINED: 0,
		};
		let needsReview = 0;
		let scoreSum = 0;
		let scoreCount = 0;
		for (const row of rows) {
			byRisk[row.riskCategory] = (byRisk[row.riskCategory] ?? 0) + 1;
			if (row.status.toUpperCase() === "SUBMITTED") needsReview += 1;
			if (typeof row.conformityScore === "number") {
				scoreSum += row.conformityScore;
				scoreCount += 1;
			}
		}
		return {
			total: assessed.length,
			byRisk,
			needsReview,
			avgScore: scoreCount > 0 ? Math.round(scoreSum / scoreCount) : null,
		};
	}, [stats.data]);

	const loading = stats.isLoading;

	return (
		<div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
			<StatCard
				icon={FileText}
				label={t("assessedApps", "Assessed apps")}
				value={summary.total}
				loading={loading}
			/>
			<StatCard
				icon={Ban}
				label="Prohibited"
				value={summary.byRisk.PROHIBITED}
				tone={
					summary.byRisk.PROHIBITED > 0 ? "text-red-600" : "text-foreground"
				}
				hint="Art. 5"
				loading={loading}
			/>
			<StatCard
				icon={ShieldAlert}
				label={t("highRisk", "High risk")}
				value={summary.byRisk.HIGH}
				tone={summary.byRisk.HIGH > 0 ? "text-orange-600" : "text-foreground"}
				hint="Annex III"
				loading={loading}
			/>
			<StatCard
				icon={ShieldQuestion}
				label={t("limitedRisk", "Limited risk")}
				value={summary.byRisk.LIMITED}
				hint="Art. 50"
				loading={loading}
			/>
			<StatCard
				icon={Activity}
				label={t("awaitingReview", "Awaiting review")}
				value={summary.needsReview}
				tone={summary.needsReview > 0 ? "text-blue-600" : "text-foreground"}
				loading={loading}
			/>
			<StatCard
				icon={Gauge}
				label={t("avgConformity", "Avg. conformity")}
				value={summary.avgScore === null ? "—" : `${summary.avgScore}`}
				tone={
					summary.avgScore === null
						? "text-muted-foreground"
						: bandTextColor(
								summary.avgScore >= 80
									? "green"
									: summary.avgScore >= 50
										? "amber"
										: "red",
							)
				}
				hint="of 100"
				loading={loading}
			/>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Inventory tab
// ---------------------------------------------------------------------------

function InventoryTab({
	onSelectApp,
}: {
	onSelectApp: (appId: string) => void;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [search, setSearch] = useState("");
	const [risk, setRisk] = useState("all");
	const [status, setStatus] = useState("all");
	const [page, setPage] = useState(1);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			page,
			limit: PAGE_SIZE,
		};
		if (search.trim()) params.search = search.trim();
		if (risk !== "all") params.risk = risk;
		if (status !== "all") params.status = status;
		return params;
	}, [page, search, risk, status]);

	const inventory = useQuery<InventoryResponse>({
		queryKey: ["admin", "ai-act", "inventory", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<InventoryResponse>(
				profile.data,
				`admin/ai-act/inventory?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const exportInventory = useMutation({
		mutationFn: async (format: "csv" | "json") => {
			if (!profile.data) throw new Error("Profile not loaded");
			const rows = await backend.apiState.get<Array<Record<string, unknown>>>(
				profile.data,
				"admin/ai-act/inventory/export?format=json",
			);
			if (format === "json") {
				triggerDownload(
					"ai-act-inventory.json",
					JSON.stringify(rows, null, 2),
					"application/json",
				);
				return;
			}
			const headers = [
				`appId`,
				`appName`,
				"riskCategory",
				"status",
				"conformityScore",
				"conformityBand",
				"securityScore",
				"privacyScore",
				"worstScore",
				"modelCount",
				"unvettedModelCount",
				"driftCount",
				"updatedAt",
			];
			const csv = [
				headers.join(","),
				...rows.map((r) =>
					headers
						.map((h) => {
							const v = r[h];
							const s = v === null || v === undefined ? "" : String(v);
							return `"${s.replace(/"/g, '""')}"`;
						})
						.join(","),
				),
			].join("\n");
			triggerDownload("ai-act-inventory.csv", csv, "text/csv");
		},
		onError: (err: Error) => toast.error(err.message ?? "Export failed"),
	});

	const totalPages = Math.max(
		1,
		Math.ceil((inventory.data?.total ?? 0) / PAGE_SIZE),
	);

	const resetPage = useCallback(() => setPage(1), []);
	const hasFilters = search.trim() !== "" || risk !== "all" || status !== "all";
	const items = inventory.data?.items ?? [];

	return (
		<div className="space-y-4">
			<InventoryStats />

			<div className="flex flex-wrap items-center gap-2">
				<div className="relative flex-1 min-w-[200px]">
					<Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						value={search}
						onChange={(e) => {
							setSearch(e.target.value);
							resetPage();
						}}
						placeholder={t(
							"searchByAppNameOrId",
							"Search by app name or id...",
						)}
						className="pl-8"
					/>
				</div>
				<Select
					value={risk}
					onValueChange={(v) => {
						setRisk(v);
						resetPage();
					}}
				>
					<SelectTrigger className="w-[160px]">
						<SelectValue placeholder="Risk" />
					</SelectTrigger>
					<SelectContent>
						{RISK_FILTER_OPTIONS.map((o) => (
							<SelectItem key={o.value} value={o.value}>
								{o.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Select
					value={status}
					onValueChange={(v) => {
						setStatus(v);
						resetPage();
					}}
				>
					<SelectTrigger className="w-[160px]">
						<SelectValue placeholder="Status" />
					</SelectTrigger>
					<SelectContent>
						{STATUS_FILTER_OPTIONS.map((o) => (
							<SelectItem key={o.value} value={o.value}>
								{o.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Button
					variant="outline"
					size="sm"
					disabled={exportInventory.isPending || !profile.data}
					onClick={() => exportInventory.mutate("csv")}
				>
					<Download className="mr-2 h-4 w-4" />
					CSV
				</Button>
				<Button
					variant="outline"
					size="sm"
					disabled={exportInventory.isPending || !profile.data}
					onClick={() => exportInventory.mutate("json")}
				>
					<Download className="mr-2 h-4 w-4" />
					{`JSON`}
				</Button>
			</div>

			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>{t("application", "Application")}</TableHead>
								<TableHead>{t("riskClass", "Risk class")}</TableHead>
								<TableHead>{t("status", "Status")}</TableHead>
								<TableHead>{t("conformity", "Conformity")}</TableHead>
								<TableHead className="text-center">
									<Tooltip>
										<TooltipTrigger asChild>
											<span className="cursor-default">{t("sec", "Sec")}</span>
										</TooltipTrigger>
										<TooltipContent>
											{t(
												"securityScoreWorstBoard",
												"Security score (worst board)",
											)}
										</TooltipContent>
									</Tooltip>
								</TableHead>
								<TableHead className="text-center">
									<Tooltip>
										<TooltipTrigger asChild>
											<span className="cursor-default">
												{t("priv", "Priv")}
											</span>
										</TooltipTrigger>
										<TooltipContent>
											{t(
												"privacyScoreWorstBoard",
												"Privacy score (worst board)",
											)}
										</TooltipContent>
									</Tooltip>
								</TableHead>
								<TableHead className="text-center">
									<Tooltip>
										<TooltipTrigger asChild>
											<span className="cursor-default">
												{t("worst", "Worst")}
											</span>
										</TooltipTrigger>
										<TooltipContent>
											{t(
												"lowestQualityScoreAcrossAllCategories",
												"Lowest quality score across all categories",
											)}
										</TooltipContent>
									</Tooltip>
								</TableHead>
								<TableHead className="text-center">
									{t("models", "Models")}
								</TableHead>
								<TableHead className="text-center">
									{t("unvetted", "Unvetted")}
								</TableHead>
								<TableHead className="text-center">
									{t("drift", "Drift")}
								</TableHead>
								<TableHead>{t("updated2", "Updated")}</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{inventory.isLoading &&
								["s1", "s2", "s3", "s4", "s5", "s6"].map((k) => (
									<TableRow key={`skeleton-${k}`}>
										<TableCell colSpan={11}>
											<Skeleton className="h-6 w-full" />
										</TableCell>
									</TableRow>
								))}
							{!inventory.isLoading && items.length === 0 && (
								<TableRow>
									<TableCell colSpan={11} className="py-10">
										<div className="flex flex-col items-center gap-1 text-center">
											<ShieldQuestion className="h-6 w-6 text-muted-foreground" />
											<p className="text-sm font-medium">
												{hasFilters
													? t(
															"noApplicationsMatchYourFilters",
															"No applications match your filters.",
														)
													: t("noInventoryAppsYet", "No inventory apps yet.")}
											</p>
											<p className="text-xs text-muted-foreground">
												{hasFilters
													? t(
															"tryClearingTheSearchOrRiskstatusFilters",
															"Try clearing the search or risk/status filters.",
														)
													: t(
															"appsAppearHereAfterTheyHaveGovernanceScoresModelObservationsOrEuAiActAssessments",
															"Apps appear here after they have governance scores, model observations, or EU AI Act assessments.",
														)}
											</p>
										</div>
									</TableCell>
								</TableRow>
							)}
							{items.map((item) => (
								<TableRow
									key={item.appId}
									className="cursor-pointer"
									onClick={() => onSelectApp(item.appId)}
								>
									<TableCell className="font-medium">
										{item.appName ?? item.appId}
									</TableCell>
									<TableCell>
										<RiskBadge risk={item.riskCategory} withTooltip />
									</TableCell>
									<TableCell>
										<StatusBadge status={item.status} />
									</TableCell>
									<TableCell>
										{typeof item.conformityScore === "number" ? (
											<div className="flex items-center gap-2">
												<Progress
													value={item.conformityScore}
													className="h-1.5 w-16"
												/>
												<span
													className={`text-sm font-semibold ${bandTextColor(item.conformityBand)}`}
												>
													{item.conformityScore}
												</span>
											</div>
										) : (
											<span className="text-sm text-muted-foreground">—</span>
										)}
									</TableCell>
									<TableCell className="text-center">
										{typeof item.securityScore === "number" ? (
											<ScoreChip value={item.securityScore} />
										) : (
											<span className="text-muted-foreground">—</span>
										)}
									</TableCell>
									<TableCell className="text-center">
										{typeof item.privacyScore === "number" ? (
											<ScoreChip value={item.privacyScore} />
										) : (
											<span className="text-muted-foreground">—</span>
										)}
									</TableCell>
									<TableCell className="text-center">
										{typeof item.worstScore === "number" ? (
											<span
												className={`text-base font-bold ${scoreText(item.worstScore)}`}
											>
												{item.worstScore}
											</span>
										) : (
											<span className="text-muted-foreground">—</span>
										)}
									</TableCell>
									<TableCell className="text-center">
										{item.modelCount}
									</TableCell>
									<TableCell className="text-center">
										{item.unvettedModelCount > 0 ? (
											<span className="font-medium text-amber-600">
												{item.unvettedModelCount}
											</span>
										) : (
											<span className="text-muted-foreground">0</span>
										)}
									</TableCell>
									<TableCell className="text-center">
										{item.driftCount > 0 ? (
											<span className="font-medium text-red-600">
												{item.driftCount}
											</span>
										) : (
											<span className="text-muted-foreground">0</span>
										)}
									</TableCell>
									<TableCell>
										<RelativeTime value={item.updatedAt} />
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</CardContent>
			</Card>

			<div className="flex items-center justify-between">
				<p className="text-sm text-muted-foreground">
					{inventory.data?.total ?? 0}{" "}
					{t("appS", {
						defaultValue_one: "app",
						defaultValue_other: "apps",
						count: inventory.data?.total ?? 0,
					})}
				</p>
				<div className="flex items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						disabled={page <= 1}
						onClick={() => setPage((p) => Math.max(1, p - 1))}
					>
						<ChevronLeft className="h-4 w-4" />
					</Button>
					<span className="text-sm">{`${page} / ${totalPages}`}</span>
					<Button
						variant="outline"
						size="sm"
						disabled={!inventory.data?.hasMore}
						onClick={() => setPage((p) => p + 1)}
					>
						<ChevronRight className="h-4 w-4" />
					</Button>
				</div>
			</div>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Inventory detail
// ---------------------------------------------------------------------------

function InventoryDetail({
	appId,
	onBack,
	onRegistryModelOpen,
}: {
	appId: string;
	onBack: () => void;
	onRegistryModelOpen?: (provider: string, modelId: string) => void;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const detail = useQuery<InventoryDetailResponse>({
		queryKey: ["admin", "ai-act", "inventory", appId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<InventoryDetailResponse>(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}`,
			);
		},
		enabled: !!profile.data,
	});

	const reconcile = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}/reconcile-models`,
				{},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "inventory", appId],
			});
			toast.success("Models reconciled.");
		},
		onError: (err: Error) => toast.error(err.message ?? "Reconcile failed"),
	});

	const acknowledge = useMutation({
		mutationFn: async (observationId: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}/models/${encodeURIComponent(observationId)}/acknowledge`,
				{},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "inventory", appId],
			});
			toast.success("Drift acknowledged.");
		},
		onError: (err: Error) => toast.error(err.message ?? "Acknowledge failed"),
	});

	const assist = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<{
				suggestion?: { purpose?: string; notes?: string };
			}>(profile.data, `admin/ai-act/assist/${encodeURIComponent(appId)}`, {
				profile: profile.data,
			});
		},
		onSuccess: (res) => {
			toast.success(
				res.suggestion?.purpose
					? t("governanceAgentPurpose", "Governance agent: {{purpose}}", {
							purpose: res.suggestion.purpose,
						})
					: t("governanceAgentCompleted", "Governance agent completed."),
			);
		},
		onError: (err: Error) => toast.error(err.message ?? "Assist failed"),
	});

	const data = detail.data;
	const [editOpen, setEditOpen] = useState(false);

	return (
		<div className="space-y-4">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="flex items-center gap-2">
					<Button variant="ghost" size="sm" onClick={onBack}>
						<ArrowLeft className="h-4 w-4 mr-1" />
						{t("back", "Back")}
					</Button>
					<div>
						<h2 className="text-lg font-semibold leading-tight">
							{data?.appName ?? appId}
						</h2>
						<p className="text-xs text-muted-foreground">
							{t("euAiActConformityRecord", "EU AI Act conformity record")}
						</p>
					</div>
				</div>
				<div className="flex items-center gap-2">
					<Button
						variant="default"
						size="sm"
						disabled={!data || !profile.data}
						onClick={() => setEditOpen(true)}
					>
						<Pencil className="mr-2 h-4 w-4" />
						{t("editAssessment", "Edit assessment")}
					</Button>
					<Button
						variant="outline"
						size="sm"
						disabled={assist.isPending || !profile.data}
						onClick={() => assist.mutate()}
					>
						<Sparkles
							className={`mr-2 h-4 w-4 ${assist.isPending ? "animate-pulse" : ""}`}
						/>
						{t("runGovernanceAgent", "Run governance agent")}
					</Button>
					<Button
						variant="outline"
						size="sm"
						disabled={reconcile.isPending || !profile.data}
						onClick={() => reconcile.mutate()}
					>
						<RefreshCw
							className={`mr-2 h-4 w-4 ${reconcile.isPending ? "animate-spin" : ""}`}
						/>
						{t("reconcileModels", "Reconcile models")}
					</Button>
				</div>
			</div>

			{detail.isLoading && <Skeleton className="h-64 w-full" />}

			{data && (
				<>
					<ConformityOverview data={data} />
					<ConformityRecommendations
						recommendations={data.recommendations}
						score={data.classification.conformityScore}
						onEdit={() => setEditOpen(true)}
					/>
					<SecurityScores appId={appId} />
					<TransparencyObligations classification={data.classification} />
					<ClassificationRationale rationale={data.classification.rationale} />
					<QuestionnaireSummary data={data} />
					<AttachedModels
						models={data.models}
						onAcknowledge={(id) => acknowledge.mutate(id)}
						onRegistryModelOpen={onRegistryModelOpen}
						acknowledging={acknowledge.isPending}
					/>
					<EditAssessmentDialog
						open={editOpen}
						onOpenChange={setEditOpen}
						appId={appId}
						data={data}
					/>
				</>
			)}
		</div>
	);
}

const REVIEW_STATUS_OPTIONS = [
	{ value: "DRAFT", label: "Draft" },
	{ value: "SUBMITTED", label: "Submitted (awaiting review)" },
	{ value: "APPROVED", label: "Approved" },
	{ value: "REJECTED", label: "Rejected" },
] as const;

interface AdminAssessmentPayload {
	answers: Record<string, unknown>;
	reviewStatus: string | null;
	reviewNote: string | null;
}

function EditAssessmentDialog({
	open,
	onOpenChange,
	appId,
	data,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	appId: string;
	data: InventoryDetailResponse;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [answers, setAnswers] = useState<Record<string, unknown>>(() => ({
		...data.answers,
	}));
	const [reviewStatus, setReviewStatus] = useState<string>(
		data.assessment?.status ?? "DRAFT",
	);
	const [reviewNote, setReviewNote] = useState(
		data.assessment?.reviewNote ?? "",
	);

	useEffect(() => {
		if (open) {
			setAnswers({ ...data.answers });
			setReviewStatus(data.assessment?.status ?? "DRAFT");
			setReviewNote(data.assessment?.reviewNote ?? "");
		}
	}, [open, data]);

	const setAnswer = useCallback((key: string, value: unknown) => {
		setAnswers((prev) => ({ ...prev, [key]: value }));
	}, []);

	const hasObligations =
		(data.classification.transparencyObligations?.length ?? 0) > 0;

	const save = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const payload: AdminAssessmentPayload = {
				answers,
				reviewStatus,
				reviewNote: reviewNote.trim() || null,
			};
			return backend.apiState.put(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}/assessment`,
				payload,
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "inventory", appId],
			});
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "inventory"],
			});
			toast.success("Assessment updated. Score recomputed.");
			onOpenChange(false);
		},
		onError: (err: Error) => toast.error(err.message ?? "Update failed"),
	});

	const visibleScreens = data.schema.screens.filter(
		(screen) =>
			!screen.highRiskOnly ||
			data.classification.riskCategory === "HIGH" ||
			data.classification.riskCategory === "UNDETERMINED",
	);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-3xl">
				<DialogHeader>
					<DialogTitle>
						{t("editConformityAssessment", "Edit conformity assessment")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"updateTheQuestionnaireAndReviewDecisionTheResponsiblePersonIsHardlinkedToTheAppOwnerAndTheRiskCategoryAndConformityScoreAreAlwaysRecomputedOnSave",
							"Update the questionnaire and review decision. The responsible person is hard-linked to the app owner, and the risk category and conformity score are always recomputed on save.",
						)}
					</DialogDescription>
				</DialogHeader>

				<DialogBody className="max-h-[60vh] space-y-5 overflow-y-auto pr-1">
					{visibleScreens.map((screen) => (
						<div key={screen.id} className="space-y-3">
							<div>
								<h4 className="text-sm font-semibold">{screen.title}</h4>
								{screen.description && (
									<p className="text-xs text-muted-foreground">
										{screen.description}
									</p>
								)}
							</div>
							{screen.questions
								.filter((q) => q.kind !== "contact")
								.map((question) => (
									<QuestionField
										key={question.key}
										question={question}
										value={answers[question.key]}
										onChange={(value) => setAnswer(question.key, value)}
									/>
								))}
						</div>
					))}

					{hasObligations && (
						<div className="flex items-center justify-between gap-3 rounded-lg border p-3">
							<div className="min-w-0">
								<Label className="text-sm">
									{t(
										"transparencyObligationsImplemented",
										"Transparency obligations implemented",
									)}
								</Label>
								<p className="text-xs text-muted-foreground">
									{t(
										"confirmTheTriggeredArt50DutiesAreInPlaceRaisesTheTransparencyScore",
										"Confirm the triggered Art. 50 duties are in place (raises the transparency score).",
									)}
								</p>
							</div>
							<Switch
								checked={answers.ack_transparency === "yes"}
								onCheckedChange={(checked) =>
									setAnswer("ack_transparency", checked ? "yes" : "no")
								}
							/>
						</div>
					)}

					<Separator />

					<div className="space-y-2">
						<h4 className="text-sm font-semibold">
							{t("responsiblePerson", "Responsible person")}
						</h4>
						<div className="flex items-center gap-3 rounded-lg border bg-muted/30 p-3">
							<UserCheck className="h-4 w-4 shrink-0 text-muted-foreground" />
							<div className="min-w-0">
								<p className="text-sm font-medium">
									{data.assessment?.responsibleName ??
										data.assessment?.responsibleEmail ??
										t("appOwner", "App owner")}
								</p>
								<p className="text-xs text-muted-foreground">
									{data.assessment?.responsibleEmail
										? `${data.assessment.responsibleEmail} · `
										: ""}
									{t(
										"hardlinkedToTheAppOwnerArt26CannotBeReassigned",
										"Hard-linked to the app owner (Art. 26) — cannot be reassigned.",
									)}
								</p>
							</div>
						</div>
					</div>

					<Separator />

					<div className="space-y-3">
						<h4 className="text-sm font-semibold">
							{t("reviewDecision2", "Review decision")}
						</h4>
						<div className="grid gap-3 sm:grid-cols-2">
							<div className="space-y-1.5">
								<Label className="text-xs">{t("status", "Status")}</Label>
								<Select value={reviewStatus} onValueChange={setReviewStatus}>
									<SelectTrigger>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{REVIEW_STATUS_OPTIONS.map((option) => (
											<SelectItem key={option.value} value={option.value}>
												{option.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						</div>
						<div className="space-y-1.5">
							<Label htmlFor="review-note" className="text-xs">
								{t("reviewNote", "Review note")}
							</Label>
							<Textarea
								id="review-note"
								value={reviewNote}
								onChange={(e) => setReviewNote(e.target.value)}
								placeholder={t(
									"optionalRationaleRecordedWithTheReviewDecision",
									"Optional rationale recorded with the review decision.",
								)}
								rows={3}
							/>
						</div>
						{data.classification.blocked && (
							<Alert variant="destructive">
								<Ban className="h-4 w-4" />
								<AlertTitle>
									{t(
										"prohibitedPracticeDeclared",
										"Prohibited practice declared",
									)}
								</AlertTitle>
								<AlertDescription>
									{t(
										"whileAnArt5ProhibitedPracticeIsSelectedTheAssessmentIsForcedToBlockedRegardlessOfTheChosenStatus",
										"While an Art. 5 prohibited practice is selected, the assessment is forced to BLOCKED regardless of the chosen status.",
									)}
								</AlertDescription>
							</Alert>
						)}
					</div>
				</DialogBody>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => onOpenChange(false)}
						disabled={save.isPending}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button
						onClick={() => save.mutate()}
						disabled={save.isPending || !profile.data}
					>
						{save.isPending
							? "Saving…"
							: t("saveRecompute", "Save & recompute")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function QuestionField({
	question,
	value,
	onChange,
}: {
	question: Question;
	value: unknown;
	onChange: (value: unknown) => void;
}) {
	if (question.kind === "yesno") {
		return (
			<div className="flex items-center justify-between gap-3 rounded-lg border p-3">
				<div className="min-w-0">
					<Label className="text-sm">{question.label}</Label>
					{question.help && (
						<p className="text-xs text-muted-foreground">{question.help}</p>
					)}
				</div>
				<Switch
					checked={value === "yes"}
					onCheckedChange={(checked) => onChange(checked ? "yes" : "no")}
				/>
			</div>
		);
	}

	if (question.kind === "text") {
		return (
			<div className="space-y-1.5">
				<Label className="text-sm">{question.label}</Label>
				<Input
					value={typeof value === "string" ? value : ""}
					onChange={(e) => onChange(e.target.value)}
					placeholder={question.help ?? ""}
				/>
			</div>
		);
	}

	if (question.kind === "select") {
		return (
			<div className="space-y-1.5">
				<Label className="text-sm">{question.label}</Label>
				<Select
					value={typeof value === "string" ? value : ""}
					onValueChange={onChange}
				>
					<SelectTrigger>
						<SelectValue placeholder="Select…" />
					</SelectTrigger>
					<SelectContent>
						{(question.options ?? []).map((option) => (
							<SelectItem key={option.value} value={option.value}>
								{option.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
		);
	}

	if (question.kind === "multi") {
		const selected = Array.isArray(value) ? (value as string[]) : [];
		const toggle = (optionValue: string, checked: boolean) => {
			const next = checked
				? [...selected, optionValue]
				: selected.filter((v) => v !== optionValue);
			onChange(next);
		};
		return (
			<div className="space-y-2">
				<Label className="text-sm">{question.label}</Label>
				{question.help && (
					<p className="text-xs text-muted-foreground">{question.help}</p>
				)}
				<div className="space-y-1.5">
					{(question.options ?? []).map((option) => (
						<div
							key={option.value}
							className="flex items-start gap-2 rounded-md border p-2 text-sm"
						>
							<Checkbox
								id={`q-${question.key}-${option.value}`}
								checked={selected.includes(option.value)}
								onCheckedChange={(checked) =>
									toggle(option.value, checked === true)
								}
							/>
							<Label
								htmlFor={`q-${question.key}-${option.value}`}
								className="cursor-pointer font-normal"
							>
								<span className="font-medium">{option.label}</span>
								{option.help && (
									<span className="block text-xs text-muted-foreground">
										{option.help}
									</span>
								)}
							</Label>
						</div>
					))}
				</div>
			</div>
		);
	}

	return null;
}

function SecurityScores({ appId }: { appId: string }) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const detail = useQuery<AppScoreDetailResponse>({
		queryKey: ["admin", "governance", "scores", appId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<AppScoreDetailResponse>(
				profile.data,
				`admin/governance/scores/${encodeURIComponent(appId)}`,
			);
		},
		enabled: !!profile.data,
	});

	const boards = detail.data?.boards ?? [];

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-sm flex items-center gap-2">
					<ShieldCheck className="h-4 w-4" />
					{t("securityAmpQualityScores", "Security & Quality Scores")}
				</CardTitle>
				<CardDescription>
					{`Per-board governance scores (0–10, worst-first) and flagged low-score nodes from the latest static analysis.`}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{detail.isLoading && <Skeleton className="h-24 w-full" />}
				{!detail.isLoading && boards.length === 0 && (
					<p className="text-sm text-muted-foreground">
						{t(
							"noBoardScoresRecordedYetScoresAreComputedWhenTheAppsBoardsAreAnalysed",
							"No board scores recorded yet. Scores are computed when the app's boards are analysed.",
						)}
					</p>
				)}
				{boards.map((board) => (
					<div key={board.boardId} className="rounded-lg border p-3 space-y-3">
						<div className="flex items-center justify-between gap-3">
							<div className="min-w-0">
								<p className="truncate font-mono text-xs text-muted-foreground">
									{board.boardId}
								</p>
								<p className="text-xs text-muted-foreground">
									{`${board.scoredNodeCount}/${board.nodeCount} nodes scored · updated`}{" "}
									<RelativeTime
										value={board.updatedAt}
										fallback={board.updatedAt}
									/>
								</p>
							</div>
							<span
								className={`text-lg font-bold ${scoreText(board.worstScore)}`}
							>
								{board.worstScore}
							</span>
						</div>

						<div className="flex flex-wrap gap-3">
							{SCORE_CATEGORIES.map((category) => (
								<div
									key={category}
									className="flex items-center gap-1.5 text-xs"
								>
									<ScoreChip value={board[category]} />
									<span className="capitalize text-muted-foreground">
										{category}
									</span>
								</div>
							))}
						</div>

						{board.flaggedPatterns.length > 0 && (
							<div className="space-y-1.5 border-t pt-3">
								<p className="flex items-center gap-1.5 text-xs font-medium">
									<ShieldAlert className="h-3.5 w-3.5 text-red-500" />
									{t("flaggedNodesLength", "Flagged nodes ({{length}})", {
										length: board.flaggedPatterns.length,
									})}
								</p>
								<div className="flex flex-wrap gap-1.5">
									{board.flaggedPatterns.map((pattern, index) => (
										<Badge
											key={`${pattern.node}-${pattern.category}-${index}`}
											variant="outline"
											className="text-[11px]"
										>
											<span className="font-medium">{pattern.node}</span>
											<span className="mx-1 text-muted-foreground">
												{pattern.category}
											</span>
											<span className={scoreText(pattern.score)}>
												{pattern.score}
											</span>
											{(pattern.count ?? 1) > 1 && (
												<span className="ml-1 text-muted-foreground">{`×${pattern.count}`}</span>
											)}
										</Badge>
									))}
								</div>
							</div>
						)}
					</div>
				))}
			</CardContent>
		</Card>
	);
}

function ConformityOverview({ data }: { data: InventoryDetailResponse }) {
	const { t } = useTranslation("admin");
	const { classification, assessment, hasAssessment } = data;
	const meta = riskMeta(classification.riskCategory);
	const Icon = meta.icon;
	const score = classification.conformityScore;
	const band = classification.conformityBand;
	const statusLabel = hasAssessment
		? (assessment?.status ?? "DRAFT")
		: t("notSubmitted", "NOT SUBMITTED");

	return (
		<Card>
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div className="space-y-1">
						<CardTitle className="text-base">
							{t("conformityAssessment", "Conformity Assessment")}
						</CardTitle>
						<CardDescription>
							{hasAssessment
								? t(
										"theOwnersSubmittedAssessmentAlongsideTheLiveRecomputedClassification",
										"The owner's submitted assessment alongside the live, recomputed classification.",
									)
								: `No assessment submitted yet. Classification below is auto-derived from board signals.`}
						</CardDescription>
					</div>
					<StatusBadge status={statusLabel} />
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				{classification.blocked && (
					<Alert variant="destructive">
						<Ban className="h-4 w-4" />
						<AlertTitle>
							{t(
								"publicationBlockedProhibitedPractice",
								"Publication blocked — prohibited practice",
							)}
						</AlertTitle>
						<AlertDescription>
							{t(
								"anArt5ProhibitedPracticeWasDetectedThisSystemMayNotBePlacedOnTheEuMarket",
								"An Art. 5 prohibited practice was detected. This system may not be placed on the EU market.",
							)}
						</AlertDescription>
					</Alert>
				)}

				<div className="grid gap-4 md:grid-cols-2">
					<div className="rounded-lg border p-4 space-y-2">
						<div className="flex items-center gap-2">
							<span
								className={`grid h-9 w-9 place-items-center rounded-md ${meta.badge}`}
							>
								<Icon className="h-5 w-5" />
							</span>
							<div>
								<p className="text-sm font-semibold">{meta.label}</p>
								<p className="text-xs text-muted-foreground">
									{meta.article !== "—"
										? t("articleEuAiAct", "{{article}} • EU AI Act", {
												article: meta.article,
											})
										: t("euAiAct", "EU AI Act")}
								</p>
							</div>
						</div>
						<p className="text-xs text-muted-foreground">{meta.description}</p>
					</div>

					<div className="rounded-lg border p-4 space-y-3">
						<div className="flex items-center justify-between">
							<span className="text-sm font-medium">
								{t("conformityScore", "Conformity score")}
							</span>
							{band && (
								<span className={`text-xs font-medium ${BAND_META[band].text}`}>
									{BAND_META[band].label}
								</span>
							)}
						</div>
						{typeof score === "number" ? (
							<>
								<div className="flex items-baseline gap-1">
									<span className={`text-3xl font-bold ${bandTextColor(band)}`}>
										{score}
									</span>
									<span className="text-sm text-muted-foreground">/ 100</span>
								</div>
								<Progress value={score} className="h-2" />
							</>
						) : (
							<p className="text-sm text-muted-foreground">
								{t(
									"notScoredClassificationIs",
									"Not scored — classification is",
								)}{" "}
								{classification.riskCategory === "PROHIBITED"
									? "blocked"
									: "undetermined"}
								.
							</p>
						)}
					</div>
				</div>

				{(assessment?.responsibleName ||
					assessment?.responsibleEmail ||
					assessment?.submittedAt ||
					assessment?.reviewedAt) && (
					<>
						<Separator />
						<div className="grid gap-x-6 gap-y-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
							<ResponsiblePersonField
								person={assessment?.responsiblePerson}
								fallbackName={assessment?.responsibleName}
								fallbackEmail={assessment?.responsibleEmail}
							/>
							<DetailField
								icon={FileText}
								label={t("submitted2", "Submitted")}
								value={
									assessment?.submittedAt ? (
										<RelativeTime value={assessment.submittedAt} />
									) : (
										"—"
									)
								}
							/>
							<DetailField
								icon={ShieldCheck}
								label="Reviewed"
								value={
									assessment?.reviewedAt ? (
										<span className="inline-flex flex-col">
											<RelativeTime value={assessment.reviewedAt} />
											{assessment?.reviewedByName && (
												<span className="text-xs text-muted-foreground">
													{t("byReviewedbyname", "by {{reviewedByName}}", {
														reviewedByName: assessment.reviewedByName,
													})}
												</span>
											)}
										</span>
									) : assessment?.status === "SUBMITTED" ? (
										<span className="text-amber-600 dark:text-amber-500">
											{t("pendingReview", "Pending review")}
										</span>
									) : (
										"—"
									)
								}
							/>
							<DetailField
								icon={ScrollText}
								label={t("schemaVersion", "Schema version")}
								value={`v${data.schema.version}`}
							/>
						</div>
						{assessment?.reviewNote && (
							<p className="rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
								<span className="font-medium text-foreground">
									{t("reviewNote2", "Review note:")}{" "}
								</span>
								{assessment.reviewNote}
							</p>
						)}
					</>
				)}
			</CardContent>
		</Card>
	);
}

function DetailField({
	icon: Icon,
	label,
	value,
}: {
	icon: LucideIcon;
	label: string;
	value: ReactNode;
}) {
	return (
		<div className="space-y-0.5">
			<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<Icon className="h-3.5 w-3.5" />
				{label}
			</span>
			<span className="text-sm font-medium">{value}</span>
		</div>
	);
}

function initialsFromName(name?: string | null, email?: string | null): string {
	const source = (name ?? email ?? "").trim();
	if (!source) return "?";
	const parts = source.split(/\s+/).filter(Boolean);
	if (parts.length >= 2) {
		return `${parts[0][0] ?? ""}${parts[1][0] ?? ""}`.toUpperCase();
	}
	return source.slice(0, 2).toUpperCase();
}

function ResponsiblePersonField({
	person,
	fallbackName,
	fallbackEmail,
}: {
	person?: ResponsiblePerson | null;
	fallbackName?: string | null;
	fallbackEmail?: string | null;
}) {
	const { t } = useTranslation("admin");
	const [open, setOpen] = useState(false);
	const name = person?.name ?? fallbackName ?? null;
	const email = person?.email ?? fallbackEmail ?? null;
	const username = person?.username ?? null;
	const avatar = person?.avatar ?? null;
	const description = person?.description ?? null;
	const hasContact = Boolean(name || email || username);

	const display = name || email || "—";

	if (!hasContact) {
		return (
			<DetailField
				icon={UserCheck}
				label={t("responsiblePerson", "Responsible person")}
				value="—"
			/>
		);
	}

	return (
		<>
			<div className="space-y-0.5">
				<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
					<UserCheck className="h-3.5 w-3.5" />
					{t("responsiblePerson", "Responsible person")}
				</span>
				<button
					type="button"
					onClick={() => setOpen(true)}
					className="group inline-flex items-center gap-2 rounded-md text-left text-sm font-medium text-primary underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<Avatar className="h-5 w-5">
						{avatar && <AvatarImage src={avatar} alt={display} />}
						<AvatarFallback className="bg-muted text-[10px]">
							{initialsFromName(name, email)}
						</AvatarFallback>
					</Avatar>
					<span className="truncate">{display}</span>
				</button>
			</div>

			<Sheet open={open} onOpenChange={setOpen}>
				<SheetContent className="sm:max-w-md">
					<SheetHeader>
						<SheetTitle>
							{t("responsiblePerson", "Responsible person")}
						</SheetTitle>
						<SheetDescription>
							{t(
								"accountableOwnerForThisAiSystemArt26ReachOutToCoordinateConformityActions",
								"Accountable owner for this AI system (Art. 26). Reach out to coordinate conformity actions.",
							)}
						</SheetDescription>
					</SheetHeader>

					<div className="mt-2 flex flex-1 flex-col gap-6 overflow-y-auto px-4 pb-6">
						<div className="flex items-center gap-4">
							<Avatar className="h-14 w-14">
								{avatar && <AvatarImage src={avatar} alt={display} />}
								<AvatarFallback className="bg-muted text-base">
									{initialsFromName(name, email)}
								</AvatarFallback>
							</Avatar>
							<div className="min-w-0 space-y-0.5">
								<p className="truncate text-base font-semibold">
									{name ?? username ?? email ?? "Unknown"}
								</p>
								{username && (
									<p className="truncate text-sm text-muted-foreground">{`@${username}`}</p>
								)}
							</div>
						</div>

						{description && (
							<p className="rounded-md bg-muted/40 p-3 text-sm text-muted-foreground">
								{description}
							</p>
						)}

						<div className="space-y-3">
							{email && (
								<div className="space-y-1">
									<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
										<Mail className="h-3.5 w-3.5" />
										{t("email", "Email")}
									</span>
									<div className="flex flex-wrap items-center gap-2">
										<a
											href={`mailto:${email}`}
											className="text-sm font-medium text-primary underline-offset-2 hover:underline"
										>
											{email}
										</a>
										<Button
											size="sm"
											variant="outline"
											onClick={() => {
												void navigator.clipboard?.writeText(email);
											}}
										>
											{t("copy", "Copy")}
										</Button>
									</div>
								</div>
							)}

							{person?.userId && (
								<div className="space-y-1">
									<span className="text-xs text-muted-foreground">
										{t("userId", "User ID")}
									</span>
									<p className="break-all font-mono text-xs text-muted-foreground">
										{person.userId}
									</p>
								</div>
							)}
						</div>

						{email && (
							<Button asChild className="w-full">
								<a href={`mailto:${email}`}>
									<Mail className="mr-2 h-4 w-4" />
									{t("contactResponsiblePerson", "Contact responsible person")}
								</a>
							</Button>
						)}
					</div>
				</SheetContent>
			</Sheet>
		</>
	);
}

function TransparencyObligations({
	classification,
}: {
	classification: Classification;
}) {
	const { t } = useTranslation("admin");
	const obligations = classification.transparencyObligations ?? [];

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-sm">
					{t("transparencyObligations", "Transparency Obligations")}
				</CardTitle>
				<CardDescription>
					{t(
						"disclosureAndOversightDutiesTriggeredForThisSystemArt50141112",
						"Disclosure and oversight duties triggered for this system (Art. 50, 14, 11–12).",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{obligations.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t(
							"noTransparencyObligationsAreTriggeredForThisRiskClass",
							"No transparency obligations are triggered for this risk class.",
						)}
					</p>
				) : (
					<div className="grid gap-3 sm:grid-cols-2">
						{obligations.map((key) => {
							const meta = TRANSPARENCY_META[key];
							return (
								<div key={key} className="flex gap-3 rounded-lg border p-3">
									<ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
									<div className="space-y-0.5">
										<div className="flex items-center gap-2">
											<span className="text-sm font-medium">
												{meta?.label ?? key}
											</span>
											{meta && (
												<Badge variant="outline" className="text-[10px]">
													{meta.article}
												</Badge>
											)}
										</div>
										<p className="text-xs text-muted-foreground">
											{meta?.description ?? ""}
										</p>
									</div>
								</div>
							);
						})}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function ClassificationRationale({ rationale }: { rationale: string[] }) {
	const { t } = useTranslation("admin");
	if (!rationale || rationale.length === 0) return null;
	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-sm flex items-center gap-2">
					<ScrollText className="h-4 w-4" />
					{t("whyThisClassification", "Why this classification")}
				</CardTitle>
				<CardDescription>
					{t(
						"auditTrailOfTheDominantFactorsBehindTheDetermination",
						"Audit trail of the dominant factors behind the determination.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<ul className="space-y-1.5">
					{rationale.map((line) => (
						<li key={line} className="flex gap-2 text-sm text-muted-foreground">
							<span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-primary/60" />
							{line}
						</li>
					))}
				</ul>
			</CardContent>
		</Card>
	);
}

function QuestionnaireSummary({ data }: { data: InventoryDetailResponse }) {
	const { t } = useTranslation("admin");
	const { schema, answers, classification, hasAssessment } = data;
	const screens = schema.screens.filter(
		(screen) =>
			!screen.highRiskOnly ||
			classification.riskCategory === "HIGH" ||
			classification.riskCategory === "PROHIBITED",
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-sm">
					{t("conformityQuestionnaire", "Conformity Questionnaire")}
				</CardTitle>
				<CardDescription>
					{hasAssessment
						? t(
								"answersSubmittedByTheApplicationOwner",
								"Answers submitted by the application owner.",
							)
						: `Auto-derived answers from board signals (no owner submission yet).`}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<Accordion
					type="multiple"
					defaultValue={screens.length > 0 ? [screens[0].id] : []}
				>
					{screens.map((screen) => (
						<AccordionItem key={screen.id} value={screen.id}>
							<AccordionTrigger className="text-sm">
								{screen.title}
							</AccordionTrigger>
							<AccordionContent>
								{screen.description && (
									<p className="mb-3 text-xs text-muted-foreground">
										{screen.description}
									</p>
								)}
								<dl className="grid gap-x-6 gap-y-3 sm:grid-cols-2">
									{screen.questions.map((question) => (
										<div key={question.key} className="space-y-0.5">
											<dt className="text-xs text-muted-foreground">
												{question.label}
											</dt>
											<dd className="text-sm font-medium">
												{formatAnswer(question, answers[question.key])}
											</dd>
										</div>
									))}
								</dl>
							</AccordionContent>
						</AccordionItem>
					))}
				</Accordion>
			</CardContent>
		</Card>
	);
}

function AttachedModels({
	models,
	onAcknowledge,
	onRegistryModelOpen,
	acknowledging,
}: {
	models: ModelObservationItem[];
	onAcknowledge: (id: string) => void;
	onRegistryModelOpen?: (provider: string, modelId: string) => void;
	acknowledging: boolean;
}) {
	const { t } = useTranslation("admin");
	const unvetted = models.filter((m) => !m.vetted).length;
	const drift = models.filter((m) => m.driftFlagged).length;

	return (
		<Card>
			<CardHeader>
				<div className="flex flex-wrap items-center justify-between gap-2">
					<CardTitle className="text-sm flex items-center gap-2">
						<Boxes className="h-4 w-4" />
						{t("attachedModels", "Attached Models")}
					</CardTitle>
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<span>
							{t("lengthTotal", "{{length}} total", { length: models.length })}
						</span>
						{unvetted > 0 && (
							<Badge variant="outline" className="text-amber-600">
								{t("unvettedUnvetted", "{{unvetted}} unvetted", { unvetted })}
							</Badge>
						)}
						{drift > 0 && (
							<Badge variant="outline" className="text-red-600">
								{t("driftDrift", "{{drift}} drift", { drift })}
							</Badge>
						)}
					</div>
				</div>
			</CardHeader>
			<CardContent className="p-0">
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead>{t("model", "Model")}</TableHead>
							<TableHead>{t("provider2", "Provider")}</TableHead>
							<TableHead>{t("source", "Source")}</TableHead>
							<TableHead>{t("gpaiPosture", "GPAI posture")}</TableHead>
							<TableHead className="text-center">
								{t("vetted", "Vetted")}
							</TableHead>
							<TableHead className="text-center">
								{t("flags", "Flags")}
							</TableHead>
							<TableHead className="text-right">
								{t("action", "Action")}
							</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{models.length === 0 && (
							<TableRow>
								<TableCell
									colSpan={7}
									className="py-8 text-center text-sm text-muted-foreground"
								>
									{t(
										"noModelsObservedRunReconcileToScanTheAppsBoards",
										"No models observed. Run reconcile to scan the app's boards.",
									)}
								</TableCell>
							</TableRow>
						)}
						{models.map((m) => (
							<TableRow key={m.id}>
								<TableCell className="font-medium">{m.modelId}</TableCell>
								<TableCell>{m.provider ?? "—"}</TableCell>
								<TableCell>
									<Badge variant="outline">{m.source}</Badge>
								</TableCell>
								<TableCell>
									<Badge variant="secondary">
										{POSTURE_LABEL[m.posture] ?? m.posture}
									</Badge>
								</TableCell>
								<TableCell className="text-center">
									{m.vetted ? (
										<span className="text-emerald-600">{t("yes", "Yes")}</span>
									) : (
										<span className="text-amber-600">{t("no", "No")}</span>
									)}
								</TableCell>
								<TableCell className="text-center">
									<div className="flex flex-wrap justify-center gap-1">
										{m.systemicRisk && (
											<Badge className="bg-red-600 text-white">
												{t("systemic", "Systemic")}
											</Badge>
										)}
										{m.dynamicSelector && (
											<Badge variant="outline">{t("dynamic", "Dynamic")}</Badge>
										)}
										{m.driftFlagged && (
											<Badge className="bg-amber-500 text-black">
												{t("drift", "Drift")}
											</Badge>
										)}
										{!m.systemicRisk &&
											!m.dynamicSelector &&
											!m.driftFlagged && (
												<span className="text-xs text-muted-foreground">—</span>
											)}
									</div>
								</TableCell>
								<TableCell className="text-right">
									<div className="flex justify-end gap-2">
										{!m.dynamicSelector && (
											<Button
												variant={m.vetted ? "ghost" : "outline"}
												size="sm"
												disabled={!onRegistryModelOpen}
												onClick={() =>
													onRegistryModelOpen?.(
														m.provider?.trim() || "unknown",
														m.modelId,
													)
												}
											>
												{m.vetted ? "Registry" : "Rate"}
											</Button>
										)}
										{m.driftFlagged && (
											<Button
												variant="ghost"
												size="sm"
												disabled={acknowledging}
												onClick={() => onAcknowledge(m.id)}
											>
												{t("acknowledge", "Acknowledge")}
											</Button>
										)}
									</div>
								</TableCell>
							</TableRow>
						))}
					</TableBody>
				</Table>
			</CardContent>
		</Card>
	);
}

// ---------------------------------------------------------------------------
// Registry tab
// ---------------------------------------------------------------------------

const EMPTY_REGISTRY_FORM = {
	provider: "",
	modelId: "",
	posture: "UNKNOWN",
	hosted: false,
	openLicence: false,
	systemicRisk: false,
	vetted: false,
	note: "",
};

function registryFormFromItem(item: RegistryItem) {
	return {
		provider: item.provider,
		modelId: item.modelId,
		posture: item.posture,
		hosted: item.hosted,
		openLicence: item.openLicence,
		systemicRisk: item.systemicRisk,
		vetted: item.vetted,
		note: item.note ?? "",
	};
}

const REGISTRY_TOGGLES = [
	["hosted", "Hosted", "Served by a provider's hosted endpoint."],
	["openLicence", "Open licence", "Distributed under an open / free licence."],
	["systemicRisk", "Systemic risk", "GPAI model with systemic risk (Art. 51)."],
	["vetted", "Vetted", "Reviewed and approved for use on the platform."],
] as const;

function RegistryTab({
	initialProvider,
	initialModelId,
}: {
	initialProvider?: string | null;
	initialModelId?: string | null;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [form, setForm] = useState({ ...EMPTY_REGISTRY_FORM });
	const hydratedInitialKeyRef = useRef("");
	const userEditedInitialFormRef = useRef(false);
	const initialSelectionKey =
		initialProvider && initialModelId
			? `${initialProvider}\u0000${initialModelId}`
			: "";

	useEffect(() => {
		if (!initialProvider || !initialModelId) return;
		hydratedInitialKeyRef.current = "";
		userEditedInitialFormRef.current = false;
		setForm((current) => ({
			...current,
			provider: initialProvider,
			modelId: initialModelId,
		}));
	}, [initialProvider, initialModelId]);

	const models = useQuery<RegistryItem[]>({
		queryKey: ["admin", "ai-act", "registry"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RegistryItem[]>(
				profile.data,
				"admin/ai-act/models",
			);
		},
		enabled: !!profile.data,
	});

	const selectedModel = useMemo(() => {
		const provider = form.provider.trim();
		const modelId = form.modelId.trim();
		if (!provider || !modelId) return undefined;
		return models.data?.find(
			(item) => item.provider === provider && item.modelId === modelId,
		);
	}, [form.provider, form.modelId, models.data]);

	useEffect(() => {
		if (
			!initialSelectionKey ||
			!selectedModel ||
			userEditedInitialFormRef.current ||
			hydratedInitialKeyRef.current === initialSelectionKey ||
			selectedModel.provider !== initialProvider ||
			selectedModel.modelId !== initialModelId
		) {
			return;
		}

		hydratedInitialKeyRef.current = initialSelectionKey;
		setForm(registryFormFromItem(selectedModel));
	}, [initialModelId, initialProvider, initialSelectionKey, selectedModel]);
	const updateRegistryForm = useCallback(
		(update: Partial<typeof EMPTY_REGISTRY_FORM>) => {
			userEditedInitialFormRef.current = true;
			setForm((current) => ({ ...current, ...update }));
		},
		[],
	);

	const formTitle = selectedModel?.registered
		? t("updateModel", "Update model")
		: selectedModel?.observed
			? t("rateObservedModel", "Rate observed model")
			: t("addModel", "Add model");
	const submitLabel = selectedModel?.registered
		? t("saveChanges", "Save changes")
		: selectedModel?.observed
			? t("saveRating", "Save rating")
			: t("addModel", "Add model");

	const upsert = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.put(profile.data, "admin/ai-act/models", form);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "registry"],
			});
			await queryClient.invalidateQueries({
				queryKey: ["admin", "ai-act", "inventory"],
			});
			setForm({ ...EMPTY_REGISTRY_FORM });
			toast.success("Registry entry saved.");
		},
		onError: (err: Error) => toast.error(err.message ?? "Save failed"),
	});

	const editEntry = useCallback((item: RegistryItem) => {
		userEditedInitialFormRef.current = true;
		setForm(registryFormFromItem(item));
	}, []);

	return (
		<div className="space-y-4">
			<Alert>
				<FileText className="h-4 w-4" />
				<AlertTitle>
					{t(
						"generalpurposeAiModelRegister",
						"General-purpose AI model register",
					)}
				</AlertTitle>
				<AlertDescription>
					{t(
						"recordTheGpaiPostureOfEveryModelUsedAcrossThePlatformSoAttachedmodelGovernanceAndDriftDetectionCanClassifyThemCorrectly",
						"Record the GPAI posture of every model used across the platform so attached-model governance and drift detection can classify them correctly.",
					)}
				</AlertDescription>
			</Alert>

			<Card>
				<CardHeader>
					<CardTitle className="text-sm flex items-center gap-2">
						<Plus className="h-4 w-4" />
						{formTitle}
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-3 sm:grid-cols-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("provider2", "Provider")}</Label>
							<Input
								value={form.provider}
								onChange={(e) =>
									updateRegistryForm({ provider: e.target.value })
								}
								placeholder="openai"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("modelId", "Model ID")}</Label>
							<Input
								value={form.modelId}
								onChange={(e) =>
									updateRegistryForm({ modelId: e.target.value })
								}
								placeholder="gpt-4o"
							/>
						</div>
					</div>
					<div className="grid gap-3 sm:grid-cols-2">
						<div className="space-y-1">
							<Label className="text-xs">
								{t("gpaiPosture", "GPAI posture")}
							</Label>
							<Select
								value={form.posture}
								onValueChange={(posture) => updateRegistryForm({ posture })}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="UNKNOWN">
										{t("unknown", "Unknown")}
									</SelectItem>
									<SelectItem value="HOSTED">
										{t("hosted", "Hosted")}
									</SelectItem>
									<SelectItem value="OPEN_LICENCE">
										{t("openLicence", "Open licence")}
									</SelectItem>
									<SelectItem value="CLOSED">
										{t("closed", "Closed")}
									</SelectItem>
									<SelectItem value="SYSTEMIC">
										{t("systemicRisk", "Systemic risk")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("note", "Note")}</Label>
							<Input
								value={form.note}
								onChange={(e) => updateRegistryForm({ note: e.target.value })}
								placeholder="Optional"
							/>
						</div>
					</div>
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						{REGISTRY_TOGGLES.map(([key, label, hint]) => (
							<div
								key={key}
								className="flex items-start justify-between gap-2 rounded-lg border p-3"
							>
								<div className="space-y-0.5">
									<p className="text-sm font-medium">{label}</p>
									<p className="text-xs text-muted-foreground">{hint}</p>
								</div>
								<Switch
									checked={form[key] as boolean}
									onCheckedChange={(checked) =>
										updateRegistryForm({ [key]: checked })
									}
								/>
							</div>
						))}
					</div>
					<div className="flex justify-end gap-2">
						<Button
							variant="ghost"
							size="sm"
							onClick={() => {
								userEditedInitialFormRef.current = true;
								setForm({ ...EMPTY_REGISTRY_FORM });
							}}
						>
							{t("clear", "Clear")}
						</Button>
						<Button
							size="sm"
							disabled={
								upsert.isPending ||
								!form.provider.trim() ||
								!form.modelId.trim()
							}
							onClick={() => upsert.mutate()}
						>
							{submitLabel}
						</Button>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>{t("provider2", "Provider")}</TableHead>
								<TableHead>{t("model", "Model")}</TableHead>
								<TableHead>{t("posture", "Posture")}</TableHead>
								<TableHead className="text-center">
									{t("vetted", "Vetted")}
								</TableHead>
								<TableHead>{t("note", "Note")}</TableHead>
								<TableHead className="text-right">
									{t("action", "Action")}
								</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{models.isLoading &&
								["r1", "r2", "r3", "r4"].map((k) => (
									<TableRow key={`reg-skel-${k}`}>
										<TableCell colSpan={6}>
											<Skeleton className="h-6 w-full" />
										</TableCell>
									</TableRow>
								))}
							{!models.isLoading && (models.data?.length ?? 0) === 0 && (
								<TableRow>
									<TableCell
										colSpan={6}
										className="py-8 text-center text-sm text-muted-foreground"
									>
										{`No registered or observed models yet. Run reconcile from an app inventory detail to scan its boards.`}
									</TableCell>
								</TableRow>
							)}
							{models.data?.map((item) => (
								<TableRow key={item.id}>
									<TableCell className="font-medium">{item.provider}</TableCell>
									<TableCell>
										<div className="space-y-1">
											<div>{item.modelId}</div>
											<div className="flex flex-wrap gap-1">
												{item.needsRating && (
													<Badge variant="outline" className="text-amber-600">
														{t("needsRating", "Needs rating")}
													</Badge>
												)}
												{item.observed && (
													<Badge variant="outline">
														{item.observedCount > 1
															? t(
																	"observedcountObservations",
																	"{{observedCount}} observations",
																	{ observedCount: item.observedCount },
																)
															: "Observed"}
													</Badge>
												)}
												{!item.registered && (
													<Badge variant="secondary">
														{t("unregistered", "Unregistered")}
													</Badge>
												)}
											</div>
										</div>
									</TableCell>
									<TableCell>
										<Badge variant="secondary">
											{POSTURE_LABEL[item.posture] ?? item.posture}
										</Badge>
									</TableCell>
									<TableCell className="text-center">
										{item.vetted ? (
											<span className="text-emerald-600">
												{t("yes", "Yes")}
											</span>
										) : (
											<span className="text-amber-600">{t("no", "No")}</span>
										)}
									</TableCell>
									<TableCell className="text-sm text-muted-foreground">
										{item.note ?? "—"}
									</TableCell>
									<TableCell className="text-right">
										<Button
											variant="ghost"
											size="sm"
											onClick={() => editEntry(item)}
										>
											{item.needsRating ? "Rate" : "Edit"}
										</Button>
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</CardContent>
			</Card>
		</div>
	);
}
