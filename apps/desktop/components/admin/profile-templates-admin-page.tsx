"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IProfile,
	Input,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Pencil,
	Plus,
	RefreshCw,
	Search,
	Trash2,
	UserRound,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

type ProfileTemplate = IProfile & { id: string };

export function ProfileTemplatesAdminPage({
	manageMode,
}: Readonly<{ manageMode: boolean }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const router = useRouter();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [searchTerm, setSearchTerm] = useState("");
	const [deletingId, setDeletingId] = useState<string | null>(null);
	const debouncedSearch = useDebounce(searchTerm, 200);

	const templates = useQuery<ProfileTemplate[]>({
		queryKey: ["info", "profiles"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ProfileTemplate[]>(
				profile.data,
				"info/profiles",
			);
		},
		enabled: !!profile.data,
	});

	const filteredTemplates = useMemo(() => {
		const allTemplates = templates.data ?? [];
		const query = debouncedSearch.trim().toLowerCase();
		if (!query) return allTemplates;

		return allTemplates.filter((template) => {
			const searchableValues = [
				template.id,
				template.name,
				template.description,
				template.hub,
				...(template.tags ?? []),
				...(template.interests ?? []),
				...(template.hubs ?? []),
			]
				.filter(Boolean)
				.join(" ")
				.toLowerCase();

			return searchableValues.includes(query);
		});
	}, [templates.data, debouncedSearch]);

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["info", "profiles"] });
	}, [queryClient]);

	const handleDelete = useCallback(
		async (template: ProfileTemplate) => {
			if (!profile.data) {
				toast.error("Profile not loaded");
				return;
			}

			const confirmed = window.confirm(
				t("deleteProfileTemplateVal", 'Delete profile template "{{val}}"?', {
					val: template.name || template.id,
				}),
			);
			if (!confirmed) return;

			setDeletingId(template.id);
			try {
				await backend.apiState.del(
					profile.data,
					`admin/profiles/${template.id}`,
				);
				toast.success("Profile template deleted");
				queryClient.invalidateQueries({ queryKey: ["info", "profiles"] });
			} catch (error) {
				const message =
					error instanceof Error
						? error.message
						: t("unknownError", "Unknown error");
				toast.error(`Failed to delete template: ${message}`);
			} finally {
				setDeletingId(null);
			}
		},
		[backend.apiState, profile.data, queryClient],
	);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div className="flex items-center justify-between gap-4">
						<div>
							<h1 className="text-3xl font-bold">
								{t("profileTemplates", "Profile Templates")}
							</h1>
							<p className="text-muted-foreground">
								{t(
									"browseAndManageReusableProfileTemplatesForTheDesktopApp",
									"Browse and manage reusable profile templates for the desktop app.",
								)}
							</p>
						</div>
						<div className="flex items-center gap-2">
							<Button variant="outline" size="sm" onClick={handleRefresh}>
								<RefreshCw className="mr-2 h-4 w-4" />
								{t("refresh", "Refresh")}
							</Button>
							{manageMode && (
								<Button
									size="sm"
									onClick={() => router.push("/admin/profiles/add")}
								>
									<Plus className="mr-2 h-4 w-4" />
									{t("newTemplate", "New Template")}
								</Button>
							)}
						</div>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>{t("templates", "Templates")}</CardTitle>
							<CardDescription>
								{t(
									"searchByTemplateNameIdHubTagOrInterest",
									"Search by template name, id, hub, tag, or interest.",
								)}
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="relative max-w-md">
								<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
								<Input
									className="pl-10"
									placeholder={t(
										"searchProfileTemplates",
										"Search profile templates...",
									)}
									value={searchTerm}
									onChange={(event) => setSearchTerm(event.target.value)}
								/>
							</div>

							{templates.isLoading ? (
								<div className="space-y-2">
									{Array.from({ length: 6 }).map((_, index) => (
										<Skeleton
											key={`profile-template-skeleton-${index}`}
											className="h-12 w-full"
										/>
									))}
								</div>
							) : (
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>Name</TableHead>
											<TableHead>{t("templateId", "Template ID")}</TableHead>
											<TableHead>{t("bits", "Bits")}</TableHead>
											<TableHead>{t("tags", "Tags")}</TableHead>
											<TableHead>{t("updated", "Updated")}</TableHead>
											{manageMode && (
												<TableHead className="text-right">
													{t("actions", "Actions")}
												</TableHead>
											)}
										</TableRow>
									</TableHeader>
									<TableBody>
										{filteredTemplates.length === 0 ? (
											<TableRow>
												<TableCell
													colSpan={manageMode ? 6 : 5}
													className="py-10 text-center text-muted-foreground"
												>
													<UserRound className="mx-auto mb-3 h-5 w-5" />
													{t(
														"noProfileTemplatesFound",
														"No profile templates found.",
													)}
												</TableCell>
											</TableRow>
										) : (
											filteredTemplates.map((template) => (
												<TableRow key={template.id}>
													<TableCell>
														<div className="space-y-1">
															<div className="font-medium">
																{template.name || "Untitled template"}
															</div>
															{template.description && (
																<p className="max-w-md truncate text-xs text-muted-foreground">
																	{template.description}
																</p>
															)}
														</div>
													</TableCell>
													<TableCell className="font-mono text-xs">
														{template.id}
													</TableCell>
													<TableCell>{template.bits?.length ?? 0}</TableCell>
													<TableCell>
														<div className="flex flex-wrap gap-1">
															{(template.tags ?? []).slice(0, 3).map((tag) => (
																<Badge
																	key={`${template.id}-${tag}`}
																	variant="secondary"
																>
																	{tag}
																</Badge>
															))}
															{(template.tags?.length ?? 0) > 3 && (
																<Badge variant="outline">
																	+{(template.tags?.length ?? 0) - 3}
																</Badge>
															)}
														</div>
													</TableCell>
													<TableCell className="text-sm text-muted-foreground">
														{new Date(template.updated).toLocaleString()}
													</TableCell>
													{manageMode && (
														<TableCell className="text-right">
															<div className="flex justify-end gap-2">
																<Button
																	variant="outline"
																	size="sm"
																	onClick={() =>
																		router.push(
																			`/admin/profiles/add?id=${encodeURIComponent(template.id)}`,
																		)
																	}
																>
																	<Pencil className="mr-2 h-3 w-3" />
																	{t("edit", "Edit")}
																</Button>
																<Button
																	variant="destructive"
																	size="sm"
																	disabled={deletingId === template.id}
																	onClick={() => handleDelete(template)}
																>
																	<Trash2 className="mr-2 h-3 w-3" />
																	{t("delete", "Delete")}
																</Button>
															</div>
														</TableCell>
													)}
												</TableRow>
											))
										)}
									</TableBody>
								</Table>
							)}
						</CardContent>
					</Card>
				</div>
			</div>
		</main>
	);
}
