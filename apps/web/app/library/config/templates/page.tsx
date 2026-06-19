"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	type IDate,
	IVersionType,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	TemplatePreview,
	Textarea,
	formatRelativeTime,
	nowSystemTime,
	useBackend,
	useInvoke,
	useSetQueryParams,
} from "@flow-like/flow-like-ui";
import {
	Calendar,
	CopyIcon,
	Edit,
	Filter,
	MoreVertical,
	Plus,
	Search,
	Trash2,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

export default function TemplatesPage() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const templateId = searchParams.get("templateId");
	const setQueryParams = useSetQueryParams();
	const [searchTerm, setSearchTerm] = useState("");
	const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
	const [selectedWorkflow, setSelectedWorkflow] = useState("");
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[appId ?? ""],
		typeof appId === "string",
	);
	const templates = useInvoke(
		backend.templateState.getTemplates,
		backend.templateState,
		[appId ?? ""],
		typeof appId === "string",
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, selectedWorkflow],
		(selectedWorkflow ?? "") !== "" && isCreateDialogOpen,
	);
	const [newTemplate, setNewTemplate] = useState<any>({
		name: "",
		description: "",
		workflowId: "",
		workflowVersion: undefined,
	});

	const filteredTemplates = useMemo(() => {
		return (
			templates.data?.filter(
				(template) =>
					template[2]?.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
					template[2]?.description
						.toLowerCase()
						.includes(searchTerm.toLowerCase()),
			) ?? []
		);
	}, [templates.data, searchTerm]);

	const handleCreateTemplate = useCallback(async () => {
		if (!selectedWorkflow || !newTemplate.name) {
			toast.error("Please select a workflow and enter a template name");
			return;
		}

		const template = await backend.templateState.upsertTemplate(
			appId,
			selectedWorkflow,
			undefined,
			newTemplate.workflowVersion,
			IVersionType.Patch,
		);
		await backend.templateState.pushTemplateMeta(appId, template[0], {
			name: newTemplate.name,
			description: newTemplate.description,
			tags: [],
			long_description: "",
			created_at: nowSystemTime(),
			updated_at: nowSystemTime(),
			preview_media: [],
		});
		await templates.refetch();
		toast.success("Template created successfully");
		setIsCreateDialogOpen(false);
		setSelectedWorkflow("");
		setNewTemplate({
			name: "",
			description: "",
			workflowId: "",
			workflowVersion: undefined,
		});
	}, [appId, newTemplate, backend, selectedWorkflow, templates.refetch]);

	const openTemplate = useCallback(
		(templateId: string) => {
			setQueryParams("templateId", templateId);
		},
		[setQueryParams],
	);

	const handleDeleteTemplate = useCallback(
		async (templateAppId: string, templateId: string) => {
			try {
				await backend.templateState.deleteTemplate(templateAppId, templateId);
				await templates.refetch();
				toast.success("Template deleted");
			} catch (error) {
				console.error("Failed to delete template:", error);
				toast.error("Failed to delete template");
			}
		},
		[backend.templateState, templates.refetch],
	);

	if (templateId && templateId !== "")
		return (
			<TemplatePreview appId={appId} templateId={templateId} canEdit={true} />
		);

	return (
		<main className="flex-col flex flex-grow max-h-full p-6 pt-0 space-y-8 overflow-auto md:overflow-visible min-h-0">
			{/* Header Section */}
			<div className="flex items-center justify-between py-4">
				<div className="space-y-1">
					<h1 className="text-2xl font-bold">Flow Templates</h1>
					<p className="text-muted-foreground text-sm">
						Create, manage, and organize your workflow templates
					</p>
				</div>
				<Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
					<DialogTrigger asChild>
						<Button className="shadow-sm">
							<Plus className="w-4 h-4 mr-2" />
							Create Template
						</Button>
					</DialogTrigger>
					<DialogContent className="sm:max-w-md">
						<DialogHeader className="space-y-3">
							<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
								<CopyIcon className="h-6 w-6 text-primary" />
							</div>
							<DialogTitle className="text-center text-xl">
								Create New Template
							</DialogTitle>
							<DialogDescription className="text-center">
								Create a reusable template from an existing workflow
							</DialogDescription>
						</DialogHeader>

						<div className="space-y-6 py-4">
							<div className="space-y-2">
								<Label htmlFor="template-name" className="text-sm font-medium">
									Template Name
								</Label>
								<Input
									id="template-name"
									placeholder="Enter template name"
									value={newTemplate.name}
									onChange={(e) =>
										setNewTemplate({ ...newTemplate, name: e.target.value })
									}
								/>
							</div>

							<div className="space-y-2">
								<Label
									htmlFor="template-description"
									className="text-sm font-medium"
								>
									Description
								</Label>
								<Textarea
									id="template-description"
									placeholder="Describe what this template does"
									value={newTemplate.description}
									onChange={(e) =>
										setNewTemplate({
											...newTemplate,
											description: e.target.value,
										})
									}
									className="min-h-[80px] resize-none"
								/>
							</div>

							<div className="space-y-2">
								<Label
									htmlFor="workflow-select"
									className="text-sm font-medium"
								>
									Source Workflow
								</Label>
								<Select
									value={selectedWorkflow}
									onValueChange={setSelectedWorkflow}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select a workflow" />
									</SelectTrigger>
									<SelectContent>
										{boards.data?.map((workflow) => (
											<SelectItem key={workflow.id} value={workflow.id}>
												{workflow.name}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>

							{selectedWorkflow && (
								<div className="space-y-2">
									<Label
										htmlFor="version-select"
										className="text-sm font-medium"
									>
										Workflow Version
									</Label>
									<Select
										value={newTemplate.workflowVersion}
										onValueChange={(value) =>
											setNewTemplate({
												...newTemplate,
												workflowVersion:
													value === "" || value === "none"
														? undefined
														: value.split(".").map(Number),
											})
										}
										disabled={versions.isFetching}
									>
										<SelectTrigger>
											<SelectValue
												placeholder={
													versions.isFetching ? "Loading versions..." : "Latest"
												}
											/>
										</SelectTrigger>
										<SelectContent>
											{versions.isFetching ? (
												<div className="flex items-center justify-center py-4">
													<div className="animate-spin rounded-full h-4 w-4 border-2 border-primary border-t-transparent" />
													<span className="ml-2 text-sm text-muted-foreground">
														Loading versions...
													</span>
												</div>
											) : (
												<>
													{versions.data?.map((version) => (
														<SelectItem
															key={version.join(".")}
															value={version.join(".")}
														>
															v{version.join(".")}
														</SelectItem>
													))}
													<SelectItem key={""} value={"none"}>
														Latest
													</SelectItem>
												</>
											)}
										</SelectContent>
									</Select>
								</div>
							)}

							<div className="flex gap-2 pt-4">
								<Button
									onClick={async () => {
										await handleCreateTemplate();
									}}
									disabled={!newTemplate.name || !selectedWorkflow}
									className="flex-1"
								>
									Create Template
								</Button>
								<Button
									variant="outline"
									onClick={() => setIsCreateDialogOpen(false)}
								>
									Cancel
								</Button>
							</div>
						</div>
					</DialogContent>
				</Dialog>
			</div>

			{/* Search and Filter Bar */}
			<div className="flex items-center gap-4">
				<div className="relative flex-1 max-w-md">
					<Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-muted-foreground w-4 h-4" />
					<Input
						placeholder="Search templates..."
						value={searchTerm}
						onChange={(e) => setSearchTerm(e.target.value)}
						className="pl-10"
					/>
				</div>
				<Button variant="outline" size="sm">
					<Filter className="w-4 h-4 mr-2" />
					Filter
				</Button>
			</div>

			{/* Templates Grid */}
			<div className="flex-1 overflow-auto md:overflow-visible">
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
					{filteredTemplates.map(([templateAppId, templateId, meta]) => (
						<Card
							key={templateId}
							role="button"
							tabIndex={0}
							onClick={() => openTemplate(templateId)}
							onKeyDown={(event) => {
								if (event.key !== "Enter" && event.key !== " ") return;
								event.preventDefault();
								openTemplate(templateId);
							}}
							className="group hover:shadow-xl transition-all duration-300 h-full flex flex-col cursor-pointer text-left"
						>
							<CardHeader className="space-y-4">
								<div className="flex items-start justify-between">
									<div className="flex items-center gap-3">
										<div className="p-2 bg-primary/10 group-hover:bg-primary/30 rounded-lg">
											<CopyIcon className="w-5 h-5 text-primary" />
										</div>
										<div className="flex-1 min-w-0">
											<CardTitle className="text-lg font-semibold text-foreground group-hover:text-primary transition-colors truncate">
												{meta?.name}
											</CardTitle>
										</div>
									</div>
									<DropdownMenu>
										<DropdownMenuTrigger asChild>
											<Button
												variant="ghost"
												size="sm"
												className="opacity-0 group-hover:opacity-100 transition-opacity"
												onClick={(event) => event.stopPropagation()}
											>
												<MoreVertical className="w-4 h-4" />
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent
											align="end"
											onClick={(event) => event.stopPropagation()}
										>
											<DropdownMenuItem onClick={() => openTemplate(templateId)}>
												<Edit className="w-4 h-4 mr-2" />
												Edit
											</DropdownMenuItem>
											<DropdownMenuSeparator />
											<DropdownMenuItem
												className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
												onClick={(event) => {
													event.stopPropagation();
													void handleDeleteTemplate(templateAppId, templateId);
												}}
											>
												<Trash2 className="w-4 h-4 mr-2" />
												Delete
											</DropdownMenuItem>
										</DropdownMenuContent>
									</DropdownMenu>
								</div>
							</CardHeader>
							<CardContent className="space-y-4 flex-1 flex flex-col">
								<p className="text-muted-foreground text-sm leading-relaxed line-clamp-2 text-start flex-1">
									{meta?.description}
								</p>

								<div className="flex flex-wrap gap-1">
									{meta?.tags.map((tag) => (
										<Badge key={tag} variant="outline" className="text-xs">
											{tag}
										</Badge>
									))}
								</div>

								<div className="pt-4 border-t mt-auto">
									<div className="flex items-center justify-between text-xs text-muted-foreground">
										<div className="flex items-center gap-1">
											<Calendar className="w-3 h-3" />
											{meta?.created_at && (
												<span>
													{formatRelativeTime(meta?.created_at as IDate)}
												</span>
											)}
										</div>
									</div>
								</div>
							</CardContent>
						</Card>
					))}
				</div>
			</div>

			{filteredTemplates.length === 0 && (
				<div className="text-center py-12">
					<CopyIcon className="w-16 h-16 text-muted-foreground mx-auto mb-4" />
					<h3 className="text-lg font-medium text-foreground mb-2">
						No templates found
					</h3>
					<p className="text-muted-foreground mb-6">
						{searchTerm
							? "Try adjusting your search terms"
							: "Create your first template to get started"}
					</p>
					{!searchTerm && (
						<Button onClick={() => setIsCreateDialogOpen(true)}>
							<Plus className="w-4 h-4 mr-2" />
							Create Your First Template
						</Button>
					)}
				</div>
			)}
		</main>
	);
}
