"use client";

import {
	BookOpen,
	Bug,
	Cable,
	Database,
	FolderOpen,
	Heart,
	Home,
	Library,
	Moon,
	Plus,
	Search,
	Settings,
	Sun,
	Workflow,
} from "lucide-react";
import * as React from "react";
import { useEffect, useMemo } from "react";
import {
	useSpotlightKeyboard,
	useSpotlightStaticItems,
} from "../../hooks/use-spotlight";
import type {
	SpotlightGroup,
	SpotlightItem,
} from "../../state/spotlight-state";
import { useSpotlightStore } from "../../state/spotlight-state";
import { SpotlightDialog } from "./spotlight-dialog";

const SPOTLIGHT_GROUPS: SpotlightGroup[] = [
	{ id: "shortcuts", label: "Shortcuts", priority: 300 },
	{ id: "projects", label: "Projects", priority: 200 },
	{ id: "open-flows", label: "Open Flows", priority: 180 },
	{ id: "actions", label: "Actions", priority: 150 },
	{ id: "flowpilot", label: "FlowPilot & Docs", priority: 140 },
	{ id: "profiles", label: "Switch Profile", priority: 120 },
	{ id: "navigation", label: "Navigation", priority: 100 },
	{ id: "account", label: "Account", priority: 50 },
];

export interface ProjectQuickLink {
	id: string;
	name: string;
	icon?: string;
	links: {
		flows?: string;
		storage?: string;
		events?: string;
		explore?: string;
		settings?: string;
	};
}

export interface SpotlightProviderProps {
	children: React.ReactNode;
	navigate: (path: string) => void;
	projects?: ProjectQuickLink[];
	onCreateProject?: () => void;
	onToggleTheme?: (theme: "light" | "dark" | "system") => void;
	currentTheme?: string;
	onOpenDocs?: () => void;
	onReportBug?: () => void;
	additionalStaticItems?: SpotlightItem[];
	onFlowPilotMessage?: (message: string) => Promise<string>;
	onQuickCreateProject?: (
		name: string,
		isOffline: boolean,
	) => Promise<{ appId: string; boardId: string } | null>;
	className?: string;
}

export function SpotlightProvider({
	children,
	navigate,
	projects = [],
	onCreateProject,
	onToggleTheme,
	currentTheme,
	onOpenDocs,
	onReportBug,
	additionalStaticItems = [],
	onFlowPilotMessage,
	onQuickCreateProject,
	className,
}: SpotlightProviderProps) {
	useSpotlightKeyboard();

	const { registerGroup } = React.useMemo(() => {
		return { registerGroup: useSpotlightStore.getState().registerGroup };
	}, []);

	useEffect(() => {
		for (const group of SPOTLIGHT_GROUPS) {
			registerGroup(group);
		}
	}, [registerGroup]);

	const staticItems = useMemo<SpotlightItem[]>(() => {
		const items: SpotlightItem[] = [];

		items.push(
			{
				id: "nav-home",
				type: "navigation",
				label: "Home",
				description: "Go to the home page",
				icon: Home,
				group: "navigation",
				keywords: ["home", "start", "main", "dashboard"],
				action: () => navigate("/"),
				priority: 100,
			},
			{
				id: "nav-hub",
				type: "navigation",
				label: "Hub",
				description: "Explore the community hub",
				icon: Heart,
				group: "navigation",
				keywords: ["hub", "community", "explore", "discover"],
				action: () => navigate("/"),
				priority: 95,
			},
			{
				id: "nav-library",
				type: "navigation",
				label: "Library",
				description: "View your project library",
				icon: Library,
				group: "navigation",
				keywords: ["library", "projects", "apps", "my"],
				action: () => navigate("/library"),
				priority: 90,
			},
			{
				id: "nav-explore-apps",
				type: "navigation",
				label: "Explore",
				description: "Discover community apps and packages",
				icon: Search,
				group: "navigation",
				keywords: ["explore", "apps", "store", "marketplace", "discover"],
				action: () => navigate("/store/explore/apps"),
				priority: 85,
			},
			{
				id: "nav-explore-packages",
				type: "navigation",
				label: "Package Registry",
				description: "Discover and install WASM node packages",
				icon: Search,
				group: "navigation",
				keywords: ["packages", "registry", "wasm", "nodes", "install"],
				action: () => navigate("/store/packages"),
				priority: 84,
			},
			{
				id: "nav-settings",
				type: "navigation",
				label: "Settings",
				description: "Open application settings",
				icon: Settings,
				group: "navigation",
				keywords: ["settings", "preferences", "config", "configuration"],
				action: () => navigate("/settings"),
				priority: 80,
			},
			{
				id: "nav-ai-models",
				type: "navigation",
				label: "AI Models",
				description: "Manage AI models and embeddings",
				icon: Database,
				group: "navigation",
				keywords: ["ai", "models", "embeddings", "llm", "machine learning"],
				action: () => navigate("/settings/ai"),
				priority: 75,
			},
		);

		if (onCreateProject) {
			items.push({
				id: "action-create-project",
				type: "action",
				label: "Create New Project",
				description: "Start a new coding project",
				icon: Plus,
				group: "actions",
				keywords: ["create", "new", "project", "start", "coding"],
				shortcut: "⌘N",
				action: onCreateProject,
				priority: 200,
			});
		}

		if (onQuickCreateProject) {
			items.push({
				id: "action-quick-create",
				type: "action",
				label: "Quick Create Project",
				description: "Create a project instantly and jump to the editor",
				icon: Workflow,
				group: "actions",
				keywords: ["quick", "create", "instant", "fast", "new", "project"],
				action: () => useSpotlightStore.getState().setMode("quick-create"),
				priority: 195,
				keepOpen: true,
			});
		}

		if (onToggleTheme) {
			items.push({
				id: "action-theme-light",
				type: "action",
				label: "Light Mode",
				description: "Switch to light theme",
				icon: Sun,
				group: "actions",
				keywords: ["light", "theme", "bright", "day"],
				action: () => onToggleTheme("light"),
				priority: 50,
			});
			items.push({
				id: "action-theme-dark",
				type: "action",
				label: "Dark Mode",
				description: "Switch to dark theme",
				icon: Moon,
				group: "actions",
				keywords: ["dark", "theme", "night"],
				action: () => onToggleTheme("dark"),
				priority: 50,
			});
		}

		if (onOpenDocs) {
			items.push({
				id: "action-docs",
				type: "action",
				label: "Documentation",
				description: "Open the documentation",
				icon: BookOpen,
				group: "actions",
				keywords: ["docs", "documentation", "help", "guide", "manual"],
				action: onOpenDocs,
				priority: 60,
			});
		}

		if (onReportBug) {
			items.push({
				id: "action-bug-report",
				type: "action",
				label: "Report Bug",
				description: "Report an issue or bug",
				icon: Bug,
				group: "actions",
				keywords: ["bug", "report", "issue", "feedback", "problem"],
				action: onReportBug,
				priority: 55,
			});
		}

		for (const project of projects) {
			items.push({
				id: `project-${project.id}`,
				type: "project",
				label: project.name,
				description: "Open project settings",
				iconUrl: project.icon,
				icon: FolderOpen,
				group: "projects",
				priority: 150,
				keywords: [project.name.toLowerCase(), "project", "app"],
				action: () =>
					navigate(project.links.settings || project.links.flows || "/library"),
				subItems: [
					project.links.flows && {
						id: `project-${project.id}-flows`,
						type: "project" as const,
						label: "Flows",
						description: `Open flows for ${project.name}`,
						icon: Workflow,
						iconUrl: project.icon,
						group: "projects",
						priority: 149,
						keywords: ["flows", "workflow", "board"],
						action: () => navigate(project.links.flows!),
					},
					project.links.storage && {
						id: `project-${project.id}-storage`,
						type: "project" as const,
						label: "Storage",
						description: `Open storage for ${project.name}`,
						icon: FolderOpen,
						iconUrl: project.icon,
						group: "projects",
						priority: 148,
						keywords: ["storage", "files", "data"],
						action: () => navigate(project.links.storage!),
					},
					project.links.events && {
						id: `project-${project.id}-events`,
						type: "project" as const,
						label: "Events",
						description: `View events for ${project.name}`,
						icon: Cable,
						iconUrl: project.icon,
						group: "projects",
						priority: 147,
						keywords: ["events", "triggers", "webhooks"],
						action: () => navigate(project.links.events!),
					},
					project.links.explore && {
						id: `project-${project.id}-explore`,
						type: "project" as const,
						label: "Data Studio",
						description: `Open Data Studio for ${project.name}`,
						icon: Database,
						iconUrl: project.icon,
						group: "projects",
						priority: 146,
						keywords: ["data studio", "ontology", "objects", "database"],
						action: () => navigate(project.links.explore!),
					},
					project.links.settings && {
						id: `project-${project.id}-settings`,
						type: "project" as const,
						label: "Settings",
						description: `Configure ${project.name}`,
						icon: Settings,
						iconUrl: project.icon,
						group: "projects",
						priority: 145,
						keywords: ["settings", "config"],
						action: () => navigate(project.links.settings!),
					},
				].filter(Boolean) as SpotlightItem[],
			});
		}

		return [...items, ...additionalStaticItems];
	}, [
		navigate,
		projects,
		onCreateProject,
		onToggleTheme,
		onOpenDocs,
		onReportBug,
		additionalStaticItems,
	]);

	useSpotlightStaticItems(staticItems);

	return (
		<>
			{children}
			<SpotlightDialog
				className={className}
				onFlowPilotMessage={onFlowPilotMessage}
				onQuickCreateProject={onQuickCreateProject}
			/>
		</>
	);
}
