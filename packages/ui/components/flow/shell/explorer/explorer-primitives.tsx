"use client";

import { useTranslation } from "@flow-like/locales";
import { memo, useState } from "react";
import {
	type PeerUserInfo,
	colorFromSub,
} from "../../../../hooks/use-peer-users";
import type { PresenceMark } from "../../../../lib/realtime/presence-locations";
import { userInitials } from "../../../../lib/user-display";
import { cn } from "../../../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../../../ui/avatar";
import { Input } from "../../../ui/input";

/**
 * The pieces every explorer root shares. They live apart from `board-explorer`
 * because there are six roots now — files, pages, widgets, tables and two
 * storage scopes — and a row that only one of them can render is a row the
 * others reimplement slightly differently.
 */

/** Indentation in px per tree level, and the gutter a row's icon sits in. */
export const TREE_INDENT = 12;
const ROW_GUTTER = 4;

// Extra props (and ref) must reach the root div so `ContextMenuTrigger asChild`
// and dnd-kit's `setNodeRef` can attach to it.
export function TreeRow({
	depth,
	icon,
	label,
	active,
	muted,
	trailing,
	expander,
	onSelect,
	className,
	style,
	...rest
}: Readonly<{
	depth: number;
	icon: React.ReactNode;
	label: string;
	active?: boolean;
	muted?: boolean;
	trailing?: React.ReactNode;
	expander?: React.ReactNode;
	onSelect?: () => void;
}> &
	Omit<React.ComponentProps<"div">, "children">) {
	return (
		<div
			{...rest}
			className={cn(
				"group/row flex items-center gap-1 rounded-sm pr-1 text-xs",
				active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
				className,
			)}
			// Merged rather than overwritten: dnd-kit applies its drag transform through
			// `style`, so a row that clobbers it cannot be a drag source.
			style={{ paddingLeft: `${depth * TREE_INDENT + ROW_GUTTER}px`, ...style }}
		>
			<span className="flex size-4 shrink-0 items-center justify-center">
				{expander}
			</span>
			<button
				type="button"
				onClick={onSelect}
				className="flex min-w-0 flex-1 items-center gap-1 py-1 text-left"
			>
				<span
					className={cn(
						"shrink-0 [&>svg]:size-3.5",
						active
							? "text-accent-foreground"
							: muted
								? "text-muted-foreground"
								: "text-primary",
					)}
				>
					{icon}
				</span>
				<span className="truncate font-mono">{label}</span>
			</button>
			{trailing}
		</div>
	);
}

const MAX_PRESENCE_DOTS = 3;

/** Shared empty array, so a row with nobody on it keeps a stable prop identity. */
export const NO_MARKS: readonly PresenceMark[] = [];

/**
 * Who is at a place — a file, a layer, a node — as a facepile small enough to
 * sit in a tree row. Shared with the inspector so the same person looks the
 * same in both rails.
 */
export const PresenceDots = memo(function PresenceDots({
	marks,
	peerUsers,
	className,
}: Readonly<{
	marks: readonly PresenceMark[];
	peerUsers?: Map<string, PeerUserInfo>;
	className?: string;
}>) {
	const { t } = useTranslation("flow");
	if (marks.length === 0) return null;
	const shown = marks.slice(0, MAX_PRESENCE_DOTS);
	const overflow = marks.length - shown.length;
	const names = marks
		.map((mark) =>
			mark.self
				? t("you", "You")
				: (peerUsers?.get(mark.sub)?.name ?? mark.sub.slice(-8)),
		)
		.join(", ");
	return (
		<span
			className={cn("flex shrink-0 items-center -space-x-1", className)}
			aria-label={t("presenceOpenBy", {
				defaultValue: "Open by {{names}}",
				names,
			})}
		>
			{shown.map((mark) => {
				const info = peerUsers?.get(mark.sub);
				const color = info?.color ?? colorFromSub(mark.sub);
				const displayName = info?.name ?? mark.sub.slice(-8);
				const label = mark.self ? t("you", "You") : displayName;
				return (
					<Avatar
						key={mark.sub}
						className="size-4 rounded-full ring-1 ring-background"
						style={{ boxShadow: `0 0 0 1px ${color}` }}
						title={mark.sessions > 1 ? `${label} ×${mark.sessions}` : label}
						aria-hidden="true"
					>
						{info?.avatarUrl && (
							<AvatarImage
								src={info.avatarUrl}
								alt=""
								className="object-cover"
							/>
						)}
						<AvatarFallback
							className="rounded-full text-[8px] font-semibold leading-none text-white"
							style={{ background: color }}
						>
							{userInitials(displayName).charAt(0)}
						</AvatarFallback>
					</Avatar>
				);
			})}
			{overflow > 0 && (
				<span
					className="flex size-3.5 items-center justify-center rounded-full bg-muted text-[7px] font-semibold leading-none text-muted-foreground ring-1 ring-background"
					title={marks
						.slice(MAX_PRESENCE_DOTS)
						.map((mark) =>
							mark.self
								? t("you", "You")
								: (peerUsers?.get(mark.sub)?.name ?? mark.sub.slice(-8)),
						)
						.join(", ")}
				>
					+{overflow}
				</span>
			)}
		</span>
	);
});

/** A row's trailing slot with presence in front of whatever control it already had. */
export function withPresence(
	dots: React.ReactNode,
	control: React.ReactNode,
): React.ReactNode {
	if (!dots) return control;
	if (!control) return dots;
	return (
		<span className="flex shrink-0 items-center gap-1">
			{dots}
			{control}
		</span>
	);
}

export function SectionHeader({
	label,
	action,
}: Readonly<{ label: string; action?: React.ReactNode }>) {
	return (
		<div className="flex items-center gap-1 px-1 pb-0.5 pt-2">
			<h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
				{label}
			</h3>
			<span className="flex-1" />
			{action}
		</div>
	);
}

export function NameField({
	initial,
	depth,
	validate,
	onSubmit,
	onCancel,
}: Readonly<{
	initial: string;
	depth: number;
	validate: (value: string) => string | null;
	onSubmit: (name: string) => void;
	onCancel: () => void;
}>) {
	const [value, setValue] = useState(initial);
	const error = value.trim() ? validate(value) : null;
	const canSubmit = Boolean(value.trim()) && !error;

	return (
		<div
			className="flex flex-col gap-0.5 py-0.5"
			style={{ paddingLeft: `${depth * TREE_INDENT + 24}px` }}
		>
			<Input
				autoFocus
				value={value}
				aria-invalid={Boolean(error)}
				className="h-6 px-1.5 font-mono text-xs"
				onChange={(event) => setValue(event.target.value)}
				onBlur={() => canSubmit && onSubmit(value.trim())}
				onKeyDown={(event) => {
					if (event.key === "Enter" && canSubmit) onSubmit(value.trim());
					if (event.key === "Escape") onCancel();
				}}
			/>
			{error && <span className="text-[10px] text-destructive">{error}</span>}
		</div>
	);
}

/** Nothing here yet, in the muted voice every empty root uses. */
export function EmptyRow({
	label,
	depth = 0,
}: Readonly<{ label: string; depth?: number }>) {
	return (
		<p
			className="py-1 text-[11px] text-muted-foreground"
			style={{ paddingLeft: `${depth * TREE_INDENT + 24}px` }}
		>
			{label}
		</p>
	);
}
