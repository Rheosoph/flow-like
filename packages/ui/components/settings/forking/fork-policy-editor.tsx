"use client";

import { useTranslation } from "@flow-like/locales";
import {
	DatabaseIcon,
	FileIcon,
	LayoutTemplateIcon,
	type LucideIcon,
	ShapesIcon,
	UsersIcon,
	WorkflowIcon,
} from "lucide-react";
import { useCallback } from "react";
import type {
	IForkDatabaseMode,
	IForkPolicy,
} from "../../../lib/schema/app/fork";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";

type ToggleKey = Exclude<keyof IForkPolicy, "databases">;

interface ToggleRow {
	readonly key: ToggleKey;
	readonly icon: LucideIcon;
	readonly label: string;
	readonly description: string;
}

const TOGGLES: readonly ToggleRow[] = [
	{
		key: "flows",
		icon: WorkflowIcon,
		label: "Flows",
		description: "Boards, nodes and their pinned versions.",
	},
	{
		key: "files",
		icon: FileIcon,
		label: "Files",
		description: "Everything uploaded to the app's file storage.",
	},
	{
		key: "widgets",
		icon: ShapesIcon,
		label: "Widgets",
		description: "Reusable interface components defined in this app.",
	},
	{
		key: "templates",
		icon: LayoutTemplateIcon,
		label: "Templates",
		description: "Saved board templates published from this app.",
	},
	{
		key: "roles",
		icon: UsersIcon,
		label: "Roles",
		description:
			"Custom roles and their permissions. A fork always gets its own Owner, Admin and User roles.",
	},
];

const DATABASE_MODES: readonly {
	readonly value: IForkDatabaseMode;
	readonly label: string;
	readonly description: string;
}[] = [
	{
		value: "with_data",
		label: "Tables and data",
		description: "The fork starts with a full copy of your tables and rows.",
	},
	{
		value: "schema_only",
		label: "Tables only, no data",
		description:
			"Tables are recreated empty. Indices are not copied and must be rebuilt.",
	},
	{
		value: "none",
		label: "No database",
		description:
			"The fork starts with no tables. Flows that read a table will fail until it exists.",
	},
];

export interface ForkPolicyEditorProps {
	policy: IForkPolicy;
	disabled?: boolean;
	onChange: (policy: IForkPolicy) => void;
}

/**
 * Owner-only editor for what a fork of this app contains. The person
 * forking has no say — the fork dialog only displays the result.
 *
 * Pure presentational: the parent owns persistence and toasts.
 */
export function ForkPolicyEditor({
	policy,
	disabled = false,
	onChange,
}: Readonly<ForkPolicyEditorProps>) {
	const { t } = useTranslation("settings");
	const setToggle = useCallback(
		(key: ToggleKey, next: boolean) => onChange({ ...policy, [key]: next }),
		[policy, onChange],
	);

	const setDatabases = useCallback(
		(next: IForkDatabaseMode) => onChange({ ...policy, databases: next }),
		[policy, onChange],
	);

	const activeMode =
		DATABASE_MODES.find((mode) => mode.value === policy.databases) ??
		DATABASE_MODES[0];

	return (
		<div className="space-y-3">
			<div className="space-y-1">
				<p className="text-sm font-medium">{t('whatAForkIncludes', 'What a fork includes')}</p>
				<p className="text-xs text-muted-foreground">
					{t('anyoneWhoCanForkThisAppCanAlreadyReadItsContentsThroughTheNormalAppApisTheseSettingsKeepForksCleanAndSmallTheyAreNotAnAccessControl', "Anyone who can fork this app can already read its contents through the normal app APIs. These settings keep forks clean and small — they are not an access control.")}
				</p>
			</div>

			<div className="rounded-md border divide-y">
				{TOGGLES.map(({ key, icon: Icon, label, description }) => {
					const inputId = `fork-policy-${key}`;
					return (
						<div
							key={key}
							className="flex items-center justify-between gap-4 p-3"
						>
							<div className="space-y-0.5 pr-2">
								<Label
									htmlFor={inputId}
									className="text-sm font-medium flex items-center gap-2"
								>
									<Icon className="w-3.5 h-3.5 text-muted-foreground" />
									{label}
								</Label>
								<p className="text-xs text-muted-foreground">{description}</p>
							</div>
							<Switch
								id={inputId}
								checked={policy[key]}
								disabled={disabled}
								onCheckedChange={(next) => setToggle(key, next)}
							/>
						</div>
					);
				})}

				<div className="flex items-center justify-between gap-4 p-3">
					<div className="space-y-0.5 pr-2">
						<Label
							htmlFor="fork-policy-databases"
							className="text-sm font-medium flex items-center gap-2"
						>
							<DatabaseIcon className="w-3.5 h-3.5 text-muted-foreground" />
							{t('databases', 'Databases')}
						</Label>
						<p className="text-xs text-muted-foreground">
							{activeMode.description}
						</p>
					</div>
					<Select
						value={policy.databases}
						disabled={disabled}
						onValueChange={(next) => setDatabases(next as IForkDatabaseMode)}
					>
						<SelectTrigger id="fork-policy-databases" className="w-[190px]">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{DATABASE_MODES.map((mode) => (
								<SelectItem key={mode.value} value={mode.value}>
									{mode.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
			</div>

			{!policy.flows && (
				<p className="text-xs text-amber-600 dark:text-amber-500">
					{t('withoutFlowsAForkHasNoRunnableLogicOnlyTheAppShell', 'Without flows a fork has no runnable logic — only the app shell.')}
				</p>
			)}
			{!policy.roles && (
				<p className="text-xs text-muted-foreground">
					{t('forksGetFreshOwnerAdminAndUserRolesNodesThatReferenceARoleByIdWontResolveReferencesByNameStillWork', "Forks get fresh Owner, Admin and User roles. Nodes that reference a role by ID won't resolve; references by name still work.")}
				</p>
			)}
		</div>
	);
}
