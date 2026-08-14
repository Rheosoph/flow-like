"use client";
import { useTranslation } from "@flow-like/locales";
import {
	Building,
	Calendar,
	CopyIcon,
	Edit2,
	ExternalLink,
	FileText,
	Globe,
	Lock,
	Shield,
	Star,
	Tag,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks";
import { formatRelativeTime, useSetQueryParams } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import { FlowPreview } from "../../flow";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
	LoadingScreen,
	TextEditor,
} from "../../ui";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Separator } from "../../ui/separator";
import { Textarea } from "../../ui/textarea";

const QualityMetric = ({
	label,
	score,
	icon: Icon,
}: { label: string; score: number; icon: any }) => {
	const getScoreColor = (score: number) => {
		if (score <= 3) return "bg-green-500";
		if (score <= 6) return "bg-yellow-500";
		return "bg-red-500";
	};

	const getScoreLabel = (score: number) => {
		if (score <= 3) return "Excellent";
		if (score <= 6) return "Good";
		return "Needs Attention";
	};

	return (
		<div className="flex items-center justify-between p-3 rounded-lg bg-muted/50">
			<div className="flex items-center gap-2">
				<Icon className="h-4 w-4 text-muted-foreground" />
				<span className="text-sm font-medium">{label}</span>
			</div>
			<div className="flex items-center gap-3">
				<div className="w-16 h-2 bg-background rounded-full overflow-hidden">
					<div
						className={`h-full ${getScoreColor(score)}`}
						style={{ width: `${Math.max(10, (10 - score) * 10)}%` }}
					/>
				</div>
				<Badge
					variant="outline"
					className="text-xs min-w-[70px] justify-center"
				>
					{getScoreLabel(score)}
				</Badge>
			</div>
		</div>
	);
};

const TagsInput = ({
	tags,
	onChange,
}: { tags: string[]; onChange: (tags: string[]) => void }) => {
	const { t } = useTranslation("settings");
	const [inputValue, setInputValue] = useState("");

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter" && inputValue.trim()) {
				e.preventDefault();
				const newTag = inputValue.trim();
				if (!tags.includes(newTag)) {
					onChange([...tags, newTag]);
				}
				setInputValue("");
			}
		},
		[inputValue, tags, onChange],
	);

	const removeTag = useCallback(
		(tagToRemove: string) => {
			onChange(tags.filter((tag) => tag !== tagToRemove));
		},
		[tags, onChange],
	);

	return (
		<div className="space-y-2">
			<div className="flex flex-wrap gap-2">
				{tags.map((tag) => (
					<Badge key={tag} variant="secondary" className="gap-1">
						{tag}
						<Button
							variant="ghost"
							size="sm"
							className="h-4 w-4 p-0 hover:bg-destructive hover:text-destructive-foreground"
							onClick={() => removeTag(tag)}
						>
							<X className="h-3 w-3" />
						</Button>
					</Badge>
				))}
			</div>
			<Input
				value={inputValue}
				onChange={(e) => setInputValue(e.target.value)}
				onKeyDown={handleKeyDown}
				placeholder={t('typeTagAndPressEnter', 'Type tag and press Enter...')}
				className="w-full"
			/>
		</div>
	);
};

const MetadataField = ({
	label,
	value,
	isEditing,
	placeholder,
	onChange,
	type = "textarea",
}: {
	label: string;
	value?: string | null;
	isEditing: boolean;
	placeholder: string;
	onChange?: (value: string) => void;
	type?: "textarea" | "input";
}) => {
	if (!isEditing && !value) return null;

	return (
		<>
			<Separator />
			<div className="space-y-2">
				<Label className="font-medium">{label}</Label>
				{isEditing ? (
					type === "textarea" ? (
						<Textarea
							value={value || ""}
							onChange={(e) => onChange?.(e.target.value)}
							placeholder={placeholder}
							className="resize-none"
						/>
					) : (
						<Input
							value={value || ""}
							onChange={(e) => onChange?.(e.target.value)}
							placeholder={placeholder}
						/>
					)
				) : (
					<p className="text-sm text-muted-foreground">{value}</p>
				)}
			</div>
		</>
	);
};

interface EditState {
	name: string;
	description: string;
	long_description: string;
	tags: string[];
	age_rating?: number;
	use_case: string;
	website: string;
	docs_url: string;
	support_url: string;
	release_notes: string;
	selectedWorkflow?: string;
	selectedVersion?: number[];
}

export function TemplatePreview({
	appId,
	templateId,
	canEdit,
}: Readonly<{ appId: string; templateId: string; canEdit: boolean }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [isEditing, setIsEditing] = useState(false);
	const [editState, setEditState] = useState<EditState | null>(null);
	const setQueryParams = useSetQueryParams();

	const template = useInvoke(
		backend.templateState.getTemplate,
		backend.templateState,
		[appId, templateId],
	);
	const metadata = useInvoke(
		backend.templateState.getTemplateMeta,
		backend.templateState,
		[appId, templateId],
	);
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[appId],
		isEditing,
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, editState?.selectedWorkflow ?? ""],
		isEditing && !!editState?.selectedWorkflow,
	);

	const currentData = useMemo(
		() =>
			editState || {
				name: metadata.data?.name || "",
				description: metadata.data?.description || "",
				long_description: metadata.data?.long_description || "",
				tags: metadata.data?.tags || [],
				age_rating: metadata.data?.age_rating || undefined,
				use_case: metadata.data?.use_case || "",
				website: metadata.data?.website || "",
				docs_url: metadata.data?.docs_url || "",
				support_url: metadata.data?.support_url || "",
				release_notes: metadata.data?.release_notes || "",
			},
		[editState, metadata.data],
	);

	const handleEdit = useCallback(() => {
		if (!isEditing && metadata.data) {
			setEditState({
				name: metadata.data.name,
				description: metadata.data.description,
				long_description: metadata.data.long_description || "",
				tags: metadata.data.tags || [],
				age_rating: metadata.data.age_rating || undefined,
				use_case: metadata.data.use_case || "",
				website: metadata.data.website || "",
				docs_url: metadata.data.docs_url || "",
				support_url: metadata.data.support_url || "",
				release_notes: metadata.data.release_notes || "",
			});
		}
		setIsEditing(!isEditing);
	}, [isEditing, metadata.data]);

	const handleSave = useCallback(async () => {
		if (!editState || !metadata.data) return;

		await backend.templateState.pushTemplateMeta(appId, templateId, {
			...metadata.data,
			age_rating: editState.age_rating,
			name: editState.name,
			description: editState.description,
			long_description: editState.long_description,
			tags: editState.tags,
			use_case: editState.use_case,
			website: editState.website,
			docs_url: editState.docs_url,
			support_url: editState.support_url,
			release_notes: editState.release_notes,
		});

		if (editState.selectedWorkflow) {
			await backend.templateState.upsertTemplate(
				appId,
				editState.selectedWorkflow,
				templateId,
				editState.selectedVersion as any,
			);
		}

		await metadata.refetch();
		await template.refetch();
		setIsEditing(false);
		setEditState(null);
	}, [editState, metadata.data, appId, templateId, backend.templateState]);

	const updateEditState = useCallback((updates: Partial<EditState>) => {
		setEditState((prev) => (prev ? { ...prev, ...updates } : null));
	}, []);

	if (!template.data || !metadata.data) return <LoadingScreen />;

	const nodeScores = Object.values(template.data.nodes)
		.map((node) => node.scores)
		.filter(Boolean);

	const avgScores =
		nodeScores.length > 0
			? {
					privacy:
						nodeScores.reduce((sum, score) => sum + (score?.privacy ?? 0), 0) /
						nodeScores.length,
					security:
						nodeScores.reduce((sum, score) => sum + (score?.security ?? 0), 0) /
						nodeScores.length,
					performance:
						nodeScores.reduce(
							(sum, score) => sum + (score?.performance ?? 0),
							0,
						) / nodeScores.length,
					governance:
						nodeScores.reduce(
							(sum, score) => sum + (score?.governance ?? 0),
							0,
						) / nodeScores.length,
				}
			: null;

	return (
		<div className="flex h-full bg-background">
			{!isEditing && (
				<div className="w-2/5 border-r bg-muted/20">
					<FlowPreview
						nodes={Object.values(template.data?.nodes)}
						comments={template.data?.comments}
						layers={template.data?.layers}
						variables={template.data?.variables}
					/>
				</div>
			)}

			<div className="flex-1 overflow-y-auto">
				<div className="max-w-4xl mx-auto p-8 space-y-8">
					<div className="flex items-start justify-between">
						<div className="flex-1 space-y-4 w-full">
							<Breadcrumb>
								<BreadcrumbList>
									<BreadcrumbItem>
										<BreadcrumbLink asChild>
											<button
												onClick={() => {
													setQueryParams("templateId", undefined);
												}}
											>
												{t('templates', 'Templates')}
											</button>
										</BreadcrumbLink>
									</BreadcrumbItem>
									<BreadcrumbSeparator />
									<BreadcrumbItem>
										<BreadcrumbPage>{metadata.data?.name}</BreadcrumbPage>
									</BreadcrumbItem>
								</BreadcrumbList>
							</Breadcrumb>
							<div className="flex items-center gap-4">
								<div className="rounded-lg p-4 bg-primary/10 flex items-center justify-center">
									<CopyIcon className="h-6 w-6 text-primary" />
								</div>
								<div className="space-y-2">
									{isEditing ? (
										<Input
											value={currentData.name}
											onChange={(e) =>
												updateEditState({ name: e.target.value })
											}
											className="text-3xl font-bold h-auto py-2 border-0 px-0 focus-visible:ring-0 w-full"
										/>
									) : (
										<h1 className="text-3xl font-bold">{currentData.name}</h1>
									)}
									<div className="flex items-center gap-3">
										<Badge variant="secondary" className="gap-1">
											<Tag className="h-3 w-3" />v
											{template.data.version.join(".")}
										</Badge>
										<Badge variant="outline">{template.data.stage}</Badge>
										{currentData.age_rating && (
											<Badge variant="outline" className="gap-1">
												<Star className="h-3 w-3" />{t('ageAge_rating', 'Age {{age_rating}}+', { age_rating: currentData.age_rating })}</Badge>
										)}
										{isEditing && (
											<Input
												type="number"
												value={currentData.age_rating || ""}
												onChange={(e) =>
													updateEditState({
														age_rating: e.target.value
															? Number.parseInt(e.target.value)
															: undefined,
													})
												}
												placeholder={t('ageRating', 'Age rating')}
												className="w-40"
												min="0"
												max="100"
											/>
										)}
									</div>
								</div>
							</div>
						</div>

						{canEdit && (
							<div className="flex gap-2">
								<Button
									variant={isEditing ? "default" : "outline"}
									onClick={isEditing ? handleSave : handleEdit}
								>
									<Edit2 className="h-4 w-4 mr-2" />
									{isEditing ? "Save" : "Edit"}
								</Button>
								{isEditing && (
									<Button
										variant="outline"
										onClick={() => {
											setIsEditing(false);
											setEditState(null);
										}}
									>
										{t('cancel', 'Cancel')}
									</Button>
								)}
							</div>
						)}
					</div>

					{isEditing ? (
						<Textarea
							value={currentData.description}
							onChange={(e) => updateEditState({ description: e.target.value })}
							placeholder={t('templateDescription', 'Template description...')}
							className="resize-none w-full"
							rows={2}
						/>
					) : (
						<p className="text-lg text-muted-foreground">
							{currentData.description}
						</p>
					)}

					{(currentData.tags.length > 0 || isEditing) && (
						<div className="space-y-2">
							{isEditing && <Label className="font-medium">{t('tags', 'Tags')}</Label>}
							{isEditing ? (
								<TagsInput
									tags={currentData.tags}
									onChange={(tags) => updateEditState({ tags })}
								/>
							) : (
								<div className="flex flex-wrap gap-2">
									{currentData.tags.map((tag) => (
										<Badge key={tag} variant="secondary">
											{tag}
										</Badge>
									))}
								</div>
							)}
						</div>
					)}

					{isEditing && (
						<Card>
							<CardHeader>
								<CardTitle>{t('updateTemplateSource', 'Update Template Source')}</CardTitle>
								<CardDescription>
									{`Import a new workflow version into this template`}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="space-y-2">
									<Label>{t('sourceWorkflow', 'Source Workflow')}</Label>
									<Select
										value={editState?.selectedWorkflow || ""}
										onValueChange={(value) =>
											updateEditState({
												selectedWorkflow: value,
												selectedVersion: undefined,
											})
										}
									>
										<SelectTrigger>
											<SelectValue placeholder={t('selectAWorkflow', 'Select a workflow')} />
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

								{editState?.selectedWorkflow && (
									<div className="space-y-2">
										<Label>{t('workflowVersion', 'Workflow Version')}</Label>
										<Select
											value={editState?.selectedVersion?.join(".") || ""}
											onValueChange={(value) =>
												updateEditState({
													selectedVersion:
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
														versions.isFetching
															? t('loadingVersions', 'Loading versions...')
															: "Latest"
													}
												/>
											</SelectTrigger>
											<SelectContent>
												{versions.isFetching ? (
													<div className="flex items-center justify-center py-4">
														<div className="animate-spin rounded-full h-4 w-4 border-2 border-primary border-t-transparent" />
														<span className="ml-2 text-sm text-muted-foreground">
															{t('loadingVersions', 'Loading versions...')}
														</span>
													</div>
												) : (
													<>
														<SelectItem key={""} value={"none"}>
															{t('latest', 'Latest')}
														</SelectItem>
														{versions.data?.map((version) => (
															<SelectItem
																key={version.join(".")}
																value={version.join(".")}
															>
																v{version.join(".")}
															</SelectItem>
														))}
													</>
												)}
											</SelectContent>
										</Select>
									</div>
								)}
							</CardContent>
						</Card>
					)}

					{avgScores && (
						<Card>
							<CardHeader>
								<CardTitle className="flex items-center gap-2">
									<Shield className="h-5 w-5" />
									{t('qualityMetrics', 'Quality Metrics')}
								</CardTitle>
								<CardDescription>
									{t('averageScoresAcrossAllNodes', 'Average scores across all nodes')}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-3">
								<QualityMetric
									label="Privacy"
									score={avgScores.privacy}
									icon={Lock}
								/>
								<QualityMetric
									label="Security"
									score={avgScores.security}
									icon={Shield}
								/>
								<QualityMetric
									label="Performance"
									score={avgScores.performance}
									icon={Zap}
								/>
								<QualityMetric
									label="Governance"
									score={avgScores.governance}
									icon={Building}
								/>
							</CardContent>
						</Card>
					)}

					<div className="relative">
						<TextEditor
							appId={appId}
							onChange={(content) => {
								updateEditState({ long_description: content });
							}}
							editable={isEditing}
							isMarkdown={true}
							initialContent={
								metadata.data.long_description ||
								t('noDetailedDescriptionAvailable', '*No detailed description available.*')
							}
						/>
					</div>

					<Card>
						<CardHeader>
							<CardTitle>{t('templateInformation', 'Template Information')}</CardTitle>
						</CardHeader>
						<CardContent className="space-y-6">
							<div className="grid grid-cols-2 gap-6">
								<div className="space-y-1">
									<div className="flex items-center gap-2 text-sm text-muted-foreground">
										<Calendar className="h-4 w-4" />
										{t('created', 'Created')}
									</div>
									<div className="font-medium">
										{formatRelativeTime(metadata.data.created_at)}
									</div>
								</div>
								<div className="space-y-1">
									<div className="flex items-center gap-2 text-sm text-muted-foreground">
										<Calendar className="h-4 w-4" />
										{t('updated', 'Updated')}
									</div>
									<div className="font-medium">
										{formatRelativeTime(metadata.data.updated_at)}
									</div>
								</div>
							</div>

							<MetadataField
								label={t('useCase', 'Use Case')}
								value={currentData.use_case}
								isEditing={isEditing}
								placeholder={t('describeThePrimaryUseCaseForThisTemplate', 'Describe the primary use case for this template...')}
								onChange={(value) => updateEditState({ use_case: value })}
							/>

							{(isEditing ||
								currentData.website ||
								currentData.docs_url ||
								currentData.support_url) && (
								<>
									<Separator />
									<div className="space-y-3">
										<Label className="font-medium">{t('links', 'Links')}</Label>
										{isEditing ? (
											<div className="space-y-3">
												<div className="space-y-1">
													<Label className="text-sm text-muted-foreground">
														{t('website', 'Website')}
													</Label>
													<Input
														value={currentData.website}
														onChange={(e) =>
															updateEditState({ website: e.target.value })
														}
														placeholder="https://example.com"
													/>
												</div>
												<div className="space-y-1">
													<Label className="text-sm text-muted-foreground">
														{t('documentation', 'Documentation')}
													</Label>
													<Input
														value={currentData.docs_url}
														onChange={(e) =>
															updateEditState({ docs_url: e.target.value })
														}
														placeholder="https://docs.example.com"
													/>
												</div>
												<div className="space-y-1">
													<Label className="text-sm text-muted-foreground">
														{t('support', 'Support')}
													</Label>
													<Input
														value={currentData.support_url}
														onChange={(e) =>
															updateEditState({ support_url: e.target.value })
														}
														placeholder="https://support.example.com"
													/>
												</div>
											</div>
										) : (
											<div className="flex flex-col gap-2">
												{currentData.website && (
													<a
														href={currentData.website}
														target="_blank"
														rel="noopener noreferrer"
														className="flex items-center gap-2 text-sm text-blue-600 hover:text-blue-800"
													>
														<Globe className="h-4 w-4" />
														{t('website', 'Website')}
														<ExternalLink className="h-3 w-3" />
													</a>
												)}
												{currentData.docs_url && (
													<a
														href={currentData.docs_url}
														target="_blank"
														rel="noopener noreferrer"
														className="flex items-center gap-2 text-sm text-blue-600 hover:text-blue-800"
													>
														<FileText className="h-4 w-4" />
														{t('documentation', 'Documentation')}
														<ExternalLink className="h-3 w-3" />
													</a>
												)}
												{currentData.support_url && (
													<a
														href={currentData.support_url}
														target="_blank"
														rel="noopener noreferrer"
														className="flex items-center gap-2 text-sm text-blue-600 hover:text-blue-800"
													>
														<Shield className="h-4 w-4" />
														{t('support', 'Support')}
														<ExternalLink className="h-3 w-3" />
													</a>
												)}
											</div>
										)}
									</div>
								</>
							)}

							<MetadataField
								label={t('releaseNotes', 'Release Notes')}
								value={currentData.release_notes}
								isEditing={isEditing}
								placeholder={`What's new in this version...`}
								onChange={(value) => updateEditState({ release_notes: value })}
							/>
						</CardContent>
					</Card>
				</div>
			</div>
		</div>
	);
}
