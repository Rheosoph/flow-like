"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { AppWindow, Check, Loader2, Search } from "lucide-react";
import Link from "next/link";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { apiErrorMessage, isPurchaseRequiredError } from "../../lib/api-error";
import { IAppVisibility } from "../../lib/schema/app/app";
import type { AddAppPackageRequest } from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Skeleton } from "../ui/skeleton";
import { errorHttpStatus } from "./package-workspace/workspace-model";

type RowState =
	| { status: "adding" }
	| { status: "added" }
	| { status: "present" }
	| { status: "failed"; message: string };

interface ProjectOption {
	id: string;
	name: string;
}

function projectPackagesHref(appId: string): string {
	return `/library/config/packages?id=${encodeURIComponent(appId)}`;
}

/**
 * Pins a registry package into one of the viewer's projects. This is how the web,
 * which has no device install store, puts a package to use. The hub decides who
 * may: a project admin who holds the package.
 */
export function AddToProjectDialog({
	packageId,
	packageName,
	version,
	open,
	onOpenChange,
}: Readonly<{
	packageId: string;
	packageName: string;
	version: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		open,
	);
	const apps = useInvoke(backend.appState.getApps, backend.appState, [], open);
	const [query, setQuery] = useState("");
	const [rows, setRows] = useState<Readonly<Record<string, RowState>>>({});

	const projects = useMemo<ProjectOption[]>(
		() =>
			(apps.data ?? [])
				.filter(
					([app]) => !app.bot_id && app.visibility !== IAppVisibility.Offline,
				)
				.map(([app, metadata]) => ({
					id: app.id,
					name: metadata?.name?.trim() || "",
				}))
				.sort((a, b) => a.name.localeCompare(b.name)),
		[apps.data],
	);
	const needle = query.trim().toLocaleLowerCase();
	const visible = needle
		? projects.filter((project) =>
				project.name.toLocaleLowerCase().includes(needle),
			)
		: projects;

	const setRow = useCallback(
		(appId: string, state: RowState) =>
			setRows((current) => ({ ...current, [appId]: state })),
		[],
	);

	const add = useCallback(
		async (project: ProjectOption) => {
			if (!profile.data) return;
			setRow(project.id, { status: "adding" });
			const request: AddAppPackageRequest = {
				packageId,
				version,
				autoUpdate: true,
			};
			try {
				await backend.apiState.post(
					profile.data,
					`apps/${project.id}/packages`,
					request,
				);
				setRow(project.id, { status: "added" });
				void queryClient.invalidateQueries({
					queryKey: ["app", project.id, "packages"],
				});
				toast.success(
					t("addToProjectDone", "{{package}} added to {{project}}", {
						package: packageName,
						project: project.name || t("untitledProject", "Untitled project"),
					}),
				);
			} catch (error) {
				const status = errorHttpStatus(error);
				if (status === 409) {
					setRow(project.id, { status: "present" });
					return;
				}
				setRow(project.id, {
					status: "failed",
					message:
						status === 403
							? t(
									"addToProjectAdminsOnly",
									"Only admins of this project can add packages.",
								)
							: isPurchaseRequiredError(error)
								? t(
										"addToProjectGetFirst",
										"Get this package first, then add it.",
									)
								: apiErrorMessage(
										error,
										t("addToProjectFailed", "Couldn't add the package."),
									),
				});
			}
		},
		[
			backend.apiState,
			packageId,
			packageName,
			profile.data,
			queryClient,
			setRow,
			t,
			version,
		],
	);

	const handleOpenChange = (next: boolean) => {
		if (!next) {
			setQuery("");
			setRows({});
		}
		onOpenChange(next);
	};

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="flex max-h-[85dvh] flex-col gap-4 sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t("addToProjectTitle", "Add {{package}} to a project", {
							package: packageName,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"addToProjectDescription",
							"The project pins v{{version}} and its members can use the package's nodes and widgets. Updates arrive automatically.",
							{ version },
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="relative">
					<Search
						className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
						aria-hidden
					/>
					<Input
						type="search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder={t("addToProjectSearch", "Search your projects…")}
						aria-label={t("addToProjectSearch", "Search your projects…")}
						className="pl-9"
					/>
				</div>

				{apps.isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-12 w-full" />
						<Skeleton className="h-12 w-full" />
						<Skeleton className="h-12 w-full" />
					</div>
				) : apps.isError ? (
					<div className="flex items-center justify-between gap-3 rounded-lg border border-critical-line bg-critical-bg px-3 py-2 text-sm text-critical">
						{t("addToProjectLoadFailed", "Couldn't load your projects.")}
						<Button size="sm" variant="outline" onClick={() => apps.refetch()}>
							{t("retry", "Retry")}
						</Button>
					</div>
				) : projects.length === 0 ? (
					<p className="py-6 text-center text-sm text-muted-foreground">
						{t(
							"addToProjectNoProjects",
							"You have no projects yet. Create one in your library first.",
						)}
					</p>
				) : (
					<ul className="min-h-0 flex-1 divide-y divide-border overflow-y-auto rounded-lg border">
						{visible.map((project) => (
							<ProjectRow
								key={project.id}
								project={project}
								state={rows[project.id]}
								onAdd={() => void add(project)}
							/>
						))}
						{visible.length === 0 && (
							<li className="px-3 py-4 text-sm text-muted-foreground">
								{t("addToProjectNoMatch", "No project matches the search.")}
							</li>
						)}
					</ul>
				)}
			</DialogContent>
		</Dialog>
	);
}

function ProjectRow({
	project,
	state,
	onAdd,
}: Readonly<{
	project: ProjectOption;
	state?: RowState;
	onAdd: () => void;
}>) {
	const { t } = useTranslation("store");
	const name = project.name || t("untitledProject", "Untitled project");
	const done = state?.status === "added" || state?.status === "present";

	return (
		<li className="flex items-center gap-3 px-3 py-2.5">
			<AppWindow
				className="size-4 shrink-0 text-muted-foreground"
				aria-hidden
			/>
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm font-medium" title={name}>
					{name}
				</p>
				{state?.status === "failed" && (
					<p className="text-xs text-critical">{state.message}</p>
				)}
				{state?.status === "present" && (
					<p className="text-xs text-muted-foreground">
						{t("addToProjectAlreadyThere", "Already in this project")}
					</p>
				)}
			</div>
			{done ? (
				<Button asChild size="sm" variant="ghost" className="shrink-0 gap-1.5">
					<Link href={projectPackagesHref(project.id)}>
						<Check className="size-3.5 text-good" />
						{t("addToProjectOpen", "Open packages")}
					</Link>
				</Button>
			) : (
				<Button
					size="sm"
					variant="outline"
					className="shrink-0"
					disabled={state?.status === "adding"}
					onClick={onAdd}
					aria-label={t("addToProjectAddTo", "Add to {{project}}", {
						project: name,
					})}
				>
					{state?.status === "adding" && (
						<Loader2 className="size-3.5 animate-spin" />
					)}
					{t("add", "Add")}
				</Button>
			)}
		</li>
	);
}
