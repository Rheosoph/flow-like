"use client";

import {
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import {
	type DeveloperSettings,
	EDITOR_OPTIONS,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
	ChevronRight,
	FlaskConical,
	Loader2,
	SlidersHorizontal,
	Workflow,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";
import { toast } from "sonner";

const SETTINGS_KEY = ["developer-settings"] as const;

function DeveloperTools() {
	const { t } = useTranslation("common");
	const tools = [
		{
			href: "/developer/flowpilot-workflows",
			label: t("workflowBehaviorBenchmarks", "Workflow behavior benchmarks"),
			icon: Workflow,
		},
		{
			href: "/developer/flowpilot-e2e",
			label: t("flowpilotAppcreationE2e", "FlowPilot app-creation E2E"),
			icon: FlaskConical,
		},
	];
	return (
		<div className="space-y-2">
			<Label>{t("developerTools", "Developer Tools")}</Label>
			<nav className="overflow-hidden rounded-lg border border-border/60">
				{tools.map((tool) => (
					<DialogClose asChild key={tool.href}>
						<Link
							href={tool.href}
							className="flex items-center gap-2.5 border-b border-border/60 px-3 py-2.5 text-sm transition-colors last:border-b-0 hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-none"
						>
							<tool.icon className="h-4 w-4 text-muted-foreground" />
							<span className="flex-1">{tool.label}</span>
							<ChevronRight className="h-4 w-4 text-muted-foreground" />
						</Link>
					</DialogClose>
				))}
			</nav>
		</div>
	);
}

function EditorPreference() {
	const { t } = useTranslation("common");
	const queryClient = useQueryClient();
	const settings = useQuery({
		queryKey: SETTINGS_KEY,
		queryFn: () => invoke<DeveloperSettings>("developer_get_settings"),
	});
	const [draft, setDraft] = useState<DeveloperSettings | null>(null);

	useEffect(() => {
		if (settings.data) setDraft(settings.data);
	}, [settings.data]);

	const save = useMutation({
		mutationFn: (devSettings: DeveloperSettings) =>
			invoke("developer_save_settings", { devSettings }),
		onSuccess: () => {
			toast.success(t("settingsSaved", "Settings saved"));
			queryClient.invalidateQueries({ queryKey: SETTINGS_KEY });
		},
		onError: (error) => toast.error(getErrorMessage(error)),
	});

	if (!draft) return <Skeleton className="h-16 w-full rounded-lg" />;

	return (
		<>
			<div className="space-y-2">
				<Label htmlFor="developer-preferred-editor">
					{t("preferredEditor", "Preferred Editor")}
				</Label>
				<Select
					value={draft.preferredEditor}
					onValueChange={(preferredEditor) =>
						setDraft({ ...draft, preferredEditor })
					}
				>
					<SelectTrigger id="developer-preferred-editor">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{EDITOR_OPTIONS.map((editor) => (
							<SelectItem key={editor.value} value={editor.value}>
								{editor.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<DeveloperTools />
			<DialogFooter>
				<DialogClose asChild>
					<Button variant="outline">{t("cancel", "Cancel")}</Button>
				</DialogClose>
				<Button onClick={() => save.mutate(draft)} disabled={save.isPending}>
					{save.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
					{t("save", "Save")}
				</Button>
			</DialogFooter>
		</>
	);
}

export function DeveloperSettingsDialog() {
	const { t } = useTranslation("common");
	const label = t("developerSettings", "Developer Settings");
	return (
		<Dialog>
			<Tooltip>
				<TooltipTrigger asChild>
					<DialogTrigger asChild>
						<Button variant="outline" size="icon" aria-label={label}>
							<SlidersHorizontal className="h-4 w-4" />
						</Button>
					</DialogTrigger>
				</TooltipTrigger>
				<TooltipContent>{label}</TooltipContent>
			</Tooltip>
			<DialogContent className="max-w-sm">
				<DialogHeader>
					<DialogTitle>{label}</DialogTitle>
					<DialogDescription>
						{t(
							"configureYourDevelopmentEnvironment",
							"Configure your development environment",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-5">
					<EditorPreference />
				</div>
			</DialogContent>
		</Dialog>
	);
}
