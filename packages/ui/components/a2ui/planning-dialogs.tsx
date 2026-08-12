"use client";

// Shared detail / edit dialogs and context menus for the Calendar and Gantt
// A2UI components. The dialogs are presentation-only: the host component owns
// the item state and fires the workflow actions from the callbacks.

import { differenceInCalendarDays } from "date-fns";
import {
	AlignLeftIcon,
	ArrowUpRightIcon,
	CheckIcon,
	DiamondIcon,
	ExternalLinkIcon,
	GaugeIcon,
	Link2Icon,
	LinkIcon,
	MapPinIcon,
	PaletteIcon,
	PencilIcon,
	PlusIcon,
	TagIcon,
	Trash2Icon,
	UserIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import {
	Fragment,
	type ReactNode,
	useCallback,
	useMemo,
	useState,
} from "react";
import { useInvoke } from "../../hooks/use-invoke";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IMember, IUserLookup } from "../../state/backend-state/types";
import { isLocalUserSub } from "../../state/backend-state/user-state";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Checkbox,
	Command,
	CommandEmpty,
	CommandInput,
	CommandItem,
	CommandList,
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Slider,
	Switch,
	Textarea,
} from "../ui/index";
import { useActionContext } from "./ActionHandler";
import {
	eventEnd,
	genId,
	toDate,
	toDateInput,
	toDateTimeLocalInput,
} from "./planning-utils";
import type { CalendarEvent, GanttTask } from "./types";

// ── Color palette ───────────────────────────────────────────────────

export const PLANNING_COLORS = [
	"#3b82f6",
	"#8b5cf6",
	"#ec4899",
	"#ef4444",
	"#f59e0b",
	"#10b981",
	"#06b6d4",
	"#64748b",
] as const;

/** Soft translucent fill from an accent color that works in both themes. */
export function planningTint(color: string, percent = 22): string {
	return `color-mix(in srgb, ${color} ${percent}%, transparent)`;
}

function ColorField({
	value,
	onChange,
}: {
	value: string | undefined;
	onChange: (color: string | undefined) => void;
}) {
	return (
		<div className="flex items-center gap-1.5">
			<button
				type="button"
				onClick={() => onChange(undefined)}
				aria-label="Default color"
				className={cn(
					"flex h-6 w-6 items-center justify-center rounded-full border border-border bg-primary/20 transition-transform hover:scale-110",
					!value && "ring-2 ring-ring ring-offset-1 ring-offset-background",
				)}
			>
				{!value && <CheckIcon className="h-3 w-3 text-primary" />}
			</button>
			{PLANNING_COLORS.map((c) => (
				<button
					key={c}
					type="button"
					onClick={() => onChange(c)}
					aria-label={`Color ${c}`}
					style={{ backgroundColor: c }}
					className={cn(
						"flex h-6 w-6 items-center justify-center rounded-full transition-transform hover:scale-110",
						value === c &&
							"ring-2 ring-ring ring-offset-1 ring-offset-background",
					)}
				>
					{value === c && <CheckIcon className="h-3 w-3 text-white" />}
				</button>
			))}
			<label
				className="relative flex h-6 w-6 cursor-pointer items-center justify-center rounded-full border border-dashed border-muted-foreground/50 transition-transform hover:scale-110"
				aria-label="Custom color"
				style={
					value && !PLANNING_COLORS.includes(value as never)
						? {
								backgroundColor: value,
								borderStyle: "solid",
								borderColor: value,
							}
						: undefined
				}
			>
				<PaletteIcon
					className={cn(
						"h-3 w-3",
						value && !PLANNING_COLORS.includes(value as never)
							? "text-white"
							: "text-muted-foreground",
					)}
				/>
				<input
					type="color"
					value={value ?? "#3b82f6"}
					onChange={(e) => onChange(e.target.value)}
					className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
				/>
			</label>
		</div>
	);
}

// ── Context menu ────────────────────────────────────────────────────

export interface PlanningMenuAction {
	label: string;
	icon?: ReactNode;
	destructive?: boolean;
	disabled?: boolean;
	onSelect: () => void;
}

/**
 * Right-click menu wrapper. Pass action groups (separated by dividers); when
 * `disabled` or no actions, children render untouched.
 */
export function PlanningContextMenu({
	groups,
	disabled,
	children,
}: {
	groups: PlanningMenuAction[][];
	disabled?: boolean;
	children: ReactNode;
}) {
	const nonEmpty = groups.filter((g) => g.length > 0);
	if (disabled || nonEmpty.length === 0) return <>{children}</>;
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
			<ContextMenuContent className="w-48">
				{nonEmpty.map((group, gi) => (
					// biome-ignore lint/suspicious/noArrayIndexKey: groups are positional
					<Fragment key={gi}>
						{gi > 0 && <ContextMenuSeparator />}
						{group.map((item) => (
							<ContextMenuItem
								key={item.label}
								disabled={item.disabled}
								variant={item.destructive ? "destructive" : "default"}
								onSelect={item.onSelect}
							>
								{item.icon}
								{item.label}
							</ContextMenuItem>
						))}
					</Fragment>
				))}
			</ContextMenuContent>
		</ContextMenu>
	);
}

// ── Shared dialog bits ──────────────────────────────────────────────

export type PlanningDialogMode = "view" | "edit" | "create";

function DetailRow({
	icon,
	children,
}: {
	icon: ReactNode;
	children: ReactNode;
}) {
	return (
		<div className="flex items-start gap-3 text-sm">
			<span className="mt-0.5 shrink-0 text-muted-foreground">{icon}</span>
			<div className="min-w-0 flex-1">{children}</div>
		</div>
	);
}

function FieldRow({
	label,
	children,
	htmlFor,
}: {
	label: string;
	children: ReactNode;
	htmlFor?: string;
}) {
	return (
		<div className="space-y-1.5">
			<Label htmlFor={htmlFor} className="text-xs text-muted-foreground">
				{label}
			</Label>
			{children}
		</div>
	);
}

function DialogActions({
	mode,
	editable,
	onDelete,
	onEdit,
	onCancel,
	onSave,
}: {
	mode: PlanningDialogMode;
	editable: boolean;
	onDelete: () => void;
	onEdit: () => void;
	onCancel: () => void;
	onSave: () => void;
}) {
	if (mode === "view") {
		return (
			<DialogFooter className="gap-2 sm:justify-between">
				{editable ? (
					<Button
						variant="ghost"
						size="sm"
						onClick={onDelete}
						className="text-destructive hover:text-destructive"
					>
						<Trash2Icon className="mr-1.5 h-3.5 w-3.5" /> Delete
					</Button>
				) : (
					<span />
				)}
				<div className="flex gap-2">
					<Button variant="outline" size="sm" onClick={onCancel}>
						Close
					</Button>
					{editable && (
						<Button size="sm" onClick={onEdit}>
							<PencilIcon className="mr-1.5 h-3.5 w-3.5" /> Edit
						</Button>
					)}
				</div>
			</DialogFooter>
		);
	}
	return (
		<DialogFooter className="gap-2">
			<Button variant="outline" size="sm" onClick={onCancel}>
				Cancel
			</Button>
			<Button size="sm" onClick={onSave}>
				{mode === "create" ? "Create" : "Save"}
			</Button>
		</DialogFooter>
	);
}

function colorDot(color: string | undefined): ReactNode {
	return (
		<span
			className="inline-block h-3 w-3 shrink-0 rounded-full bg-primary"
			style={color ? { backgroundColor: color } : undefined}
		/>
	);
}

/** Accent strip along the top edge of the dialog, tinted by the item color. */
function AccentBar({ color }: { color: string | undefined }) {
	return (
		<span
			aria-hidden
			className={cn("absolute inset-x-0 top-0 h-1", !color && "bg-primary/60")}
			style={color ? { backgroundColor: color } : undefined}
		/>
	);
}

/** Any scheme prefix ("https:", "mailto:", …) marks a link as external. */
function isExternalLink(link: string): boolean {
	return /^[a-z][a-z0-9+.-]*:/i.test(link);
}

/** Friendly display text: hostname for external URLs, the path for routes. */
function linkLabel(link: string): string {
	if (isExternalLink(link)) {
		try {
			return new URL(link).hostname.replace(/^www\./, "");
		} catch {
			return link;
		}
	}
	return link;
}

const LINK_CHIP_CLASS =
	"inline-flex max-w-full items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1 text-xs font-medium transition-colors hover:bg-accent hover:text-accent-foreground";

/**
 * Resolve a relative item link to the URL to push, mirroring the frontend
 * `navigateTo` handling (ActionHandler): query params embedded in the route
 * become real query params on the /use URL; already-formed /use links and
 * appId-less contexts push the route as-is.
 */
function resolveInternalLink(link: string, appId: string | undefined): string {
	const normalized = link.startsWith("/") ? link : `/${link}`;
	if (!appId || normalized.startsWith("/use")) return normalized;
	const [path = "", query] = normalized.split(/\?(.*)/s);
	const params = new URLSearchParams();
	params.set("id", appId);
	params.set("route", path);
	if (query) {
		for (const [key, value] of new URLSearchParams(query)) {
			params.set(key, value);
		}
	}
	return `/use?${params.toString()}`;
}

/**
 * Item link, only reachable from the detail dialog. Relative paths navigate
 * inside the app (same route resolution as the Navigate To node); absolute
 * URLs open in a new tab.
 */
function LinkRow({
	link,
	onNavigate,
}: { link: string; onNavigate: () => void }) {
	const router = useRouter();
	const { appId } = useActionContext();

	if (isExternalLink(link)) {
		return (
			<DetailRow icon={<LinkIcon className="h-4 w-4" />}>
				<a
					href={link}
					target="_blank"
					rel="noopener noreferrer"
					title={link}
					className={LINK_CHIP_CLASS}
				>
					<span className="truncate">{linkLabel(link)}</span>
					<ExternalLinkIcon className="h-3 w-3 shrink-0 text-muted-foreground" />
				</a>
			</DetailRow>
		);
	}

	return (
		<DetailRow icon={<LinkIcon className="h-4 w-4" />}>
			<button
				type="button"
				title={link}
				onClick={() => {
					onNavigate();
					router.push(resolveInternalLink(link, appId));
				}}
				className={LINK_CHIP_CLASS}
			>
				<span className="truncate">{linkLabel(link)}</span>
				<ArrowUpRightIcon className="h-3 w-3 shrink-0 text-muted-foreground" />
			</button>
		</DetailRow>
	);
}

// ── Metadata (key-value pairs, e.g. ticket number) ──────────────────

interface MetaEntry {
	uid: string;
	key: string;
	text: string;
	original?: unknown;
}

function formatMetaValue(value: unknown): string {
	if (value == null) return "";
	if (typeof value === "object") return JSON.stringify(value);
	return String(value);
}

function metadataToEntries(
	metadata: Record<string, unknown> | undefined,
): MetaEntry[] {
	return Object.entries(metadata ?? {}).map(([key, value]) => ({
		uid: genId("meta"),
		key,
		text: formatMetaValue(value),
		original: value,
	}));
}

/** Untouched entries keep their original (possibly non-string) values. */
function entriesToMetadata(
	entries: MetaEntry[],
): Record<string, unknown> | undefined {
	const out: Record<string, unknown> = {};
	for (const entry of entries) {
		const key = entry.key.trim();
		if (!key) continue;
		out[key] =
			entry.original !== undefined &&
			entry.text === formatMetaValue(entry.original)
				? entry.original
				: entry.text;
	}
	return Object.keys(out).length > 0 ? out : undefined;
}

function MetadataRows({ metadata }: { metadata: Record<string, unknown> }) {
	return (
		<DetailRow icon={<TagIcon className="h-4 w-4" />}>
			<div className="grid gap-1 pt-0.5">
				{Object.entries(metadata).map(([key, value]) => (
					<div key={key} className="flex items-baseline gap-2 text-xs">
						<span className="shrink-0 font-medium text-muted-foreground">
							{key}
						</span>
						<span className="min-w-0 truncate" title={formatMetaValue(value)}>
							{formatMetaValue(value)}
						</span>
					</div>
				))}
			</div>
		</DetailRow>
	);
}

function MetadataField({
	entries,
	onChange,
}: {
	entries: MetaEntry[];
	onChange: (entries: MetaEntry[]) => void;
}) {
	const update = (uid: string, patch: Partial<MetaEntry>) =>
		onChange(entries.map((e) => (e.uid === uid ? { ...e, ...patch } : e)));
	return (
		<div className="space-y-1.5">
			{entries.map((entry) => (
				<div key={entry.uid} className="flex items-center gap-1.5">
					<Input
						value={entry.key}
						onChange={(e) => update(entry.uid, { key: e.target.value })}
						placeholder="Key"
						className="h-7 w-28 shrink-0 text-xs"
					/>
					<Input
						value={entry.text}
						onChange={(e) => update(entry.uid, { text: e.target.value })}
						placeholder="Value"
						className="h-7 flex-1 text-xs"
					/>
					<button
						type="button"
						aria-label="Remove field"
						onClick={() => onChange(entries.filter((e) => e.uid !== entry.uid))}
						className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
					>
						<XIcon className="h-3 w-3" />
					</button>
				</div>
			))}
			<Button
				type="button"
				variant="ghost"
				size="sm"
				onClick={() =>
					onChange([...entries, { uid: genId("meta"), key: "", text: "" }])
				}
				className="h-6 px-1.5 text-xs text-muted-foreground hover:text-foreground"
			>
				<PlusIcon className="mr-1 h-3 w-3" /> Add field
			</Button>
		</div>
	);
}

// ── Assignee (free text or team-member sub) ─────────────────────────

/** Sub-like tokens (no whitespace, reasonably long) are worth a lookup. */
function looksLikeSub(value: string): boolean {
	return value.length >= 12 && !/\s/.test(value);
}

/** Resolve an assignee value to a user when it looks like a sub reference. */
function useAssigneeUser(value: string | undefined): IUserLookup | undefined {
	const backend = useBackend();
	const enabled = !!value && (looksLikeSub(value) || isLocalUserSub(value));
	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		[value ?? "__noop__"],
		enabled,
	);
	return enabled ? lookup.data : undefined;
}

/**
 * Renders an assignee: user subs resolve to avatar + display name (same
 * lookup as the userProfile element); anything else renders as plain text.
 */
export function AssigneeDisplay({
	value,
	className,
}: {
	value: string;
	className?: string;
}) {
	const user = useAssigneeUser(value);
	if (!user) return <span className={cn("truncate", className)}>{value}</span>;
	const label = userDisplayName(user, value);
	return (
		<span
			className={cn(
				"inline-flex min-w-0 max-w-full items-center gap-1.5",
				className,
			)}
			title={label}
		>
			<Avatar className="h-5 w-5 shrink-0">
				<AvatarImage src={userAvatarUrl(user) ?? ""} alt={label} />
				<AvatarFallback className="text-[8px]">
					{userInitials(label, "??")}
				</AvatarFallback>
			</Avatar>
			<span className="truncate">{label}</span>
		</span>
	);
}

function MemberCommandItem({
	sub,
	onPick,
}: {
	sub: string;
	onPick: (sub: string) => void;
}) {
	const backend = useBackend();
	const lookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		[sub],
		true,
	);
	const label = userDisplayName(lookup.data, sub);
	const secondary = userSecondaryLabel(lookup.data);
	return (
		<CommandItem
			value={`${label} ${secondary ?? ""} ${sub}`}
			onSelect={() => onPick(sub)}
		>
			<Avatar className="h-5 w-5 shrink-0">
				<AvatarImage src={userAvatarUrl(lookup.data) ?? ""} alt={label} />
				<AvatarFallback className="text-[8px]">
					{userInitials(label, "??")}
				</AvatarFallback>
			</Avatar>
			<div className="min-w-0 flex-1">
				<div className="truncate text-xs">{label}</div>
				{secondary && secondary !== label && secondary !== `@${label}` && (
					<div className="truncate text-[10px] text-muted-foreground">
						{secondary}
					</div>
				)}
			</div>
		</CommandItem>
	);
}

function TeamMemberPicker({ onPick }: { onPick: (sub: string) => void }) {
	const backend = useBackend();
	const { appId } = useActionContext();
	const [open, setOpen] = useState(false);
	const team = useInvoke(
		backend.teamState.getTeam,
		backend.teamState,
		[appId ?? "", 0, 100],
		open && !!appId,
	);
	const members = useMemo(() => {
		const seen = new Set<string>();
		return ((team.data ?? []) as IMember[]).filter((m) => {
			if (!m.user_id || seen.has(m.user_id)) return false;
			seen.add(m.user_id);
			return true;
		});
	}, [team.data]);

	if (!appId) return null;
	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="outline"
					size="icon"
					className="h-8 w-8 shrink-0"
					aria-label="Pick from team"
					title="Pick from team"
				>
					<UsersIcon className="h-3.5 w-3.5" />
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-64 p-0">
				<Command>
					<CommandInput placeholder="Search team…" />
					<CommandList>
						<CommandEmpty>
							{team.isLoading ? "Loading members…" : "No members found"}
						</CommandEmpty>
						{members.map((m) => (
							<MemberCommandItem
								key={m.id}
								sub={m.user_id}
								onPick={(sub) => {
									onPick(sub);
									setOpen(false);
								}}
							/>
						))}
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}

/**
 * Assignee input: free text, or pick a team member (stores the user's sub,
 * shown resolved with a clear affordance).
 */
function AssigneeField({
	value,
	onChange,
}: {
	value: string;
	onChange: (value: string) => void;
}) {
	const user = useAssigneeUser(value || undefined);
	if (value && user) {
		return (
			<div className="flex h-8 items-center gap-1.5 rounded-md border border-border px-2">
				<AssigneeDisplay value={value} className="flex-1 text-sm" />
				<button
					type="button"
					aria-label="Clear assignee"
					onClick={() => onChange("")}
					className="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
				>
					<XIcon className="h-3 w-3" />
				</button>
			</div>
		);
	}
	return (
		<div className="flex items-center gap-1.5">
			<Input
				value={value}
				onChange={(e) => onChange(e.target.value)}
				placeholder="Type a name, or pick from the team"
				className="h-8 flex-1"
			/>
			<TeamMemberPicker onPick={onChange} />
		</div>
	);
}

// ── Event dialog (Calendar) ─────────────────────────────────────────

export interface EventDialogState {
	event: CalendarEvent;
	mode: PlanningDialogMode;
}

interface EventDialogProps {
	state: EventDialogState | null;
	editable: boolean;
	locale?: string;
	onClose: () => void;
	/** Fired with the edited event and the original it replaces. */
	onSave: (
		next: CalendarEvent,
		original: CalendarEvent,
		mode: PlanningDialogMode,
	) => void;
	onDelete: (event: CalendarEvent) => void;
}

export function EventDialog({
	state,
	editable,
	locale,
	onClose,
	onSave,
	onDelete,
}: EventDialogProps) {
	return (
		<Dialog open={!!state} onOpenChange={(open) => !open && onClose()}>
			<DialogContent
				className="overflow-hidden sm:max-w-md"
				onClick={(e) => e.stopPropagation()}
			>
				{state && (
					<EventDialogBody
						key={`${state.event.id}:${state.mode}`}
						state={state}
						editable={editable}
						locale={locale}
						onClose={onClose}
						onSave={onSave}
						onDelete={onDelete}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function formatEventRange(ev: CalendarEvent, locale?: string): string {
	const start = toDate(ev.start);
	const end = eventEnd(ev);
	const day = new Intl.DateTimeFormat(locale, {
		weekday: "short",
		month: "short",
		day: "numeric",
	});
	const time = new Intl.DateTimeFormat(locale, {
		hour: "2-digit",
		minute: "2-digit",
	});
	const sameDay = differenceInCalendarDays(end, start) === 0;
	if (ev.allDay) {
		return sameDay
			? `${day.format(start)} · All day`
			: `${day.format(start)} → ${day.format(end)} · All day`;
	}
	if (sameDay)
		return `${day.format(start)} · ${time.format(start)} – ${time.format(end)}`;
	return `${day.format(start)} ${time.format(start)} → ${day.format(end)} ${time.format(end)}`;
}

function EventDialogBody({
	state,
	editable,
	locale,
	onClose,
	onSave,
	onDelete,
}: EventDialogProps & { state: EventDialogState }) {
	const original = state.event;
	const [mode, setMode] = useState<PlanningDialogMode>(state.mode);
	const [title, setTitle] = useState(original.title);
	const [allDay, setAllDay] = useState(!!original.allDay);
	const [start, setStart] = useState(original.start);
	const [end, setEnd] = useState(original.end ?? "");
	const [color, setColor] = useState(original.color);
	const [location, setLocation] = useState(original.location ?? "");
	const [description, setDescription] = useState(original.description ?? "");
	const [link, setLink] = useState(original.link ?? "");
	const [metaEntries, setMetaEntries] = useState<MetaEntry[]>(() =>
		metadataToEntries(original.metadata),
	);

	const save = useCallback(() => {
		const startDate = toDate(start);
		let endDate = end ? toDate(end) : eventEnd({ ...original, start, allDay });
		if (endDate < startDate) endDate = startDate;
		const next: CalendarEvent = {
			...original,
			title: title.trim() || "Untitled",
			allDay,
			start: allDay ? toDateInput(startDate) : startDate.toISOString(),
			end: allDay ? toDateInput(endDate) : endDate.toISOString(),
			color,
			location: location.trim() || undefined,
			description: description.trim() || undefined,
			link: link.trim() || undefined,
			metadata: entriesToMetadata(metaEntries),
		};
		onSave(next, original, mode);
		onClose();
	}, [
		title,
		allDay,
		start,
		end,
		color,
		location,
		description,
		link,
		metaEntries,
		original,
		mode,
		onSave,
		onClose,
	]);

	if (mode === "view") {
		return (
			<>
				<AccentBar color={original.color} />
				<DialogHeader className="space-y-1.5">
					<DialogTitle className="flex items-center gap-2 pr-6 text-lg leading-tight">
						{colorDot(original.color)}
						<span className="truncate">{original.title}</span>
						{original.allDay && (
							<Badge variant="secondary" className="ml-1 shrink-0 text-[10px]">
								All day
							</Badge>
						)}
					</DialogTitle>
					<p className="text-sm text-muted-foreground">
						{formatEventRange(original, locale)}
					</p>
				</DialogHeader>
				{(original.location ||
					original.description ||
					original.link ||
					Object.keys(original.metadata ?? {}).length > 0) && (
					<div className="space-y-3 rounded-lg border border-border/60 bg-muted/30 px-3 py-2.5">
						{original.location && (
							<DetailRow icon={<MapPinIcon className="h-4 w-4" />}>
								{original.location}
							</DetailRow>
						)}
						{original.link && (
							<LinkRow link={original.link} onNavigate={onClose} />
						)}
						{original.description && (
							<DetailRow icon={<AlignLeftIcon className="h-4 w-4" />}>
								<p className="whitespace-pre-wrap text-muted-foreground">
									{original.description}
								</p>
							</DetailRow>
						)}
						{Object.keys(original.metadata ?? {}).length > 0 && (
							<MetadataRows metadata={original.metadata ?? {}} />
						)}
					</div>
				)}
				<DialogActions
					mode="view"
					editable={editable && original.editable !== false}
					onDelete={() => {
						onDelete(original);
						onClose();
					}}
					onEdit={() => setMode("edit")}
					onCancel={onClose}
					onSave={save}
				/>
			</>
		);
	}

	const startInput = allDay
		? toDateInput(toDate(start))
		: toDateTimeLocalInput(toDate(start));
	const endInput = allDay
		? toDateInput(end ? toDate(end) : eventEnd(original))
		: toDateTimeLocalInput(end ? toDate(end) : eventEnd(original));

	return (
		<>
			<AccentBar color={color} />
			<DialogHeader>
				<DialogTitle className="text-base">
					{mode === "create" ? "New event" : "Edit event"}
				</DialogTitle>
			</DialogHeader>
			<div className="space-y-3 py-1">
				<FieldRow label="Title" htmlFor="ev-title">
					<Input
						id="ev-title"
						value={title}
						autoFocus
						onChange={(e) => setTitle(e.target.value)}
						onKeyDown={(e) => e.key === "Enter" && save()}
						placeholder="Event title"
						className="h-8"
					/>
				</FieldRow>
				<div className="flex items-center justify-between rounded-md border border-border px-3 py-2">
					<Label htmlFor="ev-allday" className="text-xs">
						All day
					</Label>
					<Switch id="ev-allday" checked={allDay} onCheckedChange={setAllDay} />
				</div>
				<div className="grid grid-cols-2 gap-2">
					<FieldRow label="Start" htmlFor="ev-start">
						<Input
							id="ev-start"
							type={allDay ? "date" : "datetime-local"}
							value={startInput}
							onChange={(e) => e.target.value && setStart(e.target.value)}
							className="h-8 text-xs"
						/>
					</FieldRow>
					<FieldRow label="End" htmlFor="ev-end">
						<Input
							id="ev-end"
							type={allDay ? "date" : "datetime-local"}
							value={endInput}
							onChange={(e) => e.target.value && setEnd(e.target.value)}
							className="h-8 text-xs"
						/>
					</FieldRow>
				</div>
				<FieldRow label="Color">
					<ColorField value={color} onChange={setColor} />
				</FieldRow>
				<FieldRow label="Location" htmlFor="ev-location">
					<Input
						id="ev-location"
						value={location}
						onChange={(e) => setLocation(e.target.value)}
						placeholder="Add location"
						className="h-8"
					/>
				</FieldRow>
				<FieldRow label="Link" htmlFor="ev-link">
					<Input
						id="ev-link"
						value={link}
						onChange={(e) => setLink(e.target.value)}
						placeholder="/route/in-app or https://example.com"
						className="h-8"
					/>
				</FieldRow>
				<FieldRow label="Description" htmlFor="ev-description">
					<Textarea
						id="ev-description"
						value={description}
						onChange={(e) => setDescription(e.target.value)}
						placeholder="Add description"
						className="min-h-16 text-sm"
					/>
				</FieldRow>
				<FieldRow label="Metadata">
					<MetadataField entries={metaEntries} onChange={setMetaEntries} />
				</FieldRow>
			</div>
			<DialogActions
				mode={mode}
				editable={editable}
				onDelete={() => {
					onDelete(original);
					onClose();
				}}
				onEdit={() => setMode("edit")}
				onCancel={onClose}
				onSave={save}
			/>
		</>
	);
}

// ── Task dialog (Gantt) ─────────────────────────────────────────────

export interface TaskDialogState {
	task: GanttTask;
	mode: PlanningDialogMode;
}

interface TaskDialogProps {
	state: TaskDialogState | null;
	/** All tasks, used to offer dependency choices and resolve names. */
	tasks: GanttTask[];
	editable: boolean;
	locale?: string;
	onClose: () => void;
	onSave: (
		next: GanttTask,
		original: GanttTask,
		mode: PlanningDialogMode,
	) => void;
	onDelete: (task: GanttTask) => void;
}

export function TaskDialog({
	state,
	tasks,
	editable,
	locale,
	onClose,
	onSave,
	onDelete,
}: TaskDialogProps) {
	return (
		<Dialog open={!!state} onOpenChange={(open) => !open && onClose()}>
			<DialogContent
				className="overflow-hidden sm:max-w-md"
				onClick={(e) => e.stopPropagation()}
			>
				{state && (
					<TaskDialogBody
						key={`${state.task.id}:${state.mode}`}
						state={state}
						tasks={tasks}
						editable={editable}
						locale={locale}
						onClose={onClose}
						onSave={onSave}
						onDelete={onDelete}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function formatTaskRange(task: GanttTask, locale?: string): string {
	const start = toDate(task.start);
	const end = toDate(task.end);
	const fmt = new Intl.DateTimeFormat(locale, {
		month: "short",
		day: "numeric",
	});
	if (task.milestone) return fmt.format(start);
	const days = Math.abs(differenceInCalendarDays(end, start)) + 1;
	return `${fmt.format(start)} → ${fmt.format(end)} · ${days} day${days === 1 ? "" : "s"}`;
}

function TaskDialogBody({
	state,
	tasks,
	editable,
	locale,
	onClose,
	onSave,
	onDelete,
}: TaskDialogProps & { state: TaskDialogState }) {
	const original = state.task;
	const [mode, setMode] = useState<PlanningDialogMode>(state.mode);
	const [name, setName] = useState(original.name);
	const [start, setStart] = useState(original.start);
	const [end, setEnd] = useState(original.end);
	const [progress, setProgress] = useState(original.progress ?? 0);
	const [assignee, setAssignee] = useState(original.assignee ?? "");
	const [color, setColor] = useState(original.color);
	const [milestone, setMilestone] = useState(!!original.milestone);
	const [link, setLink] = useState(original.link ?? "");
	const [metaEntries, setMetaEntries] = useState<MetaEntry[]>(() =>
		metadataToEntries(original.metadata),
	);
	const [dependencies, setDependencies] = useState<string[]>(
		original.dependencies ?? [],
	);

	const dependencyChoices = useMemo(
		() => tasks.filter((t) => t.id !== original.id),
		[tasks, original.id],
	);
	const taskName = useCallback(
		(id: string) => tasks.find((t) => t.id === id)?.name ?? id,
		[tasks],
	);

	const save = useCallback(() => {
		const startDate = toDate(start);
		let endDate = milestone ? startDate : toDate(end);
		if (endDate < startDate) endDate = startDate;
		const next: GanttTask = {
			...original,
			name: name.trim() || "Untitled",
			start: toDateInput(startDate),
			end: toDateInput(endDate),
			progress: milestone
				? undefined
				: Math.max(0, Math.min(100, Math.round(progress))),
			assignee: assignee.trim() || undefined,
			color,
			milestone: milestone || undefined,
			link: link.trim() || undefined,
			metadata: entriesToMetadata(metaEntries),
			dependencies: dependencies.length > 0 ? dependencies : undefined,
		};
		onSave(next, original, mode);
		onClose();
	}, [
		name,
		start,
		end,
		progress,
		assignee,
		color,
		milestone,
		link,
		metaEntries,
		dependencies,
		original,
		mode,
		onSave,
		onClose,
	]);

	if (mode === "view") {
		const hasBody =
			(!original.milestone && original.progress != null) ||
			original.assignee ||
			original.link ||
			Object.keys(original.metadata ?? {}).length > 0 ||
			(original.dependencies?.length ?? 0) > 0;
		return (
			<>
				<AccentBar color={original.color} />
				<DialogHeader className="space-y-1.5">
					<DialogTitle className="flex items-center gap-2 pr-6 text-lg leading-tight">
						{colorDot(original.color)}
						<span className="truncate">{original.name}</span>
						{original.milestone && (
							<Badge variant="secondary" className="ml-1 shrink-0 text-[10px]">
								<DiamondIcon className="mr-1 h-2.5 w-2.5" /> Milestone
							</Badge>
						)}
					</DialogTitle>
					<p className="text-sm text-muted-foreground">
						{formatTaskRange(original, locale)}
					</p>
				</DialogHeader>
				{hasBody && (
					<div className="space-y-3 rounded-lg border border-border/60 bg-muted/30 px-3 py-2.5">
						{!original.milestone && original.progress != null && (
							<DetailRow icon={<GaugeIcon className="h-4 w-4" />}>
								<div className="flex items-center gap-2 pt-0.5">
									<div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
										<div
											className="h-full rounded-full bg-primary"
											style={{
												width: `${Math.max(0, Math.min(100, original.progress))}%`,
												backgroundColor: original.color,
											}}
										/>
									</div>
									<span className="shrink-0 text-xs tabular-nums text-muted-foreground">
										{Math.round(original.progress)}%
									</span>
								</div>
							</DetailRow>
						)}
						{original.assignee && (
							<DetailRow icon={<UserIcon className="h-4 w-4" />}>
								<AssigneeDisplay value={original.assignee} />
							</DetailRow>
						)}
						{original.link && (
							<LinkRow link={original.link} onNavigate={onClose} />
						)}
						{(original.dependencies?.length ?? 0) > 0 && (
							<DetailRow icon={<Link2Icon className="h-4 w-4" />}>
								<div className="flex flex-wrap gap-1">
									{original.dependencies?.map((dep) => (
										<Badge key={dep} variant="outline" className="text-[10px]">
											{taskName(dep)}
										</Badge>
									))}
								</div>
							</DetailRow>
						)}
						{Object.keys(original.metadata ?? {}).length > 0 && (
							<MetadataRows metadata={original.metadata ?? {}} />
						)}
					</div>
				)}
				<DialogActions
					mode="view"
					editable={editable}
					onDelete={() => {
						onDelete(original);
						onClose();
					}}
					onEdit={() => setMode("edit")}
					onCancel={onClose}
					onSave={save}
				/>
			</>
		);
	}

	return (
		<>
			<AccentBar color={color} />
			<DialogHeader>
				<DialogTitle className="text-base">
					{mode === "create" ? "New task" : "Edit task"}
				</DialogTitle>
			</DialogHeader>
			<div className="space-y-3 py-1">
				<FieldRow label="Name" htmlFor="task-name">
					<Input
						id="task-name"
						value={name}
						autoFocus
						onChange={(e) => setName(e.target.value)}
						onKeyDown={(e) => e.key === "Enter" && save()}
						placeholder="Task name"
						className="h-8"
					/>
				</FieldRow>
				<div className="flex items-center justify-between rounded-md border border-border px-3 py-2">
					<Label htmlFor="task-milestone" className="text-xs">
						Milestone
					</Label>
					<Switch
						id="task-milestone"
						checked={milestone}
						onCheckedChange={setMilestone}
					/>
				</div>
				<div className="grid grid-cols-2 gap-2">
					<FieldRow label={milestone ? "Date" : "Start"} htmlFor="task-start">
						<Input
							id="task-start"
							type="date"
							value={toDateInput(toDate(start))}
							onChange={(e) => e.target.value && setStart(e.target.value)}
							className="h-8 text-xs"
						/>
					</FieldRow>
					{!milestone && (
						<FieldRow label="End" htmlFor="task-end">
							<Input
								id="task-end"
								type="date"
								value={toDateInput(toDate(end))}
								onChange={(e) => e.target.value && setEnd(e.target.value)}
								className="h-8 text-xs"
							/>
						</FieldRow>
					)}
				</div>
				{!milestone && (
					<FieldRow label={`Progress · ${Math.round(progress)}%`}>
						<Slider
							value={[progress]}
							min={0}
							max={100}
							step={5}
							onValueChange={(v) => setProgress(v[0] ?? 0)}
						/>
					</FieldRow>
				)}
				<FieldRow label="Assignee">
					<AssigneeField value={assignee} onChange={setAssignee} />
				</FieldRow>
				<FieldRow label="Link" htmlFor="task-link">
					<Input
						id="task-link"
						value={link}
						onChange={(e) => setLink(e.target.value)}
						placeholder="/route/in-app or https://example.com"
						className="h-8"
					/>
				</FieldRow>
				<FieldRow label="Metadata">
					<MetadataField entries={metaEntries} onChange={setMetaEntries} />
				</FieldRow>
				<FieldRow label="Color">
					<ColorField value={color} onChange={setColor} />
				</FieldRow>
				{dependencyChoices.length > 0 && (
					<FieldRow label="Depends on">
						<div className="max-h-32 space-y-1 overflow-y-auto rounded-md border border-border p-2">
							{dependencyChoices.map((t) => (
								<label
									key={t.id}
									htmlFor={`task-dep-${t.id}`}
									className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-accent/50"
								>
									<Checkbox
										id={`task-dep-${t.id}`}
										checked={dependencies.includes(t.id)}
										onCheckedChange={(checked) =>
											setDependencies((prev) =>
												checked
													? [...prev, t.id]
													: prev.filter((d) => d !== t.id),
											)
										}
									/>
									<span className="truncate">{t.name}</span>
								</label>
							))}
						</div>
					</FieldRow>
				)}
			</div>
			<DialogActions
				mode={mode}
				editable={editable}
				onDelete={() => {
					onDelete(original);
					onClose();
				}}
				onEdit={() => setMode("edit")}
				onCancel={onClose}
				onSave={save}
			/>
		</>
	);
}
