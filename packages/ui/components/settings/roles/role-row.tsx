"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, Crown, Shield, Star, User2Icon } from "lucide-react";
import { useMemo } from "react";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { cn } from "../../../lib/utils";
import type { IBackendRole } from "../../../state/backend-state/types";
import { Badge } from "../../ui/badge";
import {
	ACCESS_LADDERS,
	type AccessLadder,
	TONE_GAUGE_CLASS,
	TOTAL_PERMISSION_COUNT,
	effectiveLevel,
	effectivePermissionCount,
	elevationOf,
	writePermissionCount,
} from "./access-ladders";

/** Column headings above the gauges, aligned with every row's grid. */
export function LadderKey() {
	return (
		<div className="hidden md:grid grid-cols-[minmax(0,1fr)_repeat(7,3.25rem)_2rem] gap-x-1.5 items-end px-4 pb-1.5">
			<span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
				Role
			</span>
			{ACCESS_LADDERS.map((ladder) => {
				const LadderIcon = ladder.icon;
				return (
					<span
						key={ladder.id}
						title={ladder.label}
						className="flex flex-col items-center gap-1 text-muted-foreground"
					>
						<LadderIcon className="h-3.5 w-3.5" />
						<span className="text-[9px] font-semibold uppercase tracking-wide">
							{ladder.short}
						</span>
					</span>
				);
			})}
			<span />
		</div>
	);
}

interface RoleRowProps {
	role: IBackendRole;
	isDefault: boolean;
	isOpen: boolean;
	memberCount?: number;
	onToggle: () => void;
	children?: React.ReactNode;
}

export function RoleRow({
	role,
	isDefault,
	isOpen,
	memberCount,
	onToggle,
	children,
}: Readonly<RoleRowProps>) {
	const { t } = useTranslation("settings");
	const permissions = useMemo(
		() => new RolePermissions(BigInt(role.permissions)),
		[role.permissions],
	);
	const elevation = elevationOf(permissions);
	const RoleIcon =
		elevation === "owner" ? Crown : elevation === "admin" ? Shield : User2Icon;
	const writes = writePermissionCount(permissions);
	const attributes = role.attributes ?? [];

	const facts = [
		memberCount === undefined
			? undefined
			: `${memberCount} ${memberCount === 1 ? "member" : "members"}`,
		t('valOfTotal_permission_countPermissions', '{{val}} of {{TOTAL_PERMISSION_COUNT}} permissions', { val: effectivePermissionCount(permissions), TOTAL_PERMISSION_COUNT }),
		writes > 0 ? t('writesCanChangeData', '{{writes}} can change data', { writes }) : "read-only",
	].filter(Boolean);

	return (
		<article
			className={cn(
				"rounded-lg border bg-card overflow-hidden",
				isOpen && "border-primary/40",
			)}
		>
			<button
				type="button"
				aria-expanded={isOpen}
				onClick={onToggle}
				className="w-full grid grid-cols-[minmax(0,1fr)_2rem] md:grid-cols-[minmax(0,1fr)_repeat(7,3.25rem)_2rem] gap-x-1.5 items-center px-4 py-3 text-left hover:bg-muted/50 transition-colors"
			>
				<span className="flex items-center gap-3 min-w-0">
					<span
						className={cn(
							"w-8 h-8 rounded-lg border grid place-items-center shrink-0",
							elevation === "owner" &&
								"bg-amber-100 text-amber-700 border-amber-300/50 dark:bg-amber-900/30 dark:text-amber-400 dark:border-amber-700/40",
							elevation === "admin" &&
								"bg-primary/10 text-primary border-primary/30",
							elevation === "standard" && "bg-muted text-muted-foreground",
						)}
					>
						<RoleIcon className="h-4 w-4" />
					</span>
					<span className="min-w-0 flex flex-col">
						<span className="flex items-center gap-1.5 min-w-0">
							<span className="text-[15px] font-semibold truncate">
								{role.name || "Untitled role"}
							</span>
							{isDefault && (
								<Badge
									variant="outline"
									className="text-[10px] px-1.5 py-0 border-primary/40 text-primary shrink-0"
								>
									<Star className="h-2.5 w-2.5 mr-1" />
									{t('default', 'Default')}
								</Badge>
							)}
							{memberCount === 0 && (
								<Badge
									variant="outline"
									className="text-[10px] px-1.5 py-0 text-muted-foreground shrink-0"
								>
									{t('noMembers', 'No members')}
								</Badge>
							)}
							{attributes.slice(0, 2).map((attribute) => (
								<span
									key={attribute}
									className="hidden sm:inline rounded border px-1.5 font-mono text-[10px] text-muted-foreground shrink-0"
								>
									{attribute}
								</span>
							))}
							{attributes.length > 2 && (
								<span className="hidden sm:inline rounded border px-1.5 font-mono text-[10px] text-muted-foreground shrink-0">
									+{attributes.length - 2}
								</span>
							)}
						</span>
						<span className="text-xs text-muted-foreground truncate">
							{facts.join(" · ")}
						</span>
					</span>
				</span>

				{ACCESS_LADDERS.map((ladder) => (
					<AccessGauge
						key={ladder.id}
						ladder={ladder}
						permissions={permissions}
					/>
				))}

				<ChevronDown
					className={cn(
						"h-4 w-4 text-muted-foreground justify-self-end transition-transform",
						isOpen && "rotate-180",
					)}
				/>
			</button>
			{isOpen && children}
		</article>
	);
}

/** Bar chart of one ladder: filled steps up to the role's level. */
function AccessGauge({
	ladder,
	permissions,
}: Readonly<{ ladder: AccessLadder; permissions: RolePermissions }>) {
	const index = effectiveLevel(permissions, ladder);
	const level = index >= 0 ? ladder.levels[index] : undefined;
	const steps = ladder.levels.length - 1;

	return (
		<span
			title={`${ladder.label}: ${level?.name ?? "Custom"}`}
			className="hidden md:flex flex-col items-center gap-1"
		>
			<span className="flex items-end gap-0.5 h-4">
				{Array.from({ length: steps }, (_, step) => (
					<span
						key={`${ladder.id}-${step}`}
						style={{ height: `${6 + step * 3}px` }}
						className={cn(
							"w-[5px] rounded-sm",
							index < 0 || index > step
								? TONE_GAUGE_CLASS[level?.tone ?? "manage"]
								: "bg-muted-foreground/25",
						)}
					/>
				))}
			</span>
			<span className="font-mono text-[9px] text-muted-foreground">
				{level?.name ?? "custom"}
			</span>
		</span>
	);
}
