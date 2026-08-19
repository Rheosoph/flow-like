"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowLeft,
	Check,
	Copy,
	LayoutGridIcon,
	Link2,
	Loader2,
	MoreVertical,
	Pencil,
	Plus,
	RotateCcw,
	Settings,
	Trash2,
	Zap,
} from "lucide-react";
import { type CSSProperties, useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks";
import {
	type IMetadata,
	cn,
	formatRelativeTime,
	nowSystemTime,
	useSetQueryParams,
} from "../../lib";
import { useBackend } from "../../state/backend-state";
import type {
	ExposedProp,
	ExposedPropType,
} from "../../state/backend-state/widget-state";
import type { IDate } from "../../types";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { DataProvider } from "../a2ui/DataContext";
import {
	type WidgetComponentDef,
	applyExposedProps,
} from "../a2ui/layout/A2UIWidgetInstance";
import type { Surface, SurfaceComponent, WidgetAction } from "../a2ui/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { ScrollArea } from "../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
} from "../ui/sheet";
import { Switch } from "../ui/switch";
import { TextEditor } from "../ui/text-editor";
import { Textarea } from "../ui/textarea";

const DOT_GRID: CSSProperties = {
	backgroundImage:
		"radial-gradient(circle at 1px 1px, var(--border) 1px, transparent 0)",
	backgroundSize: "14px 14px",
};

const VIEWPORTS = [360, 440, 720] as const;

/** Groups the builder writes, in the order they read best. Anything else follows alphabetically. */
const GROUP_ORDER = ["Content", "Data", "Style"];

type PropValues = Record<string, unknown>;

interface EditState {
	name: string;
	description: string;
	long_description: string;
	tags: string[];
}

export function WidgetDetail({
	appId,
	widgetId,
}: Readonly<{ appId: string; widgetId: string }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const setQueryParams = useSetQueryParams();
	const [previewWidth, setPreviewWidth] = useState<number | null>(440);
	const [overrides, setOverrides] = useState<PropValues>({});
	const [isEditOpen, setIsEditOpen] = useState(false);

	const widget = useInvoke(
		backend.widgetState.getWidget,
		backend.widgetState,
		[appId, widgetId],
		!!appId && !!widgetId,
	);

	const metadata = useInvoke(
		backend.widgetState.getWidgetMeta,
		backend.widgetState,
		[appId, widgetId],
		!!appId && !!widgetId,
	);

	const exposedProps = useMemo(
		() => widget.data?.exposedProps ?? [],
		[widget.data],
	);

	const surface = useMemo<Surface | null>(() => {
		const components = widget.data?.components;
		if (!components?.length) return null;

		const applied = applyExposedProps(
			components as unknown as WidgetComponentDef[],
			exposedProps,
			overrides,
		) as unknown as SurfaceComponent[];

		const record = applied.reduce<Record<string, SurfaceComponent>>(
			(acc, component) => {
				acc[component.id] = component;
				return acc;
			},
			{},
		);
		const storedRootId = widget.data?.rootComponentId;
		const rootComponentId = record.root
			? "root"
			: storedRootId && record[storedRootId]
				? storedRootId
				: (applied[0]?.id ?? "");

		return { id: widgetId, rootComponentId, components: record };
	}, [widget.data, exposedProps, overrides, widgetId]);

	const groups = useMemo(() => {
		const byGroup = new Map<string, ExposedProp[]>();
		for (const prop of exposedProps) {
			const group = prop.group?.trim() || "General";
			byGroup.set(group, [...(byGroup.get(group) ?? []), prop]);
		}
		return Array.from(byGroup.entries()).sort(([a], [b]) => {
			if (a === "General") return -1;
			if (b === "General") return 1;
			const ia = GROUP_ORDER.indexOf(a);
			const ib = GROUP_ORDER.indexOf(b);
			if (ia !== -1 || ib !== -1) {
				return (
					(ia === -1 ? GROUP_ORDER.length : ia) -
					(ib === -1 ? GROUP_ORDER.length : ib)
				);
			}
			return a.localeCompare(b);
		});
	}, [exposedProps]);

	const name = metadata.data?.name || widget.data?.name || widgetId;
	const version = widget.data?.version;
	const tags = metadata.data?.tags ?? widget.data?.tags ?? [];
	const description =
		metadata.data?.description || widget.data?.description || "";
	const longDescription = metadata.data?.long_description || "";
	const actions = (widget.data?.actions ?? []) as WidgetAction[];
	const overrideCount = Object.keys(overrides).length;

	const openBuilder = useCallback(() => {
		window.location.href = `/widget?id=${widgetId}&app=${appId}`;
	}, [widgetId, appId]);

	const copyId = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(widgetId);
			toast.success("Widget ID copied");
		} catch {
			toast.error("Could not reach the clipboard");
		}
	}, [widgetId]);

	const handleDelete = useCallback(async () => {
		try {
			await backend.widgetState.deleteWidget(appId, widgetId);
			toast.success("Widget deleted");
			setQueryParams("widgetId", undefined);
		} catch (error) {
			console.error("Failed to delete widget:", error);
			toast.error("Failed to delete widget");
		}
	}, [appId, widgetId, backend.widgetState, setQueryParams]);

	const handleSaveMeta = useCallback(
		async (next: EditState) => {
			await backend.widgetState.pushWidgetMeta(appId, widgetId, {
				...((metadata.data ?? {
					created_at: nowSystemTime(),
					preview_media: [],
				}) as IMetadata),
				name: next.name,
				description: next.description,
				long_description: next.long_description,
				tags: next.tags,
				updated_at: nowSystemTime(),
			});
			await metadata.refetch();
			setIsEditOpen(false);
			toast.success("Widget details saved");
		},
		[appId, widgetId, backend.widgetState, metadata],
	);

	if (widget.isLoading || metadata.isLoading) {
		return (
			<div className="flex h-full items-center justify-center">
				<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
			</div>
		);
	}

	return (
		<main className="flex h-full max-h-full flex-col overflow-hidden p-6 pt-0">
			<div className="flex flex-wrap items-center gap-3 py-3">
				<Button
					variant="outline"
					size="icon"
					className="h-7 w-7 shrink-0"
					onClick={() => setQueryParams("widgetId", undefined)}
					aria-label={t('backToWidgets', 'Back to widgets')}
				>
					<ArrowLeft className="h-4 w-4" />
				</Button>
				<div className="min-w-0">
					<div className="text-[11px] text-muted-foreground">
						{t('widgets', 'Widgets')}{tags[0] ? ` / ${tags[0]}` : ""}
					</div>
					<div className="flex items-center gap-2">
						<h1 className="truncate text-[17px] font-semibold tracking-tight">
							{name}
						</h1>
						<Badge
							variant={version ? "secondary" : "outline"}
							className={cn(
								"font-mono text-[9.5px] uppercase tracking-wide",
								!version && "text-tertiary",
							)}
						>
							{version ? "Published" : "Draft"}
						</Badge>
					</div>
				</div>

				<button
					type="button"
					onClick={copyId}
					className="hidden items-center gap-1.5 rounded-md border px-2 py-1 font-mono text-[10.5px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring md:inline-flex"
				>
					<Copy className="h-3 w-3" />
					{widgetId}
					{version && ` · v${version.join(".")}`}
				</button>

				<div className="flex-1" />

				<Button variant="outline" size="sm" onClick={() => setIsEditOpen(true)}>
					<Pencil className="mr-1.5 h-3.5 w-3.5" />
					{t('editDetails', 'Edit details')}
				</Button>
				<Button size="sm" onClick={openBuilder}>
					<Settings className="mr-1.5 h-3.5 w-3.5" />
					{t('openBuilder', 'Open builder')}
				</Button>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button
							variant="outline"
							size="icon"
							className="h-8 w-8"
							aria-label={t('moreActions', 'More actions')}
						>
							<MoreVertical className="h-4 w-4" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						<DropdownMenuItem onClick={copyId}>
							<Copy className="mr-2 h-4 w-4" />
							{t('copyWidgetId', 'Copy widget ID')}
						</DropdownMenuItem>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							className="text-destructive focus:text-destructive"
							onClick={handleDelete}
						>
							<Trash2 className="mr-2 h-4 w-4" />
							{t('deleteWidget', 'Delete widget')}
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>
			</div>

			<div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden rounded-lg border lg:grid-cols-[1fr_340px]">
				<section className="flex min-h-0 min-w-0 flex-col border-b lg:border-b-0 lg:border-r">
					<div className="flex items-center gap-3 border-b px-4 py-2">
						<span className="text-[10px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
							{t('preview', 'Preview')}
						</span>
						<div className="flex h-6 items-center overflow-hidden rounded-md border">
							{VIEWPORTS.map((width) => (
								<ViewportButton
									key={width}
									active={previewWidth === width}
									onClick={() => setPreviewWidth(width)}
								>
									{width}
								</ViewportButton>
							))}
							<ViewportButton
								active={previewWidth === null}
								onClick={() => setPreviewWidth(null)}
							>
								{t('fill', 'Fill')}
							</ViewportButton>
						</div>
						<div className="flex-1" />
						<span className="flex items-center gap-1.5 text-[11px] text-muted-foreground"><Trans i18nKey="spanClassnameh15W15RoundedfullBgtertiaryLiveSampleData"><span className="h-1.5 w-1.5 rounded-full bg-tertiary" />
							Live · sample data</Trans></span>
					</div>

					<div
						className="flex flex-1 justify-center overflow-auto bg-muted/25 p-8"
						style={DOT_GRID}
					>
						{surface ? (
							<div
								className="h-fit max-w-full"
								style={{ width: previewWidth ?? "100%" }}
							>
								<DataProvider initialData={widget.data?.dataModel ?? []}>
									<A2UIRenderer
										surface={surface}
										appId={appId}
										isPreviewMode={true}
									/>
								</DataProvider>
							</div>
						) : (
							<div className="flex flex-col items-center justify-center gap-2 text-muted-foreground">
								<LayoutGridIcon className="h-10 w-10 opacity-40" />
								<p className="text-sm">{t('thisWidgetHasNoElementsYet', 'This widget has no elements yet')}</p>
								<Button variant="link" size="sm" onClick={openBuilder}>
									{t('openTheBuilderToAddSome', 'Open the builder to add some')}
								</Button>
							</div>
						)}
					</div>

					<div className="flex flex-wrap items-start gap-x-8 gap-y-4 border-t px-4 py-3">
						<StripItem label="Description">
							{description ? (
								<p className="max-w-[52ch] text-[12.5px]">{description}</p>
							) : (
								<AddLink onClick={() => setIsEditOpen(true)}>
									{t('addADescription', 'Add a description')}
								</AddLink>
							)}
						</StripItem>

						<StripItem label="Tags">
							{tags.length > 0 ? (
								<div className="flex flex-wrap items-center gap-1">
									{tags.map((tag) => (
										<Badge key={tag} variant="outline" className="text-[11px]">
											{tag}
										</Badge>
									))}
								</div>
							) : (
								<AddLink onClick={() => setIsEditOpen(true)}>{t('addTags', 'Add tags')}</AddLink>
							)}
						</StripItem>

						<StripItem label={t('notes', 'Notes')}>
							{longDescription ? (
								<button
									type="button"
									onClick={() => setIsEditOpen(true)}
									className="text-[12.5px] text-muted-foreground underline-offset-2 hover:underline"
								>
									{t('writtenOpenToEdit', 'Written · open to edit')}
								</button>
							) : (
								<AddLink onClick={() => setIsEditOpen(true)}>
									{t('writeDetailedNotes', 'Write detailed notes')}
								</AddLink>
							)}
						</StripItem>

						<div className="flex-1" />

						<div className="flex gap-6">
							<Fact label="Version">
								{version ? `v${version.join(".")}` : "Draft"}
							</Fact>
							<Fact label="Elements">
								{widget.data?.components?.length ?? 0}
							</Fact>
							<Fact label="Data">{widget.data?.dataModel?.length ?? 0}</Fact>
							{metadata.data?.updated_at && (
								<Fact label={t('updated', 'Updated')}>
									{formatRelativeTime(
										metadata.data.updated_at as IDate,
										"short",
									)}
								</Fact>
							)}
						</div>
					</div>
				</section>

				<aside className="flex min-h-0 flex-col bg-muted/20">
					<div className="shrink-0 space-y-1 border-b px-4 py-3">
						<div className="flex items-center gap-2">
							<h2 className="text-[13px] font-semibold">{t('properties', 'Properties')}</h2>
							<Badge
								variant="outline"
								className="font-mono text-[10.5px] tabular-nums"
							>
								{exposedProps.length}
							</Badge>
						</div>
						<p className="text-[11.5px] leading-snug text-muted-foreground">
							{t('whatAPageCanConfigureWhenItPlacesThisWidgetChangesHereOnlyAffectThePreview', "What a page can configure when it places this widget. Changes here only affect the preview.")}
						</p>
					</div>

					<ScrollArea className="min-h-0 flex-1">
						<div className="px-4 pb-4">
							{exposedProps.length === 0 ? (
								<div className="flex flex-col items-center gap-2 py-10 text-center text-muted-foreground">
									<Settings className="h-7 w-7 opacity-40" />
									<p className="text-sm">{t('noPropertiesExposedYet', 'No properties exposed yet')}</p>
									<p className="max-w-[30ch] text-[11.5px]">
										{`Expose a property in the builder to let pages configure this widget without duplicating it.`}
									</p>
									<Button variant="link" size="sm" onClick={openBuilder}>
										{t('openTheBuilder', 'Open the builder')}
									</Button>
								</div>
							) : (
								groups.map(([group, props]) => (
									<div key={group}>
										<div className="flex items-center gap-2 pb-2 pt-4">
											<span className="text-[9.5px] font-semibold uppercase tracking-[0.09em] text-muted-foreground">
												{group}
											</span>
											<span className="h-px flex-1 bg-border" />
										</div>
										{props.map((prop) => (
											<PropControl
												key={prop.id}
												prop={prop}
												components={widget.data?.components ?? []}
												override={overrides[prop.id]}
												onChange={(value) =>
													setOverrides((prev) => ({
														...prev,
														[prop.id]: value,
													}))
												}
												onReset={() =>
													setOverrides((prev) => {
														const { [prop.id]: _dropped, ...rest } = prev;
														return rest;
													})
												}
											/>
										))}
									</div>
								))
							)}
						</div>
					</ScrollArea>

					<div className="flex shrink-0 items-center gap-2 border-t px-4 py-2.5">
						<Button
							variant="outline"
							size="sm"
							className="h-7 text-[11.5px]"
							disabled={overrideCount === 0}
							onClick={() => setOverrides({})}
						>
							<RotateCcw className="mr-1.5 h-3 w-3" />
							{t('resetPreview', 'Reset preview')}
						</Button>
						{overrideCount > 0 && (
							<span className="font-mono text-[10.5px] text-muted-foreground tabular-nums">{t('overridecountChanged', '{{overrideCount}} changed', { overrideCount })}</span>
						)}
					</div>

					{actions.length > 0 && (
						<div className="shrink-0 border-t px-4 py-3">
							<h3 className="pb-2 text-[9.5px] font-semibold uppercase tracking-[0.09em] text-muted-foreground">
								{t('actions', 'Actions')}
							</h3>
							{actions.map((action) => (
								<div
									key={action.id}
									className="flex items-baseline gap-2 py-1.5 text-[12px]"
								>
									<Zap className="h-3 w-3 shrink-0 translate-y-0.5 text-muted-foreground" />
									<span className="font-mono text-[11.5px]">
										{action.label}
									</span>
									<span className="flex-1" />
									{action.contextSchema.length > 0 && (
										<span className="font-mono text-[10.5px] text-muted-foreground">
											{action.contextSchema.map((f) => f.name).join(", ")}
										</span>
									)}
								</div>
							))}
							<p className="pt-1.5 text-[11px] leading-snug text-muted-foreground">
								{t('eachPageDecidesWhatTheseRunWhenItPlacesTheWidget', 'Each page decides what these run when it places the widget.')}
							</p>
						</div>
					)}
				</aside>
			</div>

			<EditDetailsSheet
				open={isEditOpen}
				onOpenChange={setIsEditOpen}
				appId={appId}
				initial={{
					name,
					description,
					long_description: longDescription,
					tags,
				}}
				onSave={handleSaveMeta}
			/>
		</main>
	);
}

function ViewportButton({
	active,
	onClick,
	children,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	children: React.ReactNode;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			className={cn(
				"h-full border-l px-2 font-mono text-[10.5px] text-muted-foreground transition-colors first:border-l-0 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
				active && "bg-muted text-foreground",
			)}
		>
			{children}
		</button>
	);
}

function StripItem({
	label,
	children,
}: Readonly<{ label: string; children: React.ReactNode }>) {
	return (
		<div className="min-w-0">
			<div className="pb-1 text-[9.5px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
				{label}
			</div>
			{children}
		</div>
	);
}

function Fact({
	label,
	children,
}: Readonly<{ label: string; children: React.ReactNode }>) {
	return (
		<div>
			<div className="pb-1 text-[9.5px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
				{label}
			</div>
			<div className="font-mono text-[12.5px] tabular-nums">{children}</div>
		</div>
	);
}

function AddLink({
	onClick,
	children,
}: Readonly<{ onClick: () => void; children: React.ReactNode }>) {
	return (
		<button
			type="button"
			onClick={onClick}
			className="inline-flex items-center gap-1 text-[12.5px] text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			<Plus className="h-3 w-3" />
			{children}
		</button>
	);
}

/** Walks `propertyPath` the same way applyExposedProps writes it, so the control starts where the widget is. */
function readPropValue(
	components: SurfaceComponent[],
	prop: ExposedProp,
): unknown {
	const target = components.find(
		(component) => component.id === prop.targetComponentId,
	);
	if (!target) return undefined;
	const parts = prop.propertyPath.split(".").filter(Boolean);
	if (parts.length === 0) return undefined;

	const targetRecord = target as unknown as {
		component?: Record<string, unknown>;
	};
	let cursor: unknown =
		parts[0] === "style"
			? (target as unknown as Record<string, unknown>)
			: (targetRecord.component ?? {});
	for (const part of parts) {
		if (typeof cursor !== "object" || cursor === null) return undefined;
		cursor = (cursor as Record<string, unknown>)[part];
	}
	return cursor;
}

/** BoundValue wrappers carry either a literal or a data path. */
function unwrapValue(value: unknown): { literal: unknown; path?: string } {
	if (typeof value === "object" && value !== null) {
		const wrapper = value as Record<string, unknown>;
		if ("literalString" in wrapper) return { literal: wrapper.literalString };
		if ("literalNumber" in wrapper) return { literal: wrapper.literalNumber };
		if ("literalBool" in wrapper) return { literal: wrapper.literalBool };
		if ("path" in wrapper) {
			return {
				literal: wrapper.defaultValue,
				path: String(wrapper.path ?? ""),
			};
		}
	}
	return { literal: value };
}

function typeLabel(propType: ExposedPropType): string {
	return typeof propType === "string" ? propType : "Enum";
}

function PropControl({
	prop,
	components,
	override,
	onChange,
	onReset,
}: Readonly<{
	prop: ExposedProp;
	components: SurfaceComponent[];
	override: unknown;
	onChange: (value: unknown) => void;
	onReset: () => void;
}>) {
	const { t } = useTranslation("common");
	const { literal, path } = useMemo(
		() => unwrapValue(readPropValue(components, prop)),
		[components, prop],
	);
	const value = override ?? literal;
	const isOverridden = override !== undefined;
	const label = typeLabel(prop.propType);
	const isBound = prop.propType === "BoundValue" || !!path;

	return (
		<div className="flex flex-col gap-1.5 py-2">
			<div className="flex items-center gap-2">
				<Label className="text-[12.5px] font-medium">{prop.label}</Label>
				{isOverridden && (
					<button
						type="button"
						onClick={onReset}
						title={t('resetToTheWidgetsOwnValue', 'Reset to the widget\'s own value')}
						className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						aria-label={t('resetLabel', 'Reset {{label}}', { label: prop.label })}
					/>
				)}
				<span className="flex-1" />
				<span
					className={cn(
						"rounded-sm border px-1.5 font-mono text-[9.5px]",
						isBound
							? "border-primary/40 text-primary"
							: "text-muted-foreground",
					)}
				>
					{label}
				</span>
			</div>

			{isBound ? (
				<div className="flex h-8 items-center gap-2 rounded-md border border-dashed border-primary/40 bg-primary/5 px-2.5 font-mono text-[11.5px] text-primary">
					<Link2 className="h-3 w-3 shrink-0" />
					<span className="truncate">{path || t('boundByThePage', 'bound by the page')}</span>
				</div>
			) : (
				<PropField prop={prop} value={value} onChange={onChange} />
			)}

			{prop.description && (
				<p className="text-[11px] leading-snug text-muted-foreground">
					{prop.description}
				</p>
			)}
			<p className="font-mono text-[10px] text-muted-foreground/80">
				{prop.propertyPath}
			</p>
		</div>
	);
}

function PropField({
	prop,
	value,
	onChange,
}: Readonly<{
	prop: ExposedProp;
	value: unknown;
	onChange: (value: unknown) => void;
}>) {
	if (typeof prop.propType === "object" && "Enum" in prop.propType) {
		const choices = prop.propType.Enum.choices;
		return (
			<Select value={String(value ?? "")} onValueChange={onChange}>
				<SelectTrigger className="h-8 text-[12.5px]">
					<SelectValue placeholder="Choose…" />
				</SelectTrigger>
				<SelectContent>
					{choices.map((choice) => (
						<SelectItem key={choice} value={choice}>
							{choice}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		);
	}

	switch (prop.propType) {
		case "Boolean":
			return (
				<div className="flex h-8 items-center">
					<Switch
						checked={Boolean(value)}
						onCheckedChange={(checked) => onChange(checked)}
						aria-label={prop.label}
					/>
				</div>
			);
		case "Number":
			return (
				<Input
					type="number"
					className="h-8 text-[12.5px]"
					value={value === undefined || value === null ? "" : String(value)}
					onChange={(e) => onChange(Number.parseFloat(e.target.value) || 0)}
				/>
			);
		case "Color":
			return (
				<div className="flex gap-2">
					<Input
						type="color"
						className="h-8 w-11 cursor-pointer p-1"
						value={typeof value === "string" ? value : "#000000"}
						onChange={(e) => onChange(e.target.value)}
					/>
					<Input
						className="h-8 flex-1 font-mono text-[12.5px]"
						placeholder="#000000"
						value={typeof value === "string" ? value : ""}
						onChange={(e) => onChange(e.target.value)}
					/>
				</div>
			);
		case "ImageUrl":
			return (
				<Input
					type="url"
					className="h-8 text-[12.5px]"
					placeholder="https://…"
					value={typeof value === "string" ? value : ""}
					onChange={(e) => onChange(e.target.value)}
				/>
			);
		case "Json":
		case "StyleObject":
			return (
				<Input
					className="h-8 font-mono text-[12px]"
					placeholder="{}"
					value={
						typeof value === "string" ? value : JSON.stringify(value ?? {})
					}
					onChange={(e) => {
						try {
							onChange(JSON.parse(e.target.value));
						} catch {
							onChange(e.target.value);
						}
					}}
				/>
			);
		default:
			return (
				<Input
					className={cn(
						"h-8 text-[12.5px]",
						prop.propType === "TailwindClass" && "font-mono text-[12px]",
					)}
					value={value === undefined || value === null ? "" : String(value)}
					onChange={(e) => onChange(e.target.value)}
				/>
			);
	}
}

function EditDetailsSheet({
	open,
	onOpenChange,
	appId,
	initial,
	onSave,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	appId: string;
	initial: EditState;
	onSave: (next: EditState) => Promise<void>;
}>) {
	const { t } = useTranslation("common");
	const [draft, setDraft] = useState<EditState>(initial);
	const [newTag, setNewTag] = useState("");
	const [isSaving, setIsSaving] = useState(false);

	const reseed = useCallback(
		(next: boolean) => {
			if (next) {
				setDraft(initial);
				setNewTag("");
			}
			onOpenChange(next);
		},
		[initial, onOpenChange],
	);

	const addTag = useCallback(() => {
		const tag = newTag.trim();
		if (!tag) return;
		setDraft((prev) =>
			prev.tags.includes(tag) ? prev : { ...prev, tags: [...prev.tags, tag] },
		);
		setNewTag("");
	}, [newTag]);

	const save = useCallback(async () => {
		setIsSaving(true);
		try {
			await onSave(draft);
		} catch (error) {
			console.error("Failed to save widget metadata:", error);
			toast.error("Could not save the widget details");
		} finally {
			setIsSaving(false);
		}
	}, [draft, onSave]);

	return (
		<Sheet open={open} onOpenChange={reseed}>
			<SheetContent
				side="right"
				className="flex w-full flex-col gap-0 p-0 sm:max-w-lg"
			>
				<SheetHeader className="border-b">
					<SheetTitle>{t('editDetails', 'Edit details')}</SheetTitle>
					<SheetDescription>
						{t('nameDescriptionAndNotesShownWhereverThisWidgetIsListed', 'Name, description and notes shown wherever this widget is listed.')}
					</SheetDescription>
				</SheetHeader>

				<ScrollArea className="min-h-0 flex-1">
					<div className="space-y-5 p-4">
						<div className="space-y-2">
							<Label htmlFor="widget-detail-name">Name</Label>
							<Input
								id="widget-detail-name"
								value={draft.name}
								onChange={(e) =>
									setDraft((prev) => ({ ...prev, name: e.target.value }))
								}
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="widget-detail-description">{t('description', 'Description')}</Label>
							<Textarea
								id="widget-detail-description"
								placeholder={t('oneSentenceOnWhatThisWidgetShowsAndWhenToUseIt', 'One sentence on what this widget shows and when to use it.')}
								className="min-h-20 resize-none"
								value={draft.description}
								onChange={(e) =>
									setDraft((prev) => ({ ...prev, description: e.target.value }))
								}
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="widget-detail-tag">{t('tags', 'Tags')}</Label>
							<div className="flex gap-2">
								<Input
									id="widget-detail-tag"
									placeholder={t('addATag', 'Add a tag…')}
									value={newTag}
									onChange={(e) => setNewTag(e.target.value)}
									onKeyDown={(e) => {
										if (e.key === "Enter") {
											e.preventDefault();
											addTag();
										}
									}}
								/>
								<Button variant="outline" onClick={addTag}>
									{t('add', 'Add')}
								</Button>
							</div>
							{draft.tags.length > 0 && (
								<div className="flex flex-wrap gap-1 pt-1">
									{draft.tags.map((tag) => (
										<Badge
											key={tag}
											variant="secondary"
											className="cursor-pointer"
											onClick={() =>
												setDraft((prev) => ({
													...prev,
													tags: prev.tags.filter((t) => t !== tag),
												}))
											}
										>{`${tag} ×`}</Badge>
									))}
								</div>
							)}
						</div>

						<div className="space-y-2">
							<Label>{t('notes', 'Notes')}</Label>
							<div className="min-h-[200px] rounded-md border">
								<TextEditor
									appId={appId}
									editable
									isMarkdown
									initialContent={
										draft.long_description || t('addDetailedNotes', '*Add detailed notes…*')
									}
									onChange={(content) =>
										setDraft((prev) => ({ ...prev, long_description: content }))
									}
								/>
							</div>
						</div>
					</div>
				</ScrollArea>

				<SheetFooter className="flex-row justify-end gap-2 border-t">
					<Button variant="outline" onClick={() => reseed(false)}>
						{t('cancel', 'Cancel')}
					</Button>
					<Button onClick={save} disabled={isSaving || !draft.name.trim()}>
						{isSaving ? (
							<Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
						) : (
							<Check className="mr-1.5 h-4 w-4" />
						)}
						{t('save', 'Save')}
					</Button>
				</SheetFooter>
			</SheetContent>
		</Sheet>
	);
}
