"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	ArrowRightLeft,
	Check,
	CircleAlert,
	Copy,
	EyeOff,
	Info,
	Loader2,
	MoreHorizontal,
	Sparkles,
	Trash2,
	X,
} from "lucide-react";
import { type ReactNode, useCallback, useId, useMemo, useState } from "react";
import { cn } from "../../../../lib/utils";
import { ProfileMediaField } from "../../../profile-templates/profile-media-field";
import { audienceMatches } from "../../../store/explore/explore-model";
import {
	EXPLORE_LIMITS,
	type ExploreAudienceTag,
	type ExploreItemRef,
	type ExploreLayoutDoc,
	type ExplorePlacementContent,
	type ExplorePlacementInput,
	type ExplorePlacementItem,
	type ExploreSlotKey,
	type ExploreViewer,
	type ResolvedExplore,
} from "../../../store/explore/explore-types";
import { Alert, AlertDescription } from "../../../ui/alert";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../../ui/alert-dialog";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { Input } from "../../../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { SlotName } from "./add-menu";
import { CollectionRuleEditor } from "./collection-rule-editor";
import {
	AUDIENCE_TAGS,
	type PlacementLocation,
	RAIL_KEYS,
	TONES,
	type TraceMarkers,
	audienceIncludes,
	audienceLabel,
	defaultRule,
	effectiveStatus,
	errorMessage,
	fromLocalInput,
	kindLabel,
	moveTargets,
	patchItem,
	placementSummary,
	previewCollection,
	previewSlideCount,
	railLabel,
	railSource,
	referencingSpotlights,
	skipReasonLabel,
	slotAccepts,
	slotLabel,
	slotSize,
	spotlightMeta,
	statusNote,
	toLocalInput,
	toggleAudience,
	toneLabel,
} from "./explore-admin-model";
import {
	KindIcon,
	StatusPill,
	TONE_STYLES,
	audienceIcon,
} from "./explore-admin-visuals";
import {
	CountedField,
	FieldError,
	InspectorSection,
	Segmented,
	SwitchRow,
} from "./inspector-fields";
import {
	type ItemNameOf,
	ItemOverridesEditor,
	ItemsEditor,
} from "./inspector-items";
import type { DraftSaveState } from "./use-placement-draft";

export interface ArtworkUpload {
	available: boolean | undefined;
	upload: (file: Blob) => Promise<string>;
}

export interface PlacementInspectorProps {
	layout: ExploreLayoutDoc;
	location: PlacementLocation;
	input: ExplorePlacementInput;
	issueFor: (field: string) => string | undefined;
	saveState: DraftSaveState;
	serverError: string | null;
	refs: ReadonlyMap<string, ExploreItemRef>;
	nameOf: ItemNameOf;
	markers?: TraceMarkers;
	preview?: ResolvedExplore;
	viewer: ExploreViewer;
	artwork: ArtworkUpload;
	formatDate: (iso: string) => string;
	onEdit: (
		update: (input: ExplorePlacementInput) => ExplorePlacementInput,
	) => void;
	onPicked: (key: string, name: string) => void;
	onSelect: (id: string) => void;
	onDuplicate: () => Promise<void>;
	onMove: (target: ExploreSlotKey) => Promise<void>;
	onDelete: () => Promise<void>;
}

export function PlacementInspector(props: PlacementInspectorProps) {
	const { location, input, onEdit } = props;
	const content = input.content;
	const change = useCallback(
		(patch: Partial<ExplorePlacementInput>) =>
			onEdit((current) => ({ ...current, ...patch })),
		[onEdit],
	);
	const changeContent = useCallback(
		(patch: Partial<ExplorePlacementContent>) =>
			onEdit((current) => ({
				...current,
				content: { ...current.content, ...patch } as ExplorePlacementContent,
			})),
		[onEdit],
	);
	const setItems = useCallback(
		(items: ExplorePlacementItem[]) => change({ items }),
		[change],
	);
	return (
		<div className="flex flex-col gap-3.5 px-5 pt-4 pb-5">
			<InspectorHeader {...props} onRename={(name) => change({ name })} />
			<PreviewNotes {...props} />
			<VisibilitySection
				{...props}
				onToggle={(enabled) => change({ enabled })}
			/>
			{content.kind === "announcement" && (
				<AnnouncementSections
					{...props}
					content={content}
					onContent={changeContent}
				/>
			)}
			{content.kind === "spotlight" && (
				<SpotlightSections
					{...props}
					content={content}
					onContent={changeContent}
					onItems={setItems}
				/>
			)}
			{content.kind === "feature" && (
				<FeatureSections
					{...props}
					content={content}
					onContent={changeContent}
					onItems={setItems}
				/>
			)}
			{content.kind === "collection" && (
				<CollectionSections
					{...props}
					content={content}
					onContent={changeContent}
					onItems={setItems}
				/>
			)}
			{content.kind === "rail" && (
				<RailSections
					{...props}
					content={content}
					slotKey={location.slot.key}
					onContent={changeContent}
				/>
			)}
			{content.kind === "sponsored" && (
				<SponsoredSections
					{...props}
					content={content}
					onContent={changeContent}
					onItems={setItems}
				/>
			)}
			<ScheduleSection {...props} onChange={change} />
			<AudienceSection
				{...props}
				onToggle={(tag) =>
					onEdit((current) => ({
						...current,
						audience: toggleAudience(current.audience, tag),
					}))
				}
			/>
		</div>
	);
}

function SaveIndicator({ state }: { state: DraftSaveState }) {
	const { t } = useTranslation("admin");
	const view: Record<DraftSaveState, { icon?: ReactNode; label: string }> = {
		idle: { label: "" },
		pending: { label: t("exploreSavePending", "Unsaved") },
		saving: {
			icon: <Loader2 className="size-3 animate-spin" />,
			label: t("exploreSaveSaving", "Saving…"),
		},
		saved: {
			icon: <Check className="size-3" />,
			label: t("exploreSaveSaved", "Saved"),
		},
		invalid: {
			icon: <CircleAlert className="size-3" />,
			label: t("exploreSaveInvalid", "Fix to save"),
		},
		error: {
			icon: <CircleAlert className="size-3" />,
			label: t("exploreSaveError", "Not saved"),
		},
		conflict: {
			icon: <AlertTriangle className="size-3" />,
			label: t("exploreSaveConflict", "Not saved"),
		},
	};
	const current = view[state];
	return (
		<output
			aria-live="polite"
			className={cn(
				"inline-flex shrink-0 items-center gap-1 text-[11px] whitespace-nowrap",
				state === "invalid" || state === "error" || state === "conflict"
					? "text-destructive"
					: "text-muted-foreground",
			)}
		>
			{current.icon}
			{current.label}
		</output>
	);
}

function InspectorHeader({
	layout,
	location,
	input,
	issueFor,
	saveState,
	serverError,
	formatDate,
	onRename,
	onDuplicate,
	onMove,
	onDelete,
}: PlacementInspectorProps & { onRename: (name: string) => void }) {
	const { t } = useTranslation("admin");
	const status = effectiveStatus({ ...input, status: undefined });
	const slotKey = location.slot.key;
	const nameError = issueFor("name");
	return (
		<div className="flex flex-col gap-2.5">
			<div className="flex items-center gap-2">
				<span className="inline-flex h-5.5 shrink-0 items-center gap-1.5 rounded-md bg-primary/12 px-2 text-[11.5px] font-semibold text-primary">
					<KindIcon kind={input.content.kind} className="size-3" />
					{kindLabel(input.content.kind, t)}
				</span>
				<span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground">
					{slotKey === "unplaced"
						? slotLabel(layout, slotKey, t)
						: `${slotLabel(layout, slotKey, t)} · ${slotSize(slotKey)}`}
				</span>
				<span className="ml-auto shrink-0">
					<PlacementActions
						layout={layout}
						location={location}
						onDuplicate={onDuplicate}
						onMove={onMove}
						onDelete={onDelete}
					/>
				</span>
			</div>
			<Input
				value={input.name}
				aria-label={t("explorePlacementName", "Placement name")}
				aria-invalid={nameError ? true : undefined}
				onChange={(event) => onRename(event.target.value)}
				className="-ml-2.5 h-8.5 border-transparent bg-transparent px-2.5 text-[17px] font-semibold tracking-[-0.01em] shadow-none hover:border-input focus-visible:border-input dark:bg-transparent"
			/>
			<FieldError id="placement-name-error" message={nameError} />
			<div className="flex min-w-0 items-center gap-2">
				<StatusPill status={status} size="md" />
				<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
					{statusNote(input, status, t, formatDate)}
				</span>
				<SaveIndicator state={saveState} />
			</div>
			{serverError && (
				<Alert variant="destructive" role="alert" className="py-2">
					<CircleAlert className="size-4" />
					<AlertDescription className="text-xs">{serverError}</AlertDescription>
				</Alert>
			)}
		</div>
	);
}

function PlacementActions({
	layout,
	location,
	onDuplicate,
	onMove,
	onDelete,
}: Pick<
	PlacementInspectorProps,
	"layout" | "location" | "onDuplicate" | "onMove" | "onDelete"
>) {
	const { t } = useTranslation("admin");
	const [confirm, setConfirm] = useState(false);
	const [deleting, setDeleting] = useState(false);
	const [deleteError, setDeleteError] = useState<string | null>(null);
	const placement = location.placement;
	const targets = moveTargets(layout, placement.id);
	const referencing =
		placement.content.kind === "collection"
			? referencingSpotlights(layout, placement.id)
			: [];
	const remove = async () => {
		setDeleting(true);
		setDeleteError(null);
		try {
			await onDelete();
			setConfirm(false);
		} catch (error) {
			setDeleteError(
				errorMessage(
					error,
					t("exploreDeleteFailed", "The placement could not be deleted."),
				),
			);
		} finally {
			setDeleting(false);
		}
	};
	return (
		<>
			<DropdownMenu modal={false}>
				<DropdownMenuTrigger asChild>
					<Button
						type="button"
						variant="outline"
						size="icon"
						className="size-7 text-muted-foreground"
						aria-label={t(
							"exploreMoreActions",
							"More actions for this placement",
						)}
					>
						<MoreHorizontal className="size-4" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" className="w-52">
					<DropdownMenuItem onSelect={() => void onDuplicate()}>
						<Copy />
						{t("exploreDuplicate", "Duplicate")}
					</DropdownMenuItem>
					<DropdownMenuSub>
						<DropdownMenuSubTrigger disabled={!targets.length}>
							<ArrowRightLeft className="size-4 text-muted-foreground" />
							{t("exploreMoveTo", "Move to…")}
						</DropdownMenuSubTrigger>
						<DropdownMenuSubContent className="max-h-80 overflow-y-auto">
							{targets.map((key) => (
								<DropdownMenuItem key={key} onSelect={() => void onMove(key)}>
									<SlotName layout={layout} slot={key} />
									{key !== "unplaced" && (
										<span className="font-mono text-[11px] text-muted-foreground">
											{slotSize(key)}
										</span>
									)}
								</DropdownMenuItem>
							))}
						</DropdownMenuSubContent>
					</DropdownMenuSub>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						onSelect={() => {
							setDeleteError(null);
							setConfirm(true);
						}}
					>
						<Trash2 />
						{t("exploreDelete", "Delete")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
			<AlertDialog
				open={confirm}
				onOpenChange={(open) => !deleting && setConfirm(open)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("exploreDeleteTitle", "Delete “{{name}}”?", {
								name: placement.name,
							})}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"exploreDeleteDescription",
								"It leaves the draft now and the live page when you publish.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					{referencing.length > 0 && (
						<div className="rounded-lg border border-amber-500/40 bg-amber-500/8 p-3 text-sm">
							<p>
								{t(
									"exploreDeleteReferenced",
									"These spotlights show this collection as a slide. Remove it from them first:",
								)}
							</p>
							<ul className="mt-1.5 list-disc pl-5 text-muted-foreground">
								{referencing.map((entry) => (
									<li key={entry.id}>{entry.name}</li>
								))}
							</ul>
						</div>
					)}
					{deleteError && (
						<p role="alert" className="text-sm text-destructive">
							{deleteError}
						</p>
					)}
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deleting}>
							{t("exploreCancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							disabled={deleting || referencing.length > 0}
							className="bg-destructive text-white hover:bg-destructive/90"
							onClick={(event) => {
								event.preventDefault();
								void remove();
							}}
						>
							{deleting && <Loader2 className="size-4 animate-spin" />}
							{t("exploreDelete", "Delete")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

function Note({
	tone = "info",
	icon,
	children,
	action,
}: {
	tone?: "info" | "muted" | "warning";
	icon: ReactNode;
	children: ReactNode;
	action?: ReactNode;
}) {
	return (
		<div
			className={cn(
				"flex gap-2.5 rounded-lg px-3 py-2.5 text-xs leading-4.25",
				tone === "info" &&
					"border border-dashed border-blue-500/50 bg-blue-500/6",
				tone === "muted" && "border bg-muted/40",
				tone === "warning" && "border border-amber-500/40 bg-amber-500/7",
			)}
		>
			<span
				className={cn(
					"mt-px shrink-0",
					tone === "info" && "text-blue-600 dark:text-blue-400",
					tone === "muted" && "text-muted-foreground",
					tone === "warning" && "text-amber-600 dark:text-amber-400",
				)}
			>
				{icon}
			</span>
			<div className="flex min-w-0 flex-col gap-1.5">
				<span>{children}</span>
				{action}
			</div>
		</div>
	);
}

function PreviewNotes({
	layout,
	location,
	markers,
	onSelect,
}: PlacementInspectorProps) {
	const { t } = useTranslation("admin");
	const { slot, placement, index } = location;
	const hidden = markers?.hidden.get(placement.id);
	const link = (id: string, label: string) => (
		<button
			type="button"
			onClick={() => onSelect(id)}
			className="self-start text-xs font-medium text-blue-600 hover:underline dark:text-blue-400"
		>
			{label}
		</button>
	);
	const slotName = slotLabel(layout, slot.key, t);
	let structural: ReactNode = null;
	if (slot.key === "unplaced") {
		structural = (
			<Note tone="muted" icon={<Info className="size-4" />}>
				{placement.content.kind === "collection"
					? t(
							"exploreUnplacedCollectionNote",
							"Unplaced placements never show as a tile. This collection can still be a spotlight slide or open as a collection page.",
						)
					: t(
							"exploreUnplacedNote",
							"Unplaced placements never show on the page. Move it to a slot to use it.",
						)}
			</Note>
		);
	} else if (index > 0) {
		const primary = slot.placements[0];
		structural = (
			<Note
				icon={<Info className="size-4" />}
				action={link(
					primary.id,
					t("exploreEditPrimary", "Edit the primary placement"),
				)}
			>
				{t(
					"exploreFallbackOf",
					"Fallback for {{slot}}: shown when “{{name}}” is hidden for the viewer.",
					{ slot: slotName, name: primary.name },
				)}
			</Note>
		);
	} else if (slot.placements.length > 1) {
		const next = slot.placements[1];
		structural = (
			<Note
				icon={<Info className="size-4" />}
				action={link(next.id, t("exploreEditFallback", "Edit the fallback"))}
			>
				{t(
					"exploreHasFallback",
					"People who can’t see this get “{{name}}” in {{slot}} instead.",
					{ slot: slotName, name: next.name },
				)}
			</Note>
		);
	}
	return (
		<>
			{structural}
			{hidden && (
				<Note tone="muted" icon={<EyeOff className="size-4" />}>
					{skipReasonLabel(hidden, t)}
				</Note>
			)}
		</>
	);
}

function VisibilitySection({
	input,
	layout,
	location,
	onToggle,
}: PlacementInspectorProps & { onToggle: (enabled: boolean) => void }) {
	const { t } = useTranslation("admin");
	const kind = input.content.kind;
	if (kind === "sponsored") {
		return (
			<SwitchRow
				label={t("exploreTurnOn", "Turn on")}
				note={t(
					"exploreSponsoredLocked",
					"Sponsored placements cannot be turned on yet.",
				)}
				checked={false}
				disabled
				onCheckedChange={() => undefined}
			/>
		);
	}
	const rail = kind === "rail";
	const slotName = slotLabel(layout, location.slot.key, t);
	const note = input.enabled
		? rail
			? t("exploreRailShown", "Shown in {{slot}} while it has items", {
					slot: slotName,
				})
			: t("exploreEnabledNote", "Shows while the schedule and audience match")
		: rail
			? t("exploreRailHidden", "Hidden. The rows below move up.")
			: t(
					"exploreDisabledNote",
					"Off. Only admins see it, here in the editor.",
				);
	return (
		<SwitchRow
			label={
				rail
					? t("exploreShowRail", "Show this rail")
					: t("exploreTurnOn", "Turn on")
			}
			note={note}
			checked={input.enabled}
			onCheckedChange={onToggle}
		/>
	);
}

function MediaSection({
	title,
	value,
	artwork,
	error,
	onChange,
}: {
	title: string;
	value: string | null | undefined;
	artwork: PlacementInspectorProps["artwork"];
	error?: string;
	onChange: (value: string | null) => void;
}) {
	const { t } = useTranslation("admin");
	const [open, setOpen] = useState(false);
	return (
		<InspectorSection
			title={title}
			action={
				<div className="flex gap-1">
					<Button
						type="button"
						variant="secondary"
						size="sm"
						className="h-6 px-2 text-[11.5px]"
						aria-expanded={open}
						disabled={artwork.available === false}
						onClick={() => setOpen((current) => !current)}
					>
						{value
							? t("exploreReplaceImage", "Replace")
							: t("exploreAddImage", "Add image")}
					</Button>
					{value && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-6 px-2 text-[11.5px] text-muted-foreground"
							onClick={() => onChange(null)}
						>
							{t("exploreRemoveImage", "Remove")}
						</Button>
					)}
				</div>
			}
		>
			{value && !open && (
				<img
					src={value}
					alt=""
					className="h-20 w-full rounded-lg border object-cover"
				/>
			)}
			{artwork.available === false && (
				<p className="text-[11.5px] text-muted-foreground">
					{t(
						"exploreImageNoCdn",
						"This hub has no CDN for Explore images, so an image cannot be added.",
					)}
				</p>
			)}
			{open && artwork.available !== false && (
				<ProfileMediaField
					label={title}
					kind="cover"
					value={value}
					upload={artwork.upload}
					onChange={onChange}
				/>
			)}
			<FieldError id="media-error" message={error} />
		</InspectorSection>
	);
}

type ContentOf<K extends ExplorePlacementContent["kind"]> = Extract<
	ExplorePlacementContent,
	{ kind: K }
>;

interface KindSectionProps<K extends ExplorePlacementContent["kind"]>
	extends PlacementInspectorProps {
	content: ContentOf<K>;
	onContent: (patch: Partial<ContentOf<K>>) => void;
	onItems?: (items: ExplorePlacementItem[]) => void;
}

function AnnouncementSections({
	content,
	issueFor,
	artwork,
	onContent,
}: KindSectionProps<"announcement">) {
	const { t } = useTranslation("admin");
	return (
		<>
			<InspectorSection title={t("exploreTone", "Tone")}>
				<Segmented
					label={t("exploreTone", "Tone")}
					value={content.tone}
					options={TONES.map((tone) => ({
						value: tone,
						label: toneLabel(tone, t),
						adornment: (
							<span
								aria-hidden="true"
								className={cn("size-1.5 rounded-full", TONE_STYLES[tone].dot)}
							/>
						),
						pressedClassName: TONE_STYLES[tone].pressed,
					}))}
					onChange={(tone) => onContent({ tone })}
				/>
			</InspectorSection>
			<InspectorSection title={t("exploreMessage", "Message")}>
				<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
					<CountedField
						label={t("exploreFieldTitle", "Title")}
						value={content.title}
						max={EXPLORE_LIMITS.announcementTitle}
						placeholder={t("exploreTitlePlaceholder", "What's new")}
						error={issueFor("title")}
						onChange={(title) => onContent({ title })}
					/>
					<CountedField
						label={t("exploreFieldBody", "Body")}
						value={content.body}
						max={EXPLORE_LIMITS.announcementBody}
						multiline
						placeholder={t("exploreBodyPlaceholder", "One or two sentences")}
						error={issueFor("body")}
						onChange={(body) => onContent({ body })}
					/>
					<CountedField
						half
						label={t("exploreFieldCtaLabel", "Button label")}
						value={content.ctaLabel}
						max={EXPLORE_LIMITS.ctaLabel}
						placeholder={t("exploreOptional", "Optional")}
						error={issueFor("ctaLabel")}
						onChange={(ctaLabel) => onContent({ ctaLabel })}
					/>
					<CountedField
						half
						inputMode="url"
						label={t("exploreFieldCtaLink", "Button link")}
						value={content.ctaHref}
						placeholder="/store/explore"
						error={issueFor("ctaHref")}
						onChange={(ctaHref) => onContent({ ctaHref })}
					/>
				</div>
			</InspectorSection>
			<SwitchRow
				label={t("exploreDismissible", "Dismissible")}
				note={
					content.dismissible
						? t(
								"exploreDismissibleOn",
								"People can close it, and it stays closed for them",
							)
						: t("exploreDismissibleOff", "Stays until the schedule ends")
				}
				checked={content.dismissible}
				onCheckedChange={(dismissible) => onContent({ dismissible })}
			/>
			<MediaSection
				title={t("exploreImage", "Image")}
				value={content.imageUrl}
				artwork={artwork}
				error={issueFor("imageUrl")}
				onChange={(imageUrl) => onContent({ imageUrl })}
			/>
		</>
	);
}

function useSelectedItem(items: readonly ExplorePlacementItem[]) {
	const [selected, setSelected] = useState(0);
	const index = items.length ? Math.min(selected, items.length - 1) : -1;
	return [index, setSelected] as const;
}

function OverridesFor({
	props,
	index,
}: {
	props: PlacementInspectorProps;
	index: number;
}) {
	const { input, nameOf, refs, issueFor, artwork, onEdit } = props;
	const item = input.items[index];
	if (!item) return null;
	return (
		<ItemOverridesEditor
			item={item}
			index={index}
			name={nameOf(item)}
			itemRef={refs.get(`${item.kind}:${item.id}`)}
			issueFor={issueFor}
			artwork={artwork}
			onPatch={(patch) =>
				onEdit((current) => ({
					...current,
					items: patchItem(current.items, item, index, patch),
				}))
			}
		/>
	);
}

function SpotlightSections(props: KindSectionProps<"spotlight">) {
	const { t } = useTranslation("admin");
	const { content, input, layout, location, refs, nameOf, issueFor } = props;
	const onItems = props.onItems ?? (() => undefined);
	const [index, setIndex] = useSelectedItem(input.items);
	const visible = previewSlideCount(props.preview, location.placement.id);
	const seconds = useMemo(
		() =>
			Array.from(
				{ length: EXPLORE_LIMITS.rotationMax - EXPLORE_LIMITS.rotationMin + 1 },
				(_, offset) => EXPLORE_LIMITS.rotationMin + offset,
			),
		[],
	);
	return (
		<>
			<ItemsEditor
				title={t("exploreRotation", "Rotation")}
				meta={spotlightMeta(input, visible, t, nameOf)}
				addLabel={t("exploreAdd", "Add")}
				items={input.items}
				kinds={["app", "package", "collection"]}
				max={EXPLORE_LIMITS.spotlightItems}
				layout={layout}
				placementId={location.placement.id}
				refs={refs}
				nameOf={nameOf}
				selectedIndex={index}
				error={issueFor("items")}
				onSelect={setIndex}
				onChange={onItems}
				onPicked={props.onPicked}
			/>
			<SwitchRow
				label={t("exploreAutoFill", "Fill from Popular right now")}
				note={t(
					"exploreAutoFillNote",
					"Tops the rotation up to 3 slides from the trending rail",
				)}
				checked={content.autoFill === true}
				onCheckedChange={(autoFill) => props.onContent({ autoFill })}
				trailing={<Sparkles className="size-4 text-muted-foreground" />}
			/>
			<div className="flex items-center justify-between gap-3 text-xs">
				<label htmlFor="spotlight-rotation" className="text-muted-foreground">
					{t("exploreRotateEvery", "Rotate every")}
				</label>
				<Select
					value={String(content.rotationSeconds)}
					onValueChange={(value) =>
						props.onContent({ rotationSeconds: Number(value) })
					}
				>
					<SelectTrigger
						id="spotlight-rotation"
						size="sm"
						className="h-7.5 w-28 bg-background text-xs"
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{seconds.map((value) => (
							<SelectItem key={value} value={String(value)}>
								{t("exploreSeconds", "{{count}} seconds", { count: value })}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<FieldError id="rotation-error" message={issueFor("rotationSeconds")} />
			<OverridesFor props={props} index={index} />
		</>
	);
}

function FeatureSections(props: KindSectionProps<"feature">) {
	const { t } = useTranslation("admin");
	const { content, input, layout, location, refs, nameOf, issueFor } = props;
	const onItems = props.onItems ?? (() => undefined);
	const item = input.items[0];
	return (
		<>
			<ItemsEditor
				title={t("exploreFeaturedItem", "Featured item")}
				addLabel={
					item ? t("exploreReplace", "Replace") : t("explorePick", "Pick")
				}
				items={input.items}
				kinds={["app", "package"]}
				max={1}
				layout={layout}
				placementId={location.placement.id}
				refs={refs}
				nameOf={nameOf}
				error={issueFor("items")}
				onChange={onItems}
				onPicked={props.onPicked}
			/>
			<InspectorSection title={t("exploreDisplay", "Display")}>
				<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
					<CountedField
						label={t("exploreEyebrow", "Eyebrow")}
						value={content.eyebrow}
						max={EXPLORE_LIMITS.eyebrow}
						placeholder={t(
							"exploreEyebrowPlaceholder",
							"Chosen from the item kind",
						)}
						error={issueFor("eyebrow")}
						onChange={(eyebrow) => props.onContent({ eyebrow })}
					/>
				</div>
			</InspectorSection>
			<OverridesFor props={props} index={0} />
		</>
	);
}

function CollectionSections(props: KindSectionProps<"collection">) {
	const { t } = useTranslation("admin");
	const { content, input, layout, location, refs, nameOf, issueFor } = props;
	const onItems = props.onItems ?? (() => undefined);
	const rule = content.source === "rule";
	return (
		<>
			<InspectorSection title={t("exploreItemsFrom", "Items come from")}>
				<Segmented
					label={t("exploreItemsFrom", "Items come from")}
					value={content.source}
					options={[
						{ value: "hand", label: t("exploreHandPicked", "Hand-picked") },
						{ value: "rule", label: t("exploreByRule", "A rule") },
					]}
					onChange={(source) =>
						props.onContent({
							source,
							rule:
								source === "rule"
									? (content.rule ?? defaultRule())
									: content.rule,
						})
					}
				/>
				{rule && content.rule && (
					<CollectionRuleEditor
						rule={content.rule}
						resolved={previewCollection(props.preview, location.placement.id)}
						issueFor={issueFor}
						onChange={(next) => props.onContent({ rule: next })}
					/>
				)}
			</InspectorSection>
			<ItemsEditor
				title={
					rule
						? t("explorePinnedItems", "Pinned items")
						: t("exploreItems", "Items")
				}
				meta={
					rule
						? t("explorePinnedHint", "Shown before the rule's matches")
						: placementSummary(input, t, nameOf)
				}
				addLabel={t("exploreAdd", "Add")}
				items={input.items}
				kinds={["app", "package"]}
				max={rule ? EXPLORE_LIMITS.rulePinned : EXPLORE_LIMITS.collectionItems}
				layout={layout}
				placementId={location.placement.id}
				refs={refs}
				nameOf={nameOf}
				error={issueFor("items")}
				onChange={onItems}
				onPicked={props.onPicked}
			/>
			<InspectorSection title={t("exploreCopy", "Copy")}>
				<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
					<CountedField
						label={t("exploreFieldTitle", "Title")}
						value={content.title}
						max={EXPLORE_LIMITS.collectionTitle}
						placeholder={t(
							"exploreCollectionTitlePlaceholder",
							"Collection title",
						)}
						error={issueFor("title")}
						onChange={(title) => props.onContent({ title })}
					/>
					<CountedField
						label={t("exploreFieldBlurb", "Blurb")}
						value={content.blurb}
						max={EXPLORE_LIMITS.blurb}
						multiline
						placeholder={t(
							"exploreBlurbPlaceholder",
							"One line on why these belong together",
						)}
						error={issueFor("blurb")}
						onChange={(blurb) => props.onContent({ blurb })}
					/>
				</div>
			</InspectorSection>
		</>
	);
}

function RailSections(
	props: KindSectionProps<"rail"> & { slotKey: ExploreSlotKey },
) {
	const { t } = useTranslation("admin");
	const { content, slotKey, issueFor } = props;
	const rails = RAIL_KEYS.filter((rail) =>
		slotAccepts(slotKey, { ...content, rail }),
	);
	return (
		<>
			<InspectorSection title={t("exploreDisplay", "Display")}>
				<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
					<CountedField
						label={t("exploreRailTitle", "Rail title")}
						value={content.title}
						max={EXPLORE_LIMITS.railTitle}
						placeholder={railLabel(content.rail, t)}
						error={issueFor("title")}
						onChange={(title) => props.onContent({ title })}
					/>
				</div>
				<div className="flex items-center justify-between gap-3 text-xs">
					<label htmlFor="rail-key" className="text-muted-foreground">
						{t("exploreRailSourceLabel", "Rail")}
					</label>
					<Select
						value={content.rail}
						disabled={rails.length < 2}
						onValueChange={(rail) =>
							props.onContent({ rail: rail as typeof content.rail })
						}
					>
						<SelectTrigger
							id="rail-key"
							size="sm"
							className="h-7.5 w-48 bg-background text-xs"
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{rails.map((rail) => (
								<SelectItem key={rail} value={rail}>
									{railLabel(rail, t)}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
			</InspectorSection>
			<div className="flex flex-col gap-1.5 rounded-lg border bg-background px-3 py-2.5">
				<span className="inline-flex items-center gap-1.5 text-xs font-medium">
					<Sparkles className="size-3.5 text-muted-foreground" />
					{t("exploreAutomaticItems", "Automatic items")}
				</span>
				<span className="text-xs leading-4.25 text-muted-foreground">
					{railSource(content.rail, t)}
				</span>
				<span className="text-[11.5px] leading-4 text-muted-foreground/80">
					{t(
						"exploreRailExplainer",
						"System rails fill themselves. Rename, reorder or hide them; their items cannot be edited.",
					)}
				</span>
			</div>
		</>
	);
}

function SponsoredSections(props: KindSectionProps<"sponsored">) {
	const { t } = useTranslation("admin");
	const { content, input, layout, location, refs, nameOf, issueFor } = props;
	return (
		<>
			<Note tone="warning" icon={<AlertTriangle className="size-4" />}>
				{t(
					"exploreSponsoredNotice",
					"Sponsored placements always show a visible “Sponsored” label and use their own frame, so they never look like an app or package card.",
				)}
			</Note>
			<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
				<CountedField
					label={t("exploreAdvertiser", "Advertiser")}
					value={content.advertiser}
					max={EXPLORE_LIMITS.advertiser}
					placeholder={t("exploreAdvertiserPlaceholder", "Company name")}
					error={issueFor("advertiser")}
					onChange={(advertiser) => props.onContent({ advertiser })}
				/>
			</div>
			<ItemsEditor
				title={t("explorePromotedItem", "Promoted item")}
				addLabel={
					input.items.length
						? t("exploreReplace", "Replace")
						: t("explorePick", "Pick")
				}
				items={input.items}
				kinds={["app", "package"]}
				max={1}
				layout={layout}
				placementId={location.placement.id}
				refs={refs}
				nameOf={nameOf}
				error={issueFor("items")}
				onChange={props.onItems ?? (() => undefined)}
				onPicked={props.onPicked}
			/>
		</>
	);
}

function timeZone(): string {
	try {
		return Intl.DateTimeFormat().resolvedOptions().timeZone;
	} catch {
		return "";
	}
}

function DateField({
	label,
	value,
	placeholder,
	invalid,
	onChange,
}: {
	label: string;
	value: string | null | undefined;
	placeholder: string;
	invalid: boolean;
	onChange: (value: string | null) => void;
}) {
	const { t } = useTranslation("admin");
	const id = useId();
	return (
		<div className="flex min-w-0 flex-col gap-1.25">
			<div className="flex items-center justify-between text-xs text-muted-foreground">
				<label htmlFor={id}>{label}</label>
				{value && (
					<button
						type="button"
						className="rounded p-0.5 hover:text-foreground"
						aria-label={t("exploreClearDate", "Clear {{label}}", { label })}
						onClick={() => onChange(null)}
					>
						<X className="size-3" />
					</button>
				)}
			</div>
			<Input
				id={id}
				type="datetime-local"
				value={toLocalInput(value)}
				placeholder={placeholder}
				title={value ? undefined : placeholder}
				aria-invalid={invalid ? true : undefined}
				className="h-8.5 bg-background px-2 font-mono text-[11.5px]"
				onChange={(event) => onChange(fromLocalInput(event.target.value))}
			/>
			{!value && (
				<span className="text-[11px] text-muted-foreground">{placeholder}</span>
			)}
		</div>
	);
}

function ScheduleSection({
	input,
	issueFor,
	onChange,
}: PlacementInspectorProps & {
	onChange: (patch: Partial<ExplorePlacementInput>) => void;
}) {
	const { t } = useTranslation("admin");
	const error = issueFor("schedule");
	return (
		<InspectorSection
			title={t("exploreSchedule", "Schedule")}
			meta={timeZone()}
		>
			<div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-1">
				<DateField
					label={t("exploreStarts", "Starts")}
					value={input.startsAt}
					placeholder={t("exploreStartsEmpty", "Immediately")}
					invalid={!!error}
					onChange={(startsAt) => onChange({ startsAt })}
				/>
				<DateField
					label={t("exploreEnds", "Ends")}
					value={input.endsAt}
					placeholder={t("exploreEndsEmpty", "No end date")}
					invalid={!!error}
					onChange={(endsAt) => onChange({ endsAt })}
				/>
			</div>
			<FieldError id="schedule-error" message={error} />
		</InspectorSection>
	);
}

function AudienceSection({
	input,
	viewer,
	issueFor,
	onToggle,
}: PlacementInspectorProps & { onToggle: (tag: ExploreAudienceTag) => void }) {
	const { t } = useTranslation("admin");
	const visible = audienceMatches(input.audience, viewer);
	return (
		<InspectorSection
			title={t("exploreAudience", "Audience")}
			meta={
				visible
					? t("exploreAudienceVisible", "Visible in this preview")
					: t("exploreAudienceHidden", "Hidden in this preview")
			}
		>
			<fieldset
				aria-label={t("exploreAudience", "Audience")}
				className="flex min-w-0 flex-wrap gap-1.5"
			>
				{AUDIENCE_TAGS.map((tag) => {
					const on = audienceIncludes(input.audience, tag);
					const Icon = audienceIcon(tag);
					return (
						<button
							key={tag}
							type="button"
							aria-pressed={on}
							onClick={() => onToggle(tag)}
							className={cn(
								"inline-flex h-7 items-center gap-1.25 rounded-full border px-2.5 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
								on
									? "border-primary/45 bg-primary/12 font-medium text-primary"
									: "text-muted-foreground hover:text-foreground",
							)}
						>
							{on ? (
								<Check aria-hidden="true" className="size-3" />
							) : (
								<Icon aria-hidden="true" className="size-3" />
							)}
							{audienceLabel(tag, t)}
						</button>
					);
				})}
			</fieldset>
			<FieldError id="audience-error" message={issueFor("audience")} />
		</InspectorSection>
	);
}
