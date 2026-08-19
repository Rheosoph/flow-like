"use client";

import {
	Card,
	CardDescription,
	CardHeader,
	CardTitle,
	DeveloperModeCard,
	useDeveloperMode,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import {
	BarChart3,
	Brain,
	ChevronRight,
	Cpu,
	ExternalLink,
	type LucideIcon,
	Package,
	Scroll,
	ShieldCheck,
	User,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";

interface SettingsCard {
	title: string;
	description: string;
	href: string;
	icon: LucideIcon;
	external?: boolean;
	devOnly?: boolean;
}

interface SettingsSection {
	label: string;
	cards: SettingsCard[];
}

/** Labels are built per render so a language switch relabels the menu. */
function buildSettingsSections(
	t: (key: string, defaultValue: string) => string,
): SettingsSection[] {
	return [
		{
			label: t("personalization", "Personalization"),
			cards: [
				{
					title: t("profile", "Profile"),
					description: t(
						"nameAvatarInterestsAndTheme",
						"Name, avatar, interests, and theme",
					),
					href: "/settings/profiles",
					icon: User,
				},
			],
		},
		{
			label: t("aiAmpModels", "AI & Models"),
			cards: [
				{
					title: t("aiModels", "AI Models"),
					description: t(
						"browseDownloadAndManageLlmModels",
						"Browse, download, and manage LLM models",
					),
					href: "/settings/ai",
					icon: Brain,
				},
			],
		},
		{
			label: t("extensionsAmpIntegrations", "Extensions & Integrations"),
			cards: [
				{
					title: t("registry", "Registry"),
					description: t(
						"installedPackagesAndExploreTheMarketplace",
						"Installed packages and explore the marketplace",
					),
					href: "/settings/registry",
					icon: Package,
					devOnly: true,
				},
				{
					title: t("sinksAmpTriggers", "Sinks & Triggers"),
					description: t(
						"manageActiveEventTriggersLikeWebhooksCronJobsAndMore",
						"Manage active event triggers like webhooks, cron jobs, and more",
					),
					href: "/settings/sinks",
					icon: Zap,
					devOnly: true,
				},
			],
		},
		{
			label: t("system", "System"),
			cards: [
				{
					title: t("systemInfo", "System Info"),
					description: t("cpuRamAndVramDetails", "CPU, RAM, and VRAM details"),
					href: "/settings/system",
					icon: Cpu,
				},
				{
					title: t("boardStatistics", "Board Statistics"),
					description: t(
						"nodeUsageCategoryDistributionAndBoardAnalytics",
						"Node usage, category distribution, and board analytics",
					),
					href: "/settings/statistics",
					icon: BarChart3,
					devOnly: true,
				},
				{
					title: t("privacyAmpTelemetry", "Privacy & Telemetry"),
					description: t(
						"controlAnonymousUsageTelemetryAndYourInstallId",
						"Control anonymous usage telemetry and your install id",
					),
					href: "/settings/privacy",
					icon: ShieldCheck,
				},
				{
					title: t("thirdpartyLicenses", "Third-Party Licenses"),
					description: t(
						"sbomAndOpensourceLibrariesUsedInFlowlike",
						"SBOM and open-source libraries used in Flow-Like",
					),
					href: "https://flow-like.com/thirdparty",
					icon: Scroll,
					external: true,
				},
			],
		},
	];
}

function SettingsCardItem({ card }: Readonly<{ card: SettingsCard }>) {
	const Icon = card.icon;
	const content = (
		<Card className="transition-colors hover:bg-accent/50">
			<CardHeader className="flex flex-row items-center gap-4 space-y-0">
				<div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
					<Icon className="h-5 w-5" />
				</div>
				<div className="flex-1 min-w-0">
					<CardTitle className="text-base">{card.title}</CardTitle>
					<CardDescription className="text-sm line-clamp-1">
						{card.description}
					</CardDescription>
				</div>
				{card.external ? (
					<ExternalLink className="h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
				) : (
					<ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
				)}
			</CardHeader>
		</Card>
	);

	if (card.external) {
		return (
			<a href={card.href} target="_blank" rel="noreferrer" className="group">
				{content}
			</a>
		);
	}

	return (
		<Link href={card.href} className="group">
			{content}
		</Link>
	);
}

export default function SettingsPage() {
	const { t } = useTranslation("common");
	const { developerMode } = useDeveloperMode();

	const sections = useMemo(
		() =>
			buildSettingsSections(t)
				.map((section) => ({
					...section,
					cards: developerMode
						? section.cards
						: section.cards.filter((card) => !card.devOnly),
				}))
				.filter((section) => section.cards.length > 0),
		[developerMode, t],
	);

	return (
		<div className="h-full flex flex-col max-h-full overflow-auto min-h-0">
			<div className="container mx-auto px-2 pb-4 flex flex-col gap-8">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">
						{t("settings", "Settings")}
					</h1>
					<p className="text-muted-foreground">
						{t(
							"manageYourPreferencesModelsAndIntegrations",
							"Manage your preferences, models, and integrations",
						)}
					</p>
				</div>
				{sections.map((section) => (
					<div key={section.label} className="flex flex-col gap-3">
						<h2 className="text-sm font-medium text-muted-foreground uppercase tracking-wider">
							{section.label}
						</h2>
						<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
							{section.cards.map((card) => (
								<SettingsCardItem key={card.href} card={card} />
							))}
						</div>
					</div>
				))}
				<div className="flex flex-col gap-3">
					<h2 className="text-sm font-medium text-muted-foreground uppercase tracking-wider">
						{t("developer", "Developer")}
					</h2>
					<DeveloperModeCard />
				</div>
			</div>
		</div>
	);
}
