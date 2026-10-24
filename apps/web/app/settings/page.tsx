"use client";

import {
	Card,
	CardDescription,
	CardHeader,
	CardTitle,
	DeveloperModeCard,
	useDeveloperMode,
} from "@flow-like/flow-like-ui";
import { BoardRendererCard } from "@flow-like/flow-like-ui/components/settings/board-renderer-card";
import { FLOW_LIKE_LEGAL_URLS } from "@flow-like/flow-like-ui/lib/legal-links";
import { useTranslation } from "@flow-like/locales";
import {
	Activity,
	BarChart3,
	Brain,
	ChevronRight,
	ExternalLink,
	FileLock2,
	FileText,
	HardDrive,
	type LucideIcon,
	Scale,
	Scroll,
	Server,
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
					title: t("devices", "Devices"),
					description: t(
						"devicesSettingsDescription",
						"Manage your device fleet, deployments, health, and certificates",
					),
					href: "/settings/devices",
					icon: Server,
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
					title: t("settings:storageTitle", "Storage"),
					description: t(
						"settings:storageCardDescription",
						"See what this browser keeps and clean it up",
					),
					href: "/settings/storage",
					icon: HardDrive,
				},
				{
					title: t("settings:diagnosticsCardTitle", "About & diagnostics"),
					description: t(
						"settings:diagnosticsCardDescription",
						"Version, browser features and a report for support",
					),
					href: "/settings/system",
					icon: Activity,
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
		{
			label: t("legal.title", "Legal information"),
			cards: [
				{
					title: t("legal.notice", "Legal notice"),
					description: t(
						"legal.noticeDescription",
						"Who operates Flow-Like and how to reach us",
					),
					href: FLOW_LIKE_LEGAL_URLS.notice,
					icon: Scale,
					external: true,
				},
				{
					title: t("legal.privacy", "Privacy policy"),
					description: t(
						"legal.privacyDescription",
						"What data Flow-Like processes and your rights",
					),
					href: FLOW_LIKE_LEGAL_URLS.privacy,
					icon: FileLock2,
					external: true,
				},
				{
					title: t("legal.eula", "EULA"),
					description: t(
						"legal.eulaDescription",
						"The terms for using Flow-Like",
					),
					href: FLOW_LIKE_LEGAL_URLS.eula,
					icon: FileText,
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
					cards: section.cards.filter((card) => developerMode || !card.devOnly),
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
				<BoardRendererCard />
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
