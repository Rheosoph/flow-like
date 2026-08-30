"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Copy, Plus, Star, Trash2, X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { cn } from "../../../lib/utils";
import type { IBackendRole } from "../../../state/backend-state/types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import {
	ACCESS_LADDERS,
	type AccessLadder,
	type Elevation,
	TONE_STEP_CLASS,
	TOTAL_PERMISSION_COUNT,
	applyElevation,
	applyLevel,
	describeAccess,
	effectiveLevel,
	effectivePermissionCount,
	elevationOf,
	isWritePermission,
	joinClauses,
	levelOf,
	writePermissionCount,
} from "./access-ladders";
import { getPermissionEntry } from "./permission-groups";

const ELEVATION_OPTIONS: {
	value: Elevation;
	label: string;
	hint: string;
}[] = [
	{
		value: "standard",
		label: "Standard",
		hint: "Access is exactly what the levels below say.",
	},
	{
		value: "admin",
		label: "Administrator",
		hint: "Grants every permission except ownership.",
	},
	{
		value: "owner",
		label: "Owner",
		hint: "One per app. Transferred, not assigned.",
	},
];

interface RoleEditorProps {
	role: IBackendRole;
	memberCount?: number;
	isDefault: boolean;
	/** Attributes already used by other roles, offered for reuse. */
	knownAttributes: string[];
	onChange: (next: IBackendRole) => void;
	onDuplicate: () => void;
	onDelete: () => void;
	onSetDefault: () => void;
}

export function RoleEditor({
	role,
	memberCount,
	isDefault,
	knownAttributes,
	onChange,
	onDuplicate,
	onDelete,
	onSetDefault,
}: Readonly<RoleEditorProps>) {
	const { t } = useTranslation("settings");
	const permissions = useMemo(
		() => new RolePermissions(BigInt(role.permissions)),
		[role.permissions],
	);
	const elevation = elevationOf(permissions);
	const isOwner = elevation === "owner";
	const isElevated = elevation !== "standard";

	const setPermissions = useCallback(
		(next: RolePermissions) =>
			onChange({ ...role, permissions: next.toBigInt() }),
		[onChange, role],
	);

	return (
		<div className="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_20rem] border-t">
			<div className="flex flex-col gap-4 p-4 min-w-0">
				<div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
					<div className="flex flex-col gap-1.5">
						<Label htmlFor={`role-name-${role.id}`} className="text-xs">
							Name
						</Label>
						<Input
							id={`role-name-${role.id}`}
							value={role.name}
							readOnly={isOwner}
							placeholder={t("egReviewer", "e.g. Reviewer")}
							onChange={(event) =>
								onChange({ ...role, name: event.target.value })
							}
						/>
					</div>
					<div className="flex flex-col gap-1.5">
						<Label htmlFor={`role-desc-${role.id}`} className="text-xs">
							{t("description", "Description")}
						</Label>
						<Input
							id={`role-desc-${role.id}`}
							value={role.description}
							placeholder={t("whoIsThisFor", "Who is this for?")}
							onChange={(event) =>
								onChange({ ...role, description: event.target.value })
							}
						/>
					</div>
				</div>

				<ElevationPicker
					elevation={elevation}
					disabled={isOwner}
					onSelect={(next) => setPermissions(applyElevation(permissions, next))}
				/>

				<div className="flex flex-col">
					{ACCESS_LADDERS.map((ladder) => (
						<LadderControl
							key={ladder.id}
							ladder={ladder}
							permissions={permissions}
							locked={isElevated}
							elevationLabel={isOwner ? "Owner" : "Administrator"}
							onChange={setPermissions}
						/>
					))}
				</div>

				<AttributesField
					attributes={role.attributes ?? []}
					knownAttributes={knownAttributes}
					onChange={(attributes) => onChange({ ...role, attributes })}
				/>
			</div>

			<AccessPreview
				role={role}
				permissions={permissions}
				memberCount={memberCount}
				isDefault={isDefault}
				isOwner={isOwner}
				onDuplicate={onDuplicate}
				onDelete={onDelete}
				onSetDefault={onSetDefault}
			/>
		</div>
	);
}

function ElevationPicker({
	elevation,
	disabled,
	onSelect,
}: Readonly<{
	elevation: Elevation;
	disabled: boolean;
	onSelect: (next: Elevation) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex flex-col gap-1.5">
			<Label className="text-xs">{t("elevation", "Elevation")}</Label>
			<div className="flex flex-wrap gap-2">
				{ELEVATION_OPTIONS.map((option) => {
					const active = option.value === elevation;
					const unavailable = disabled || option.value === "owner";
					return (
						<button
							key={option.value}
							type="button"
							aria-pressed={active}
							disabled={unavailable}
							onClick={() => onSelect(option.value)}
							className={cn(
								"flex-1 min-w-[10rem] rounded-lg border bg-muted/40 px-3 py-2 text-left transition-colors",
								active && "border-primary bg-primary/10",
								unavailable
									? "opacity-50 cursor-not-allowed"
									: "hover:bg-muted/70",
							)}
						>
							<p
								className={cn(
									"text-[13px] font-semibold",
									active && "text-primary",
								)}
							>
								{option.label}
							</p>
							<p className="text-xs text-muted-foreground leading-snug">
								{option.hint}
							</p>
						</button>
					);
				})}
			</div>
		</div>
	);
}

function LadderControl({
	ladder,
	permissions,
	locked,
	elevationLabel,
	onChange,
}: Readonly<{
	ladder: AccessLadder;
	permissions: RolePermissions;
	locked: boolean;
	elevationLabel: string;
	onChange: (next: RolePermissions) => void;
}>) {
	const { t } = useTranslation("settings");
	const [showAdvanced, setShowAdvanced] = useState(false);
	const exact = levelOf(permissions, ladder);
	const shown = effectiveLevel(permissions, ladder);
	const isCustom = exact < 0 && !locked;
	const advancedOpen = showAdvanced || isCustom;
	const level = shown >= 0 ? ladder.levels[shown] : undefined;
	const LadderIcon = ladder.icon;
	const raw = ladder.levels[ladder.levels.length - 1].permissions;

	const togglePermission = (permission: RolePermissions) =>
		onChange(
			permissions.contains(permission)
				? permissions.remove(permission)
				: permissions.insert(permission),
		);

	return (
		<div className="py-3 border-b last:border-b-0">
			<div className="flex items-center gap-2 mb-2">
				<LadderIcon className="h-4 w-4 text-muted-foreground shrink-0" />
				<span className="text-[13px] font-semibold flex-1 min-w-0 truncate">
					{ladder.label}
				</span>
				{isCustom && (
					<span className="text-[10px] font-bold uppercase tracking-wide rounded px-1.5 py-0.5 bg-primary/15 text-primary">
						{t("custom", "Custom")}
					</span>
				)}
				<button
					type="button"
					onClick={() => setShowAdvanced((open) => !open)}
					className="text-xs text-muted-foreground hover:text-foreground"
				>
					{advancedOpen ? "Hide" : "Advanced"} · {raw.length} permissions
				</button>
			</div>

			<div className="flex rounded-lg border overflow-hidden bg-muted/40">
				{ladder.levels.map((entry, index) => (
					<button
						key={entry.name}
						type="button"
						aria-pressed={shown === index}
						disabled={locked}
						onClick={() => onChange(applyLevel(permissions, ladder, index))}
						className={cn(
							"flex-1 px-2 py-1.5 text-xs font-medium border-r last:border-r-0 transition-colors",
							shown === index
								? TONE_STEP_CLASS[entry.tone]
								: "text-muted-foreground",
							locked
								? "opacity-50 cursor-not-allowed"
								: shown !== index && "hover:bg-muted",
						)}
					>
						{entry.name}
					</button>
				))}
			</div>

			<p className="text-xs text-muted-foreground mt-1.5">
				{locked ? (
					<>
						{t("grantedThrough", "Granted through")}{" "}
						<strong>{elevationLabel}</strong>.
					</>
				) : isCustom ? (
					t(
						"aMixThatDoesntMatchALevelTheExactPermissionsAreBelow",
						"A mix that doesn't match a level — the exact permissions are below.",
					)
				) : level?.can ? (
					<>
						{t("membersCan", "Members can")}{" "}
						<strong className="text-foreground">{level.can}</strong>.
					</>
				) : (
					<>
						<Trans i18nKey="membersStrongClassnametextforegroundcannotstrong">
							Members <strong className="text-foreground">cannot</strong>
						</Trans>{" "}
						{level?.cannot}.
					</>
				)}
			</p>

			{advancedOpen && (
				<div className="mt-2 rounded-lg border bg-muted/30 p-1.5 flex flex-col">
					{raw.map((permission) => {
						const entry = getPermissionEntry(permission);
						const active = locked || permissions.contains(permission);
						return (
							<button
								key={entry?.label ?? permission.toString()}
								type="button"
								disabled={locked}
								onClick={() => togglePermission(permission)}
								className="grid grid-cols-[auto_minmax(0,1fr)] gap-2.5 items-center rounded-md px-1.5 py-1.5 text-left hover:bg-muted/60 disabled:hover:bg-transparent"
							>
								<Switch
									checked={active}
									disabled={locked}
									tabIndex={-1}
									aria-hidden
									className="scale-75 pointer-events-none"
								/>
								<span className="min-w-0">
									<span className="flex items-center gap-1.5">
										<span className="text-[13px] leading-tight">
											{entry?.label}
										</span>
										{isWritePermission(permission) && (
											<span className="text-[9px] font-bold uppercase tracking-wider rounded px-1 py-px bg-amber-500/15 text-amber-600 dark:text-amber-400">
												write
											</span>
										)}
									</span>
									<span className="block text-xs text-muted-foreground leading-tight">
										{entry?.description}
									</span>
								</span>
							</button>
						);
					})}
				</div>
			)}
		</div>
	);
}

function AttributesField({
	attributes,
	knownAttributes,
	onChange,
}: Readonly<{
	attributes: string[];
	knownAttributes: string[];
	onChange: (next: string[]) => void;
}>) {
	const { t } = useTranslation("settings");
	const [draft, setDraft] = useState("");
	const suggestions = knownAttributes.filter(
		(attribute) => !attributes.includes(attribute),
	);

	const add = (value: string) => {
		const trimmed = value.trim();
		if (!trimmed || attributes.includes(trimmed)) return;
		onChange([...attributes, trimmed]);
		setDraft("");
	};

	return (
		<div className="flex flex-col gap-2 pt-3 border-t">
			<Label className="text-xs">{t("attributes", "Attributes")}</Label>
			<p className="text-xs text-muted-foreground">
				{t(
					"tagsThatPolicyRulesAndMemberFiltersMatchOnTheyGrantNothingOnTheirOwn",
					"Tags that policy rules and member filters match on. They grant nothing on their own.",
				)}
			</p>
			<div className="flex gap-2">
				<Input
					value={draft}
					placeholder={t("egRegioneu", "e.g. region:eu")}
					className="flex-1 font-mono text-xs"
					onChange={(event) => setDraft(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						add(draft);
					}}
				/>
				<Button
					variant="outline"
					size="sm"
					disabled={!draft.trim()}
					onClick={() => add(draft)}
				>
					{t("add", "Add")}
				</Button>
			</div>
			{attributes.length > 0 && (
				<div className="flex flex-wrap gap-1.5">
					{attributes.map((attribute) => (
						<span
							key={attribute}
							className="inline-flex items-center gap-1 rounded-md border bg-muted/50 pl-2 pr-1 py-0.5 font-mono text-xs"
						>
							{attribute}
							<button
								type="button"
								aria-label={t("removeAttribute", "Remove {{attribute}}", {
									attribute,
								})}
								className="text-muted-foreground hover:text-destructive"
								onClick={() =>
									onChange(attributes.filter((item) => item !== attribute))
								}
							>
								<X className="h-3 w-3" />
							</button>
						</span>
					))}
				</div>
			)}
			{suggestions.length > 0 && (
				<div className="flex flex-wrap items-center gap-1.5">
					<span className="text-xs text-muted-foreground">
						{t("alreadyInUse", "Already in use:")}
					</span>
					{suggestions.map((attribute) => (
						<button
							key={attribute}
							type="button"
							onClick={() => add(attribute)}
							className="inline-flex items-center gap-1 rounded-md border border-dashed px-2 py-0.5 font-mono text-xs text-muted-foreground hover:border-solid hover:border-primary hover:text-primary"
						>
							<Plus className="h-2.5 w-2.5" />
							{attribute}
						</button>
					))}
				</div>
			)}
		</div>
	);
}

function AccessPreview({
	role,
	permissions,
	memberCount,
	isDefault,
	isOwner,
	onDuplicate,
	onDelete,
	onSetDefault,
}: Readonly<{
	role: IBackendRole;
	permissions: RolePermissions;
	memberCount?: number;
	isDefault: boolean;
	isOwner: boolean;
	onDuplicate: () => void;
	onDelete: () => void;
	onSetDefault: () => void;
}>) {
	const { t } = useTranslation("settings");
	const elevation = elevationOf(permissions);
	const name = role.name.trim() || "This role";
	const { can, cannot } = useMemo(
		() => describeAccess(permissions),
		[permissions],
	);
	const writes = writePermissionCount(permissions);

	let summary: string;
	let caveat: string;
	if (elevation === "owner") {
		summary = t(
			"nameCanDoEverythingIncludingTransferringOrDeletingTheApp",
			"{{name}} can do everything, including transferring or deleting the app.",
			{ name },
		);
		caveat = t(
			"ownerCannotBeReducedMoveOwnershipToChangeIt",
			"Owner cannot be reduced — move ownership to change it.",
		);
	} else if (elevation === "admin") {
		summary = t(
			"nameCanDoEverythingExceptTransferOrDeleteTheApp",
			"{{name}} can do everything except transfer or delete the app.",
			{ name },
		);
		caveat = t(
			"individualLevelsHaveNoEffectWhileAdministratorIsOn",
			"Individual levels have no effect while Administrator is on.",
		);
	} else {
		summary =
			can.length > 0
				? t("nameCanVal", "{{name}} can {{val}}.", {
						name,
						val: joinClauses(can),
					})
				: t(
						"nameHasNoAccessToAnythingInThisApp",
						"{{name}} has no access to anything in this app.",
						{ name },
					);
		caveat =
			cannot.length > 0
				? t("cannotVal", "Cannot {{val}}.", { val: joinClauses(cannot) })
				: "";
	}

	return (
		<aside className="flex flex-col gap-3 p-4 bg-muted/30 border-t lg:border-t-0 lg:border-l">
			<p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
				{t("whatThisRoleCanDo", "What this role can do")}
			</p>
			<p className="text-[15px] leading-relaxed">{summary}</p>
			{caveat && (
				<p className="text-sm text-muted-foreground leading-relaxed border-l-2 pl-3">
					{caveat}
				</p>
			)}

			<dl className="flex flex-col gap-1.5 pt-3 border-t text-sm">
				<div className="flex items-baseline justify-between gap-3">
					<dt className="text-muted-foreground">
						{t("permissions", "Permissions")}
					</dt>
					<dd className="font-mono tabular-nums">
						{effectivePermissionCount(permissions)}/{TOTAL_PERMISSION_COUNT}
					</dd>
				</div>
				<div className="flex items-baseline justify-between gap-3">
					<dt className="text-muted-foreground">
						{t("canChangeData", "Can change data")}
					</dt>
					<dd
						className={cn(
							"font-mono tabular-nums",
							writes > 5 && "text-amber-600 dark:text-amber-400",
						)}
					>
						{writes}
					</dd>
				</div>
				{memberCount !== undefined && (
					<div className="flex items-baseline justify-between gap-3">
						<dt className="text-muted-foreground">
							{t("members2", "Members")}
						</dt>
						<dd className="font-mono tabular-nums">{memberCount}</dd>
					</div>
				)}
			</dl>

			<div className="flex flex-wrap gap-2 pt-3 border-t mt-auto">
				<Button variant="outline" size="sm" onClick={onDuplicate}>
					<Copy className="h-3.5 w-3.5 mr-1.5" />
					{t("duplicate", "Duplicate")}
				</Button>
				{!isDefault && !isOwner && (
					<Button variant="outline" size="sm" onClick={onSetDefault}>
						<Star className="h-3.5 w-3.5 mr-1.5" />
						{t("makeDefault", "Make default")}
					</Button>
				)}
				{!isOwner && (
					<AlertDialog>
						<AlertDialogTrigger asChild>
							<Button
								variant="outline"
								size="sm"
								className="text-destructive hover:text-destructive"
							>
								<Trash2 className="h-3.5 w-3.5 mr-1.5" />
								{t("delete", "Delete")}
							</Button>
						</AlertDialogTrigger>
						<AlertDialogContent>
							<AlertDialogHeader>
								<AlertDialogTitle>
									{t("deleteRole", "Delete role")}
								</AlertDialogTitle>
								<AlertDialogDescription>
									{memberCount
										? t(
												"membercountValWillBeReassignedToTheDefaultRole",
												"{{memberCount}} {{val}} will be reassigned to the default role.",
												{
													memberCount,
													val: memberCount === 1 ? "member" : "members",
												},
											)
										: t(
												"membersWithThisRoleWillBeReassignedToTheDefaultRole",
												"Members with this role will be reassigned to the default role.",
											)}
								</AlertDialogDescription>
							</AlertDialogHeader>
							<AlertDialogFooter>
								<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
								<AlertDialogAction
									onClick={onDelete}
									className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
								>
									{t("delete", "Delete")}
								</AlertDialogAction>
							</AlertDialogFooter>
						</AlertDialogContent>
					</AlertDialog>
				)}
			</div>
		</aside>
	);
}
