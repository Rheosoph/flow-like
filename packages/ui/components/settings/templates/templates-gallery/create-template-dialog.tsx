"use client";

import { useTranslation } from "@flow-like/locales";
import { LoaderCircleIcon } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import { asArray } from "../../../../lib/response-shape";
import { IVersionType } from "../../../../lib/schema/flow/version-type";
import { nowSystemTime } from "../../../../lib/time/now";
import { cn } from "../../../../lib/utils";
import { useBackend } from "../../../../state/backend-state";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../../ui/radio-group";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Skeleton } from "../../../ui/skeleton";
import { Textarea } from "../../../ui/textarea";

const LATEST = "latest";

interface CreateTemplateDialogProps {
	readonly appId: string;
	readonly open: boolean;
	readonly onOpenChange: (open: boolean) => void;
	readonly onCreated: (templateId: string) => Promise<void> | void;
}

export function CreateTemplateDialog({
	appId,
	open,
	onOpenChange,
	onCreated,
}: CreateTemplateDialogProps) {
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[calc(100dvh-3rem)] overflow-y-auto sm:max-w-xl">
				{open ? (
					<CreateTemplateForm
						appId={appId}
						onDone={() => onOpenChange(false)}
						onCreated={onCreated}
					/>
				) : null}
			</DialogContent>
		</Dialog>
	);
}

function CreateTemplateForm({
	appId,
	onDone,
	onCreated,
}: Readonly<{
	appId: string;
	onDone: () => void;
	onCreated: CreateTemplateDialogProps["onCreated"];
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const [flowId, setFlowId] = useState("");
	const [version, setVersion] = useState(LATEST);
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [saving, setSaving] = useState(false);
	const ids = {
		version: useId(),
		name: useId(),
		description: useId(),
		flows: useId(),
	};

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		appId !== "",
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, flowId],
		flowId !== "",
	);
	const flows = asArray(boards.data);
	const flow = flows.find((board) => board.id === flowId);
	const fallbackName = flow
		? version === LATEST
			? flow.name
			: `${flow.name} v${version}`
		: "";

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (!flow || saving) return;
		setSaving(true);
		try {
			const [templateId] = await backend.templateState.upsertTemplate(
				appId,
				flow.id,
				undefined,
				version === LATEST
					? undefined
					: (version.split(".").map(Number) as [number, number, number]),
				IVersionType.Patch,
			);
			const now = nowSystemTime();
			await backend.templateState.pushTemplateMeta(appId, templateId, {
				name: name.trim() || fallbackName,
				description: description.trim(),
				tags: [],
				long_description: "",
				created_at: now,
				updated_at: now,
				preview_media: [],
			});
			await onCreated(templateId);
			toast.success(
				t("templateCreatedNamed", "Template “{{name}}” created", {
					name: name.trim() || fallbackName,
				}),
			);
			onDone();
		} catch (error) {
			console.error("Failed to create template:", error);
			toast.error(
				t(
					"templateCreateFailed",
					"Could not create the template: {{message}}",
					{
						message: error instanceof Error ? error.message : String(error),
					},
				),
			);
		} finally {
			setSaving(false);
		}
	};

	return (
		<form onSubmit={submit} className="flex flex-col gap-5">
			<DialogHeader>
				<DialogTitle>{t("createTemplate", "Create Template")}</DialogTitle>
				<DialogDescription>
					{t(
						"templateCreateDescription",
						"Snapshot a flow at one of its versions.",
					)}
				</DialogDescription>
			</DialogHeader>

			<fieldset className="flex min-w-0 flex-col gap-2">
				<legend id={ids.flows} className="mb-2 text-sm font-medium">
					{t("sourceWorkflow", "Source Workflow")}
				</legend>
				{boards.isLoading ? (
					<div aria-hidden className="grid gap-2 sm:grid-cols-2">
						<Skeleton className="h-14" />
						<Skeleton className="h-14" />
					</div>
				) : flows.length === 0 ? (
					<p className="rounded-lg border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
						{t(
							"templateNoFlowsToSnapshot",
							"This app has no flows yet. Create a flow first, then save it as a template.",
						)}
					</p>
				) : (
					<RadioGroup
						aria-labelledby={ids.flows}
						value={flowId}
						onValueChange={(value) => {
							setFlowId(value);
							setVersion(LATEST);
						}}
						className="grid max-h-64 gap-2 overflow-y-auto p-0.5 sm:grid-cols-2"
					>
						{flows.map((board) => {
							const itemId = `${ids.flows}-${board.id}`;
							const selected = board.id === flowId;
							return (
								<Label
									key={board.id}
									htmlFor={itemId}
									className={cn(
										"flex min-w-0 cursor-pointer items-center gap-3 rounded-lg border px-3 py-2.5 font-normal transition-colors",
										selected
											? "border-primary bg-primary/5"
											: "border-border/70 hover:bg-muted/40",
									)}
								>
									<RadioGroupItem id={itemId} value={board.id} />
									<span className="flex min-w-0 flex-col">
										<span className="truncate text-sm font-medium">
											{board.name}
										</span>
										<span className="text-xs text-muted-foreground">
											{t(
												"templateFlowSummary",
												"v{{version}} · {{count}} nodes",
												{
													version: (board.version ?? [0, 0, 0]).join("."),
													count: board.nodeCount ?? 0,
													defaultValue_one: "v{{version}} · {{count}} node",
												},
											)}
										</span>
									</span>
								</Label>
							);
						})}
					</RadioGroup>
				)}
			</fieldset>

			<div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
				<div className="flex flex-col gap-2">
					<Label htmlFor={ids.version}>{t("version", "Version")}</Label>
					<Select
						value={version}
						onValueChange={setVersion}
						disabled={!flow || versions.isFetching}
					>
						<SelectTrigger id={ids.version} className="w-full">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value={LATEST}>{t("latest", "Latest")}</SelectItem>
							{asArray(versions.data).map((entry) => {
								const label = entry.join(".");
								return (
									<SelectItem key={label} value={label}>
										v{label}
									</SelectItem>
								);
							})}
						</SelectContent>
					</Select>
				</div>
				<div className="flex flex-col gap-2">
					<Label htmlFor={ids.name}>{t("templateFieldName", "Name")}</Label>
					<Input
						id={ids.name}
						value={name}
						onChange={(event) => setName(event.target.value)}
						placeholder={
							fallbackName || t("enterTemplateName", "Enter template name")
						}
					/>
				</div>
			</div>

			<div className="flex flex-col gap-2">
				<Label htmlFor={ids.description}>
					{t("description", "Description")}{" "}
					<span className="font-normal text-muted-foreground">
						{t("optionalParenthesized", "(optional)")}
					</span>
				</Label>
				<Textarea
					id={ids.description}
					value={description}
					onChange={(event) => setDescription(event.target.value)}
					placeholder={t(
						"templateDescriptionPlaceholder",
						"What is this snapshot for?",
					)}
					className="min-h-20 resize-none"
				/>
			</div>

			<DialogFooter>
				<Button type="button" variant="outline" onClick={onDone}>
					{t("cancel", "Cancel")}
				</Button>
				<Button type="submit" disabled={!flow || saving}>
					{saving ? (
						<LoaderCircleIcon aria-hidden className="size-4 animate-spin" />
					) : null}
					{t("createTemplate", "Create Template")}
				</Button>
			</DialogFooter>
		</form>
	);
}
