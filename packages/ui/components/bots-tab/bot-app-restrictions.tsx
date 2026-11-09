"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, Plus, ShieldCheck, X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useBotApps, useSetBotAppRestrictions } from "../../hooks/use-bots";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type {
	BotPermission,
	BotRestrictionPreset,
	IBotApp,
	IBotRestrictions,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { Input } from "../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Switch } from "../ui/switch";
import {
	PERMISSION_GROUPS,
	type PermissionGroupId,
	RESTRICTION_PRESETS,
	type ScopeList,
	detectPreset,
	holds,
	normalizeFolder,
	normalizeRestrictions,
	presetRestrictions,
	setFilesReadOnly,
	setFolders,
	setScopeList,
	togglePermission,
} from "./bot-restrictions-model";

export interface ScopeOption {
	id: string;
	name: string;
}

export interface ScopeOptions {
	tables: ScopeOption[];
	events: ScopeOption[];
	boards: ScopeOption[];
}

function usePermissionLabels(): Record<BotPermission, string> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			read_files: t("restrictions.permissions.read_files", "Look at files"),
			write_files: t(
				"restrictions.permissions.write_files",
				"Add and change files",
			),
			read_database: t(
				"restrictions.permissions.read_database",
				"Look at tables",
			),
			write_database: t(
				"restrictions.permissions.write_database",
				"Add and change table rows",
			),
			invoke_api: t("restrictions.permissions.invoke_api", "Use the app's API"),
			read_team: t(
				"restrictions.permissions.read_team",
				"See who is on the team",
			),
			read_boards: t("restrictions.permissions.read_boards", "Look at boards"),
			execute_boards: t(
				"restrictions.permissions.execute_boards",
				"Run boards",
			),
			write_boards: t("restrictions.permissions.write_boards", "Edit boards"),
			list_events: t(
				"restrictions.permissions.list_events",
				"See the list of events",
			),
			read_events: t("restrictions.permissions.read_events", "Look at events"),
			execute_events: t(
				"restrictions.permissions.execute_events",
				"Call events",
			),
			write_events: t("restrictions.permissions.write_events", "Edit events"),
			read_logs: t("restrictions.permissions.read_logs", "Read run logs"),
			read_config: t("restrictions.permissions.read_config", "Read settings"),
			read_templates: t(
				"restrictions.permissions.read_templates",
				"Look at templates",
			),
			write_templates: t(
				"restrictions.permissions.write_templates",
				"Edit templates",
			),
			read_courses: t(
				"restrictions.permissions.read_courses",
				"Look at courses",
			),
			write_courses: t(
				"restrictions.permissions.write_courses",
				"Edit courses",
			),
			read_widgets: t(
				"restrictions.permissions.read_widgets",
				"Look at widgets",
			),
			write_widgets: t(
				"restrictions.permissions.write_widgets",
				"Edit widgets",
			),
		}),
		[t],
	);
}

function useGroupLabels(): Record<PermissionGroupId, string> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			files: t("restrictions.groups.files", "Files"),
			tables: t("restrictions.groups.tables", "Tables"),
			boards: t("restrictions.groups.boards", "Boards"),
			events: t("restrictions.groups.events", "Events"),
			other: t("restrictions.groups.other", "Everything else"),
		}),
		[t],
	);
}

function usePresetLabels(): Record<BotRestrictionPreset | "custom", string> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			read_only: t("restrictions.presets.read_only", "Read only"),
			read_and_call_events: t(
				"restrictions.presets.read_and_call_events",
				"Read + call events",
			),
			full: t("restrictions.presets.full", "Full (your role)"),
			custom: t("restrictions.presets.custom", "Custom"),
		}),
		[t],
	);
}

function AllowList({
	label,
	options,
	value,
	onChange,
}: Readonly<{
	label: string;
	options: readonly ScopeOption[];
	value: string[] | null | undefined;
	onChange: (next: string[] | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const limited = Array.isArray(value);
	const names = useMemo(
		() => new Map(options.map((option) => [option.id, option.name])),
		[options],
	);
	const shown = [
		...options.map((option) => option.id),
		...(value ?? []).filter((id) => !names.has(id)),
	];
	return (
		<div className="flex flex-col gap-1.5">
			<label className="flex items-center gap-2 text-sm">
				<Checkbox
					checked={limited}
					onCheckedChange={(next) => onChange(next === true ? [] : null)}
					aria-label={t("restrictions.limit", "Only some {{what}}", {
						what: label.toLocaleLowerCase(),
					})}
				/>
				<span>
					{limited
						? t("restrictions.limitedTo", "Only these {{what}}", {
								what: label.toLocaleLowerCase(),
							})
						: t("restrictions.all", "All {{what}}", {
								what: label.toLocaleLowerCase(),
							})}
				</span>
			</label>
			{limited && (
				<ul className="max-h-40 divide-y divide-border overflow-y-auto rounded-md border border-border">
					{shown.map((id) => (
						<li key={id} className="flex items-center gap-2 px-2.5 py-1.5">
							<Checkbox
								checked={value?.includes(id) ?? false}
								aria-label={names.get(id) ?? id}
								onCheckedChange={(next) =>
									onChange(
										next === true
											? [...(value ?? []), id]
											: (value ?? []).filter((entry) => entry !== id),
									)
								}
							/>
							<span className="min-w-0 flex-1 truncate text-sm">
								{names.get(id) ?? id}
							</span>
						</li>
					))}
					{shown.length === 0 && (
						<li className="px-2.5 py-2 text-xs text-muted-foreground">
							{t("restrictions.nothingToPick", "Nothing here yet.")}
						</li>
					)}
				</ul>
			)}
		</div>
	);
}

function FolderList({
	value,
	onChange,
}: Readonly<{ value: string[]; onChange: (next: string[]) => void }>) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState("");
	const add = useCallback(() => {
		const folder = normalizeFolder(draft);
		if (folder && !value.includes(folder)) onChange([...value, folder]);
		setDraft("");
	}, [draft, onChange, value]);
	return (
		<div className="flex flex-col gap-1.5">
			<p className="text-sm">
				{value.length
					? t("restrictions.foldersOnly", "Only these folders")
					: t("restrictions.foldersAll", "All folders")}
			</p>
			<div className="flex flex-wrap gap-1.5">
				{value.map((folder) => (
					<span
						key={folder}
						className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-0.5 text-xs"
					>
						{folder}
						<button
							type="button"
							aria-label={t("restrictions.removeFolder", "Remove {{folder}}", {
								folder,
							})}
							onClick={() =>
								onChange(value.filter((entry) => entry !== folder))
							}
						>
							<X className="size-3" aria-hidden />
						</button>
					</span>
				))}
			</div>
			<div className="flex gap-2">
				<Input
					value={draft}
					className="h-8 text-xs"
					placeholder={t("restrictions.folderPlaceholder", "reports/2026")}
					aria-label={t("restrictions.folderAdd", "Add a folder")}
					onChange={(event) => setDraft(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						add();
					}}
				/>
				<Button
					type="button"
					size="sm"
					variant="outline"
					onClick={add}
					disabled={!normalizeFolder(draft)}
				>
					<Plus className="size-3.5" aria-hidden />
					{t("restrictions.folderAdd", "Add a folder")}
				</Button>
			</div>
		</div>
	);
}

function GroupAllowList({
	list,
	labels,
	options,
	value,
	onChange,
}: Readonly<{
	list: ScopeList;
	labels: Record<ScopeList, string>;
	options: ScopeOptions;
	value: IBotRestrictions;
	onChange: (next: IBotRestrictions) => void;
}>) {
	return (
		<AllowList
			label={labels[list]}
			options={options[list]}
			value={value.scopes[list]}
			onChange={(next) => onChange(setScopeList(value, list, next))}
		/>
	);
}

function scopeListOf(id: PermissionGroupId): ScopeList | null {
	return id === "tables" || id === "events" || id === "boards" ? id : null;
}

/** Preset picker and the advanced editor; it holds no state of its own. */
export function RestrictionsEditor({
	value,
	options,
	disabled,
	permissionsOnly = false,
	onChange,
}: Readonly<{
	value: IBotRestrictions;
	options: ScopeOptions;
	disabled?: boolean;
	permissionsOnly?: boolean;
	onChange: (next: IBotRestrictions) => void;
}>) {
	const { t } = useTranslation("bots");
	const permissionLabels = usePermissionLabels();
	const groupLabels = useGroupLabels();
	const presetLabels = usePresetLabels();
	const preset = detectPreset(value);
	const listLabels: Record<ScopeList, string> = {
		tables: t("restrictions.groups.tables", "Tables"),
		events: t("restrictions.groups.events", "Events"),
		boards: t("restrictions.groups.boards", "Boards"),
	};
	return (
		<div className="flex flex-col gap-3">
			<Select
				value={preset ?? "custom"}
				disabled={disabled}
				onValueChange={(next) => {
					if (next !== "custom")
						onChange(presetRestrictions(next as BotRestrictionPreset));
				}}
			>
				<SelectTrigger
					className="w-full"
					aria-label={t("restrictions.preset", "What the bot may do here")}
				>
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{RESTRICTION_PRESETS.map((id) => (
						<SelectItem key={id} value={id}>
							{presetLabels[id]}
						</SelectItem>
					))}
					{preset === null && (
						<SelectItem value="custom">{presetLabels.custom}</SelectItem>
					)}
				</SelectContent>
			</Select>
			<Collapsible>
				<CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
					<ChevronDown className="size-3.5" aria-hidden />
					{t("restrictions.advanced", "Advanced")}
				</CollapsibleTrigger>
				<CollapsibleContent className="mt-3 flex flex-col gap-4">
					{PERMISSION_GROUPS.map((group) => (
						<fieldset key={group.id} className="flex flex-col gap-2">
							<legend className="mb-1 text-xs text-muted-foreground">
								{groupLabels[group.id]}
							</legend>
							{group.permissions.map((id) => (
								<label
									key={id}
									className="flex items-center justify-between gap-3 text-sm"
								>
									{permissionLabels[id]}
									<Switch
										checked={holds(value, id)}
										disabled={disabled}
										aria-label={permissionLabels[id]}
										onCheckedChange={(on) =>
											onChange(togglePermission(value, id, on))
										}
									/>
								</label>
							))}
							{!permissionsOnly && group.id === "files" && (
								<>
									<label className="flex items-center justify-between gap-3 text-sm">
										{t("restrictions.readOnlyFiles", "Never change files")}
										<Switch
											checked={value.scopes.files.read_only}
											disabled={disabled}
											aria-label={t(
												"restrictions.readOnlyFiles",
												"Never change files",
											)}
											onCheckedChange={(on) =>
												onChange(setFilesReadOnly(value, on))
											}
										/>
									</label>
									<FolderList
										value={value.scopes.files.prefixes}
										onChange={(next) => onChange(setFolders(value, next))}
									/>
								</>
							)}
							{!permissionsOnly && scopeListOf(group.id) && (
								<GroupAllowList
									list={scopeListOf(group.id) as ScopeList}
									labels={listLabels}
									options={options}
									value={value}
									onChange={onChange}
								/>
							)}
						</fieldset>
					))}
				</CollapsibleContent>
			</Collapsible>
			{!permissionsOnly && (
				<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
					<ShieldCheck className="size-3.5 shrink-0" aria-hidden />
					{t(
						"restrictions.everyChat",
						"Restrictions apply in every chat, including yours",
					)}
				</p>
			)}
		</div>
	);
}

function useScopeOptions(appId: string, enabled: boolean): ScopeOptions {
	const backend = useBackend();
	const tables = useInvoke(
		backend.dbState.listTables,
		backend.dbState,
		[appId],
		enabled,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		enabled,
	);
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[appId],
		enabled,
	);
	return useMemo(
		() => ({
			tables: (tables.data ?? []).map((name) => ({ id: name, name })),
			events: (events.data ?? []).map((event) => ({
				id: event.id,
				name: event.name,
			})),
			boards: (boards.data ?? []).map((board) => ({
				id: board.id,
				name: board.name,
			})),
		}),
		[boards.data, events.data, tables.data],
	);
}

/** Uses the editor's draft callback when present; otherwise saves each change. */
export function BotAppRestrictions({
	botId,
	appId,
	restrictions,
	onChange,
}: Readonly<{
	botId: string;
	appId: string;
	restrictions?: IBotRestrictions;
	onChange?: (appId: string, restrictions: IBotRestrictions) => void;
}>) {
	const { t } = useTranslation("bots");
	const options = useScopeOptions(appId, true);
	const { mutateAsync, isPending } = useSetBotAppRestrictions();
	const value = useMemo(
		() => normalizeRestrictions(restrictions),
		[restrictions],
	);
	const save = useCallback(
		async (next: IBotRestrictions) => {
			try {
				await mutateAsync({ botId, appId, restrictions: next });
			} catch (error) {
				toast.error(
					error instanceof Error && error.message
						? error.message
						: t("restrictions.failed", "The restrictions couldn't be saved."),
				);
			}
		},
		[appId, botId, mutateAsync, t],
	);
	return (
		<RestrictionsEditor
			value={value}
			options={options}
			disabled={isPending}
			onChange={onChange ? (next) => onChange(appId, next) : save}
		/>
	);
}

export function useAppNames(): Map<string, string> {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	return useMemo(
		() =>
			new Map(
				(apps.data ?? []).map(([app, metadata]) => [
					app.id,
					metadata?.name ?? "",
				]),
			),
		[apps.data],
	);
}

/** Per attached project: what the bot may do there. */
export function BotAppAccess({
	botId,
	apps: draftApps,
	onRestrictionsChange,
}: Readonly<{
	botId: string;
	apps?: readonly IBotApp[];
	onRestrictionsChange?: (
		appId: string,
		restrictions: IBotRestrictions,
	) => void;
}>) {
	const { t } = useTranslation("bots");
	const attached = useBotApps(botId);
	const names = useAppNames();
	const apps = draftApps ?? attached.data ?? [];
	if (apps.length === 0) return null;
	return (
		<section className="flex flex-col gap-2">
			<h3 className="text-xs text-muted-foreground">
				{t("restrictions.title", "What it may do in your projects")}
			</h3>
			<ul className="flex flex-col gap-2">
				{apps.map((app) => (
					<li
						key={app.app_id}
						className="rounded-lg border border-border px-3 py-2.5"
					>
						<p className="mb-2 truncate text-sm font-medium">
							{names.get(app.app_id) ||
								t("editor.abilities.appsUntitled", "Untitled app")}
						</p>
						<BotAppRestrictions
							botId={botId}
							appId={app.app_id}
							restrictions={app.restrictions}
							onChange={onRestrictionsChange}
						/>
					</li>
				))}
			</ul>
		</section>
	);
}
