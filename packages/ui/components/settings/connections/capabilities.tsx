import { useTranslation } from "@flow-like/locales";
import { Database, Eye, FileText, Pencil, Zap } from "lucide-react";
import { RolePermissions, cn } from "../../..";

export type AccessLevel = "read" | "readwrite";

export const ACCESS_LABEL: Record<AccessLevel, string> = {
	read: "Read",
	readwrite: "Read/Write",
};

export interface ConnectionCapabilities {
	events: boolean;
	database?: AccessLevel;
	files?: AccessLevel;
}

/**
 * Derives a connection's coarse capabilities from a role's permission bits.
 * Mirrors the backend's implied semantics: `WriteFiles` implies database write
 * (and file read), and `ReadFiles` implies database read.
 */
export function deriveConnectionCapabilities(
	bits?: number | null,
): ConnectionCapabilities {
	if (!bits) return { events: false };
	const perms = new RolePermissions(BigInt(bits));
	const filesWrite = perms.contains(RolePermissions.WriteFiles);
	const filesRead = filesWrite || perms.contains(RolePermissions.ReadFiles);
	const dbWrite = filesWrite || perms.contains(RolePermissions.WriteDatabase);
	const dbRead =
		dbWrite || filesRead || perms.contains(RolePermissions.ReadDatabase);
	return {
		events: perms.contains(RolePermissions.ExecuteEvents),
		database: dbWrite ? "readwrite" : dbRead ? "read" : undefined,
		files: filesWrite ? "readwrite" : filesRead ? "read" : undefined,
	};
}

export function hasConnectionCapabilities(
	caps: ConnectionCapabilities,
): boolean {
	return caps.events || Boolean(caps.database) || Boolean(caps.files);
}

/** Read = Eye, Read/Write = Pencil — a colour-independent access cue. */
export function AccessIcon({
	access,
	className,
}: Readonly<{ access: AccessLevel; className?: string }>) {
	const Icon = access === "readwrite" ? Pencil : Eye;
	return <Icon className={className} aria-hidden />;
}

/** Compact capability chips used in list rows. */
export function CapabilityBadges({
	capabilities,
	className,
}: Readonly<{ capabilities: ConnectionCapabilities; className?: string }>) {
	const { t } = useTranslation("settings");
	if (!hasConnectionCapabilities(capabilities)) return null;
	return (
		<div className={cn("flex flex-wrap items-center gap-1", className)}>
			{capabilities.events && (
				<span className="flex items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[10px] leading-none text-muted-foreground">
					<Zap className="h-2.5 w-2.5" />
					{t("events", "Events")}
				</span>
			)}
			{capabilities.database && (
				<CapabilityChip
					icon={Database}
					label="DB"
					access={capabilities.database}
				/>
			)}
			{capabilities.files && (
				<CapabilityChip
					icon={FileText}
					label="Files"
					access={capabilities.files}
				/>
			)}
		</div>
	);
}

function CapabilityChip({
	icon: Icon,
	label,
	access,
}: Readonly<{
	icon: typeof Database;
	label: string;
	access: AccessLevel;
}>) {
	const emphasized = access === "readwrite";
	return (
		<span
			className={cn(
				"flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] leading-none",
				emphasized
					? "bg-muted font-medium text-foreground"
					: "bg-muted text-muted-foreground",
			)}
		>
			<Icon className="h-2.5 w-2.5" />
			{label}
			<AccessIcon access={access} className="h-2.5 w-2.5" />
			{ACCESS_LABEL[access]}
		</span>
	);
}
