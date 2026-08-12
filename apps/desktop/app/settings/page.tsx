"use client";

import {
	Card,
	CardDescription,
	CardHeader,
	CardTitle,
	DeveloperModeCard,
	useDeveloperMode,
} from "@flow-like/flow-like-ui";
import {
	BarChart3,
	Bell,
	Brain,
	ChevronRight,
	Cpu,
	ExternalLink,
	HardDrive,
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

const SETTINGS_SECTIONS: SettingsSection[] = [
	{
		label: "Personalization",
		cards: [
			{
				title: "Profile",
				description: "Name, avatar, interests, and theme",
				href: "/settings/profiles",
				icon: User,
			},
			{
				title: "Notifications",
				description: "Mobile push status and device registration",
				href: "/settings/notifications",
				icon: Bell,
			},
		],
	},
	{
		label: "AI & Models",
		cards: [
			{
				title: "AI Models",
				description: "Browse, download, and manage LLM models",
				href: "/settings/ai",
				icon: Brain,
			},
		],
	},
	{
		label: "Extensions & Integrations",
		cards: [
			{
				title: "Registry",
				description: "Installed packages and explore the marketplace",
				href: "/settings/registry",
				icon: Package,
				devOnly: true,
			},
			{
				title: "Sinks & Triggers",
				description:
					"Manage active event triggers like webhooks, cron jobs, and more",
				href: "/settings/sinks",
				icon: Zap,
				devOnly: true,
			},
		],
	},
	{
		label: "System",
		cards: [
			{
				title: "Local Storage",
				description: "See what Studio stores and clean up local files",
				href: "/settings/storage",
				icon: HardDrive,
			},
			{
				title: "System Info",
				description: "CPU, RAM, and VRAM details",
				href: "/settings/system",
				icon: Cpu,
			},
			{
				title: "Board Statistics",
				description: "Node usage, category distribution, and board analytics",
				href: "/settings/statistics",
				icon: BarChart3,
				devOnly: true,
			},
			{
				title: "Privacy & Telemetry",
				description: "Control anonymous usage telemetry and your install id",
				href: "/settings/privacy",
				icon: ShieldCheck,
			},
			{
				title: "Third-Party Licenses",
				description: "SBOM and open-source libraries used in Flow-Like",
				href: "https://flow-like.com/thirdparty",
				icon: Scroll,
				external: true,
			},
		],
	},
];

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
					<ExternalLink className="h-4 w-4 shrink-0 text-muted-foreground opacity-100 transition-opacity md:opacity-0 md:group-hover:opacity-100" />
				) : (
					<ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground opacity-100 transition-opacity md:opacity-0 md:group-hover:opacity-100" />
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
	const { developerMode } = useDeveloperMode();

	const sections = useMemo(
		() =>
			SETTINGS_SECTIONS.map((section) => ({
				...section,
				cards: section.cards.filter((card) => developerMode || !card.devOnly),
			})).filter((section) => section.cards.length > 0),
		[developerMode],
	);

	return (
		<div className="h-full flex flex-col max-h-full overflow-auto min-h-0">
			<div className="container mx-auto px-2 pb-4 flex flex-col gap-8">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">Settings</h1>
					<p className="text-muted-foreground">
						Manage your preferences, models, and integrations
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
						Developer
					</h2>
					<DeveloperModeCard />
				</div>
			</div>
		</div>
	);
}
