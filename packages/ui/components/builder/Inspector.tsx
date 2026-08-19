"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	type ContractInput,
	type WidgetContract,
	validateInputValue,
} from "@flow-like/widget-sdk";
import {
	ChevronDown,
	ChevronUp,
	Package,
	Plus,
	Trash2,
	Zap,
} from "lucide-react";
import {
	type CSSProperties,
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";
import { useInvoke } from "../../hooks";
import { cn } from "../../lib";
import {
	createContractInputValue,
	updateWidgetContractProps,
} from "../../lib/widget-contract-form";
import { homogeneousArrayItemSchema } from "../../lib/widget-schema-form";
import { useBackend } from "../../state/backend-state";
import {
	type ComponentEventDefinition,
	getComponentEventDefinitions,
} from "../a2ui/component-event-manifest";
import {
	NIVO_CHART_DEFAULTS,
	NIVO_SAMPLE_DATA,
} from "../a2ui/display/nivo-data";
import { WILDCARD_EVENT } from "../a2ui/event-handlers";
import { getModel3DView } from "../a2ui/game/model3d-view-registry";
import {
	inferFileName,
	inferFileType,
	inferMimeTypeFromSource,
} from "../a2ui/media-source";
import { normalizeStyleUpdate } from "../a2ui/style-updates";
import type {
	A2UIComponent,
	BoundValue,
	BreakpointStyle,
	ChartAxis,
	ChartSeries,
	ChartType,
	MicroWidgetInstanceComponent,
	Overflow,
	Position,
	ResponsiveOverrides,
	SelectOption,
	Shadow,
	Spacing,
	Style,
	StyleValue,
	SurfaceComponent,
	TableColumn,
} from "../a2ui/types";
import { Button } from "../ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { MonacoCodeEditor } from "../ui/monaco-code-editor";
import { ScrollArea } from "../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Slider } from "../ui/slider";
import { Switch } from "../ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { Textarea } from "../ui/textarea";
import { WidgetSchemaListEditor } from "../widget-contract/WidgetSchemaListEditor";
import { AssetPicker, type AssetPickerProps } from "./AssetPicker";
import { useBuilder } from "./BuilderContext";
import { getDefaultProps } from "./componentDefaults";
import { getComponentSchema } from "./componentSchema";

type AssetAccept = NonNullable<AssetPickerProps["accept"]>;

const FIXED_FIELD_SIZING_STYLE = { fieldSizing: "fixed" } as CSSProperties;
const INSPECTOR_FIELD_CLASS = "space-y-2 min-w-0 max-w-full";

// Component/property pairs that should use the asset picker.
const ASSET_FIELDS_BY_COMPONENT: Record<
	string,
	Partial<Record<string, AssetAccept>>
> = {
	image: { src: "image", fallback: "image" },
	video: { src: "video", poster: "image" },
	filePreview: { src: "all", url: "all" },
	avatar: { src: "image" },
	lottie: { src: "animation" },
	iframe: { src: "all" },
	boundingBoxOverlay: { src: "image" },
	imageLabeler: { src: "image" },
	imageHotspot: { src: "image" },
	sprite: { src: "image" },
	model3d: { src: "model", hdriUrl: "environment" },
	scene3d: { environmentMap: "environment" },
	characterPortrait: { image: "image" },
	miniMap: { mapImage: "image" },
};

function getAssetAccept(
	componentType: string | undefined,
	propertyName: string,
): AssetAccept | undefined {
	if (!componentType) return undefined;
	return ASSET_FIELDS_BY_COMPONENT[componentType]?.[propertyName];
}

function literalString(value: string): BoundValue {
	return { literalString: value };
}

function getLiteralAssetPath(value: unknown): string | undefined {
	if (typeof value === "string") return value;
	if (typeof value === "object" && value !== null && "literalString" in value) {
		return String(value.literalString);
	}
	return undefined;
}

function getStyleValue(value: StyleValue | undefined): string {
	if (!value) return "";
	return typeof value === "string" ? value : value.value;
}

function getSpacingValue(spacing: Spacing | undefined): string {
	if (!spacing) return "";
	if ("value" in spacing) return spacing.value ?? "";
	if (!spacing.top && !spacing.right && !spacing.bottom && !spacing.left) {
		return "";
	}
	return [spacing.top, spacing.right, spacing.bottom, spacing.left]
		.map((value) => value || "0")
		.join(" ");
}

function getSpacingEdges(spacing: Spacing | undefined): {
	top?: string;
	right?: string;
	bottom?: string;
	left?: string;
} {
	if (!spacing) return {};
	if (!("value" in spacing)) return spacing;
	const parts = (spacing.value ?? "").trim().split(/\s+/).filter(Boolean);
	if (parts.length === 0) return {};
	const [top, right = top, bottom = top, left = right] = parts;
	return { top, right, bottom, left };
}

function withSpacingSide(
	spacing: Spacing | undefined,
	side: "top" | "right" | "bottom" | "left",
	value: string,
): Spacing {
	return { ...getSpacingEdges(spacing), [side]: value || undefined };
}

function spacingFromShorthand(value: string): Spacing | undefined {
	if (!value.trim()) return undefined;
	return getSpacingEdges({ value });
}

function getPositionType(
	position: Position | undefined,
): NonNullable<Position["type"]> {
	return position?.type ?? position?.positionType ?? "relative";
}

function withPositionType(
	position: Position | undefined,
	positionType: NonNullable<Position["type"]>,
): Position {
	return {
		top: position?.top,
		right: position?.right,
		bottom: position?.bottom,
		left: position?.left,
		type: positionType,
	};
}

function withoutLegacyBoxShadow(shadow: Shadow | undefined): Shadow {
	if (!shadow) return {};
	const { boxShadows: _boxShadows, ...canonicalShadow } = shadow;
	return canonicalShadow;
}

export interface InspectorProps {
	className?: string;
}

// Helper to get component type from SurfaceComponent
function getComponentType(sc: SurfaceComponent): string {
	return sc.component?.type ?? "Unknown";
}

export function Inspector({ className }: InspectorProps) {
	const { t } = useTranslation("flow");
	const { selection, components, updateComponent, getComponent } = useBuilder();

	const selectedComponents = useMemo(
		() =>
			selection.componentIds
				.map((id) => getComponent(id))
				.filter((c): c is SurfaceComponent => !!c),
		[selection.componentIds, getComponent],
	);

	const singleSelected =
		selectedComponents.length === 1 ? selectedComponents[0] : null;

	if (selectedComponents.length === 0) {
		return (
			<div
				className={cn("flex flex-col h-full bg-background border-l", className)}
			>
				<div className="p-4 border-b">
					<h3 className="font-medium text-sm">{t("inspector", "Inspector")}</h3>
				</div>
				<div className="flex-1 flex items-center justify-center p-4 text-sm text-muted-foreground">
					{t("selectAComponentToEdit", "Select a component to edit")}
				</div>
			</div>
		);
	}

	if (selectedComponents.length > 1) {
		return (
			<div
				className={cn("flex flex-col h-full bg-background border-l", className)}
			>
				<div className="p-4 border-b">
					<h3 className="font-medium text-sm">{t("inspector", "Inspector")}</h3>
					<p className="text-xs text-muted-foreground mt-1">
						{t("countComponentsSelected", {
							defaultValue_one: "{{count}} component selected",
							defaultValue_other: "{{count}} components selected",
							count: selectedComponents.length,
						})}
					</p>
				</div>
				<div className="p-4">
					<p className="text-sm text-muted-foreground">
						{t(
							"multiselectionEditingComingSoon",
							"Multi-selection editing coming soon",
						)}
					</p>
				</div>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex flex-col h-full bg-background border-l overflow-hidden",
				className,
			)}
		>
			<div className="p-4 border-b shrink-0">
				<h3 className="font-medium text-sm truncate">
					{singleSelected ? getComponentType(singleSelected) : ""}
				</h3>
				<p className="text-xs text-muted-foreground truncate mt-0.5">
					{t("id", "ID:")} {singleSelected?.id}
				</p>
			</div>

			<Tabs
				defaultValue="properties"
				className="flex-1 flex flex-col min-h-0 overflow-hidden"
			>
				<TabsList className="w-full justify-start rounded-none border-b bg-transparent p-0 shrink-0">
					<TabsTrigger
						value="properties"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						{t("props", "Props")}
					</TabsTrigger>
					<TabsTrigger
						value="style"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						{t("style", "Style")}
					</TabsTrigger>
					<TabsTrigger
						value="canvas"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						{t("canvas", "Canvas")}
					</TabsTrigger>
					<TabsTrigger
						value="actions"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						{t("actions", "Actions")}
					</TabsTrigger>
				</TabsList>

				<ScrollArea
					className="flex-1 min-h-0 min-w-0 max-w-full overflow-hidden"
					viewportClassName="min-w-0 max-w-full overflow-x-hidden"
				>
					<TabsContent
						value="properties"
						className="m-0 min-w-0 max-w-full p-4"
					>
						{singleSelected && (
							<PropertyEditor
								component={singleSelected}
								onUpdate={(updates) =>
									updateComponent(singleSelected.id, updates)
								}
							/>
						)}
					</TabsContent>

					<TabsContent value="style" className="m-0 min-w-0 max-w-full p-4">
						{singleSelected && (
							<StyleEditor
								component={singleSelected}
								onUpdate={(updates) =>
									updateComponent(singleSelected.id, updates)
								}
							/>
						)}
					</TabsContent>

					<TabsContent value="canvas" className="m-0 min-w-0 max-w-full p-4">
						<CanvasSettingsEditor />
					</TabsContent>

					<TabsContent value="actions" className="m-0 min-w-0 max-w-full p-4">
						{singleSelected && (
							<ActionsEditor
								component={singleSelected}
								onUpdate={(updates) =>
									updateComponent(singleSelected.id, updates)
								}
							/>
						)}
					</TabsContent>
				</ScrollArea>
			</Tabs>
		</div>
	);
}

interface PropertyEditorProps {
	component: SurfaceComponent;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
}

function PropertyEditor({ component, onUpdate }: PropertyEditorProps) {
	const { t } = useTranslation("flow");
	const rawProps = (component.component ?? {}) as unknown as Record<
		string,
		unknown
	>;
	const componentType = component.component?.type;
	const props =
		componentType === "model3d"
			? { ...getDefaultProps("model3d"), ...rawProps }
			: rawProps;
	const schema = componentType ? getComponentSchema(componentType) : undefined;

	const updateProp = useCallback(
		(key: string, value: unknown) => {
			onUpdate({
				component: {
					...component.component,
					[key]: value,
				} as SurfaceComponent["component"],
			});
		},
		[component.component, onUpdate],
	);

	const updateProperty = useCallback(
		(key: string, value: unknown) => {
			const literalAssetPath = getLiteralAssetPath(value);

			if (
				componentType === "filePreview" &&
				(key === "src" || key === "url") &&
				literalAssetPath !== undefined
			) {
				const fileType = inferFileType(undefined, undefined, literalAssetPath);
				const metadata: Record<string, unknown> = {
					[key]: value,
					filename: literalString(inferFileName(literalAssetPath)),
					mimeType: literalString(inferMimeTypeFromSource(literalAssetPath)),
					fileType: literalString(fileType),
				};

				if (key === "src") {
					metadata.url = value;
				} else {
					metadata.src = value;
				}

				onUpdate({
					component: {
						...component.component,
						...metadata,
					} as SurfaceComponent["component"],
				});
				return;
			}

			updateProp(key, value);
		},
		[component.component, componentType, onUpdate, updateProp],
	);

	// Special editor for PlotlyChart
	if (componentType === "plotlyChart") {
		return (
			<ChartEditor
				component={component}
				props={props}
				onUpdate={onUpdate}
				updateProp={updateProp}
			/>
		);
	}

	// Special editor for NivoChart
	if (componentType === "nivoChart") {
		return (
			<NivoChartEditor
				component={component}
				props={props}
				onUpdate={onUpdate}
				updateProp={updateProp}
			/>
		);
	}

	// Special editor for Table
	if (componentType === "table") {
		return (
			<TableEditor
				component={component}
				props={props}
				onUpdate={onUpdate}
				updateProp={updateProp}
			/>
		);
	}

	if (componentType === "model3d") {
		return (
			<Model3DEditor
				component={component}
				props={props}
				onUpdate={onUpdate}
				updateProp={updateProp}
			/>
		);
	}

	// Package micro widgets: code widgets are not editable as a component tree.
	// Contract events are configured in the Actions tab.
	if (componentType === "microWidgetInstance") {
		return <MicroWidgetEditor component={component} onUpdate={onUpdate} />;
	}

	// Render different editors based on component type
	return (
		<div className="min-w-0 max-w-full space-y-4">
			{/* Common ID field */}
			<div className={INSPECTOR_FIELD_CLASS}>
				<Label className="text-xs">{t("componentId", "Component ID")}</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			<PropertyField
				name="hidden"
				value={props.hidden ?? { literalBool: false }}
				onChange={(newValue) => updateProperty("hidden", newValue)}
				componentType={componentType}
			/>

			{/* Type-specific properties */}
			{Object.entries(props).map(([key, value]) => {
				if (key === "hidden") return null;
				const assetAccept = getAssetAccept(componentType, key);
				return (
					<PropertyField
						key={key}
						name={key}
						value={value}
						onChange={(newValue) => updateProperty(key, newValue)}
						isAssetProperty={Boolean(assetAccept)}
						assetAccept={assetAccept}
						componentType={componentType}
						enumOptions={schema?.[key]?.enum}
					/>
				);
			})}
		</div>
	);
}

interface JsonContractFieldProps {
	id: string;
	input: ContractInput;
	value: unknown;
	disabled: boolean;
	labelledBy: string;
	describedBy?: string;
	onCommit: (value: unknown) => void;
}

// Commit-on-blur JSON editor so re-serialization never fights the caret.
function JsonContractField({
	id,
	input,
	value,
	disabled,
	labelledBy,
	describedBy,
	onCommit,
}: JsonContractFieldProps) {
	const { t } = useTranslation("flow");
	const serialized = useMemo(
		() => JSON.stringify(value ?? null, null, 2),
		[value],
	);
	const [text, setText] = useState(serialized);
	const [errors, setErrors] = useState<string[]>([]);
	const errorId = `${id}-draft-error`;

	useEffect(() => {
		setText(serialized);
		setErrors([]);
	}, [serialized]);

	const commit = useCallback(() => {
		try {
			const parsed = JSON.parse(text);
			const validation = validateInputValue(input, parsed);
			if (!validation.valid) {
				setErrors(validation.errors);
				return;
			}
			setErrors([]);
			if (JSON.stringify(parsed) !== JSON.stringify(value ?? null)) {
				onCommit(parsed);
			}
		} catch (error) {
			setErrors([
				t("invalidJsonVal", "Invalid JSON: {{val}}", {
					val: error instanceof Error ? error.message : String(error),
				}),
			]);
		}
	}, [input, text, value, onCommit]);

	return (
		<div className="space-y-1">
			<Textarea
				id={id}
				value={text}
				onChange={(e) => setText(e.target.value)}
				onBlur={commit}
				disabled={disabled}
				rows={4}
				spellCheck={false}
				aria-labelledby={labelledBy}
				aria-invalid={errors.length > 0}
				aria-describedby={
					[describedBy, errors.length > 0 ? errorId : undefined]
						.filter(Boolean)
						.join(" ") || undefined
				}
				className={cn(
					"font-mono text-xs",
					errors.length > 0 &&
						"border-destructive focus-visible:ring-destructive",
				)}
				style={FIXED_FIELD_SIZING_STYLE}
			/>
			{errors.length > 0 && (
				<div id={errorId} className="space-y-0.5" role="alert">
					{[...new Set(errors)].map((error) => (
						<p key={error} className="text-[10px] text-destructive">
							{error.replace(/^\$(?:\.|:\s*)?/, "")}
						</p>
					))}
				</div>
			)}
		</div>
	);
}

interface ContractInputFieldProps {
	name: string;
	input: ContractInput;
	value: unknown;
	present: boolean;
	onChange: (value: unknown) => void;
}

function enumOptionValue(choice: string): string {
	return JSON.stringify(choice);
}

function ContractInputField({
	name,
	input,
	value,
	present,
	onChange,
}: ContractInputFieldProps) {
	const { t } = useTranslation("flow");
	const id = useId();
	const labelId = `${id}-label`;
	const descriptionId = input.description ? `${id}-description` : undefined;
	const validation = validateInputValue(input, present ? value : undefined);
	const [draftErrors, setDraftErrors] = useState<string[]>([]);
	const controlValue = present ? value : createContractInputValue(input);
	const listItemSchema =
		input.type === "json" ? homogeneousArrayItemSchema(input.schema) : null;
	const isList = listItemSchema !== null && Array.isArray(controlValue);
	const disabled = input.optional === true && !present;
	const errorId = `${id}-error`;
	const errors = [...new Set([...validation.errors, ...draftErrors])];
	const describedBy = [descriptionId, errors.length > 0 ? errorId : undefined]
		.filter(Boolean)
		.join(" ");

	const renderControl = () => {
		switch (input.type) {
			case "boolean":
				return (
					<Switch
						id={id}
						checked={controlValue === true}
						onCheckedChange={(checked) => onChange(checked)}
						disabled={disabled}
						aria-invalid={errors.length > 0}
						aria-describedby={describedBy || undefined}
					/>
				);
			case "number":
			case "integer":
				return (
					<Input
						id={id}
						type="number"
						className="h-8 text-sm"
						value={typeof controlValue === "number" ? controlValue : ""}
						min={
							input.min === undefined
								? undefined
								: input.type === "integer"
									? Math.ceil(input.min)
									: input.min
						}
						max={
							input.max === undefined
								? undefined
								: input.type === "integer"
									? Math.floor(input.max)
									: input.max
						}
						step={input.type === "integer" ? 1 : "any"}
						disabled={disabled}
						aria-invalid={errors.length > 0}
						aria-describedby={describedBy || undefined}
						onChange={(e) => {
							if (e.target.value === "") {
								setDraftErrors(["$: value is required"]);
								return;
							}
							const parsed = Number(e.target.value);
							const nextValidation = validateInputValue(input, parsed);
							setDraftErrors(nextValidation.errors);
							if (nextValidation.valid) onChange(parsed);
						}}
					/>
				);
			case "enum":
				return (
					<Select
						value={
							typeof controlValue === "string"
								? enumOptionValue(controlValue)
								: undefined
						}
						onValueChange={(encoded) => onChange(JSON.parse(encoded))}
						disabled={disabled}
					>
						<SelectTrigger
							id={id}
							className="h-8 text-sm"
							aria-invalid={errors.length > 0}
							aria-describedby={describedBy || undefined}
						>
							<SelectValue placeholder="Select…" />
						</SelectTrigger>
						<SelectContent>
							{[...new Set(input.choices ?? [])].map((choice) => (
								<SelectItem
									key={enumOptionValue(choice)}
									value={enumOptionValue(choice)}
								>
									{choice || "Empty string"}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				);
			case "json": {
				if (input.schema && isList) {
					return (
						<WidgetSchemaListEditor
							fieldName={name}
							id={id}
							labelledBy={labelId}
							schema={input.schema}
							value={controlValue}
							disabled={disabled}
							describedBy={describedBy || undefined}
							onChange={(nextValue) => {
								const nextValidation = validateInputValue(input, nextValue);
								setDraftErrors(nextValidation.errors);
								const onlyNeedsMoreItems =
									!nextValidation.valid &&
									nextValidation.errors.length > 0 &&
									nextValidation.errors.every((error) =>
										error.includes("array has fewer than minItems"),
									);
								if (nextValidation.valid || onlyNeedsMoreItems)
									onChange(nextValue);
							}}
						/>
					);
				}
				return (
					<JsonContractField
						id={id}
						input={input}
						value={controlValue}
						disabled={disabled}
						labelledBy={labelId}
						describedBy={describedBy || undefined}
						onCommit={onChange}
					/>
				);
			}
			default:
				return (
					<Input
						id={id}
						className="h-8 text-sm"
						value={typeof controlValue === "string" ? controlValue : ""}
						disabled={disabled}
						aria-invalid={errors.length > 0}
						aria-describedby={describedBy || undefined}
						onChange={(e) => onChange(e.target.value)}
					/>
				);
		}
	};

	return (
		<div className={INSPECTOR_FIELD_CLASS}>
			<div className="flex items-center justify-between gap-2">
				{isList ? (
					<span id={labelId} className="text-xs font-medium">
						{name}
					</span>
				) : (
					<Label id={labelId} htmlFor={id} className="text-xs font-medium">
						{name}
					</Label>
				)}
				{input.optional && (
					<span className="text-[10px] text-muted-foreground">optional</span>
				)}
			</div>
			{input.description && (
				<p
					id={descriptionId}
					className="text-[10px] leading-4 text-muted-foreground"
				>
					{input.description}
				</p>
			)}
			{input.optional && (
				<div className="flex items-center gap-2">
					<Switch
						id={`${id}-included`}
						checked={present}
						onCheckedChange={(included) =>
							onChange(included ? createContractInputValue(input) : undefined)
						}
					/>
					<Label
						htmlFor={`${id}-included`}
						className="text-[10px] font-normal text-muted-foreground"
					>
						{t("includeValue", "Include value")}
					</Label>
				</div>
			)}
			{!input.optional && !present && !isList ? (
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() => onChange(controlValue)}
				>
					{t("setValue", "Set value")}
				</Button>
			) : (
				renderControl()
			)}
			{errors.length > 0 && (
				<div id={errorId} className="space-y-0.5" role="alert">
					{errors.map((error) => (
						<p key={error} className="text-[10px] text-destructive">
							{error.replace(/^\$(?:\.|:\s*)?/, "")}
						</p>
					))}
				</div>
			)}
		</div>
	);
}

interface MicroWidgetEditorProps {
	component: SurfaceComponent;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
}

/**
 * Inspector for package micro widgets: provenance header, contract-typed
 * input controls writing static values into `component.props`, plus the
 * contract's read-only queries. Contract events live in the Actions tab.
 */
function MicroWidgetEditor({ component, onUpdate }: MicroWidgetEditorProps) {
	const { t } = useTranslation("flow");
	const micro = component.component as unknown as MicroWidgetInstanceComponent;
	const contract = (micro.contract ?? null) as WidgetContract | null;
	const props = micro.props ?? {};
	const inputs = useMemo(
		() => Object.entries(contract?.inputs ?? {}),
		[contract],
	);
	const queries = useMemo(
		() => Object.entries(contract?.queries ?? {}),
		[contract],
	);

	const setProp = useCallback(
		(key: string, value: unknown) => {
			const current =
				(component.component as unknown as MicroWidgetInstanceComponent)
					.props ?? {};
			onUpdate({
				component: {
					...component.component,
					props: updateWidgetContractProps(current, key, value),
				} as SurfaceComponent["component"],
			});
		},
		[component.component, onUpdate],
	);

	return (
		<div className="min-w-0 max-w-full space-y-4">
			<div className="space-y-1 rounded-md border bg-muted/40 p-3 dark:border-white/15">
				<div className="flex items-center gap-1.5 text-xs font-medium">
					<Package className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
					<span className="truncate" title={micro.packageId}>
						{micro.packageId}
						{micro.packageVersion ? `@${micro.packageVersion}` : ""}
					</span>
				</div>
				<p className="text-[10px] text-muted-foreground">
					{t(
						"widgetWidgetidRenderedInASandboxItsInternalsAreNotEditableInTheBuilder",
						"Widget “{{widgetId}}” — rendered in a sandbox; its internals are not editable in the builder.",
						{ widgetId: micro.widgetId },
					)}
				</p>
			</div>

			<div className={INSPECTOR_FIELD_CLASS}>
				<Label className="text-xs">{t("componentId", "Component ID")}</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			<div className="space-y-3">
				<Label className="text-xs font-semibold">{t("inputs", "Inputs")}</Label>
				{inputs.length === 0 ? (
					<p className="text-xs text-muted-foreground">
						{t("thisWidgetDeclaresNoInputs", "This widget declares no inputs.")}
					</p>
				) : (
					inputs.map(([name, input]) => (
						<ContractInputField
							key={`${contract?.id ?? "widget"}-${name}`}
							name={name}
							input={input}
							value={props[name]}
							present={Object.hasOwn(props, name)}
							onChange={(value) => setProp(name, value)}
						/>
					))
				)}
			</div>

			{queries.length > 0 && (
				<div className="space-y-2">
					<Label className="text-xs font-semibold">
						{t("queries", "Queries")}
					</Label>
					<p className="text-[10px] text-muted-foreground">
						{`Callable from flows by connecting Get Element to Query Widget.`}
					</p>
					{queries.map(([name, spec]) => (
						<div
							key={name}
							className="rounded-md border px-2.5 py-1.5 dark:border-white/10"
						>
							<span className="text-xs font-medium">{name}</span>
							{spec.description && (
								<p className="mt-0.5 text-[10px] text-muted-foreground">
									{spec.description}
								</p>
							)}
						</div>
					))}
				</div>
			)}
		</div>
	);
}

// Schema enum options for Model3D fields
const MODEL3D_ENUMS = {
	cameraAngle: ["front", "side", "top", "isometric"],
	lightingPreset: ["neutral", "warm", "cool", "studio", "dramatic"],
	environment: [
		"studio",
		"sunset",
		"dawn",
		"night",
		"warehouse",
		"forest",
		"apartment",
		"city",
		"park",
		"lobby",
	],
	environmentSource: ["local", "preset", "polyhaven", "custom"],
	polyhavenHdri: [
		"studio_small_03",
		"studio_small_09",
		"brown_photostudio_02",
		"empty_warehouse_01",
		"industrial_sunset_02",
		"sunset_in_the_chalk_quarry",
		"rooftop_night",
		"abandoned_factory_canteen_01",
		"forest_slope",
		"green_point_park",
		"lebombo",
		"spruit_sunrise",
		"syferfontein_18d_clear_puresky",
		"venice_sunset",
		"potsdamer_platz",
	],
	polyhavenResolution: ["1k", "2k", "4k", "8k"],
} as const;

interface Model3DEditorProps {
	component: SurfaceComponent;
	props: Record<string, unknown>;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
	updateProp: (key: string, value: unknown) => void;
}

function Model3DEditor({
	component,
	props,
	onUpdate,
	updateProp,
}: Model3DEditorProps) {
	const { t } = useTranslation("flow");
	const getBoundValue = (key: string): BoundValue | undefined => {
		const value = props[key];
		return typeof value === "object" && value !== null
			? (value as BoundValue)
			: undefined;
	};

	const parseVector3 = (
		value: BoundValue | undefined,
		fallback: [number, number, number],
	) => {
		if (!value) return fallback;
		if ("literalJson" in value) {
			try {
				const parsed = JSON.parse(String(value.literalJson));
				if (Array.isArray(parsed) && parsed.length === 3) {
					return parsed as [number, number, number];
				}
			} catch {
				return fallback;
			}
		}
		return fallback;
	};

	const vectorToBound = (vec: [number, number, number]): BoundValue => ({
		literalJson: JSON.stringify(vec),
	});

	const isBoundPath = (
		value: BoundValue | undefined,
	): value is BoundValue & { path: string } =>
		Boolean(value && "path" in value);

	const toDeg = (rad: number) => (rad * 180) / Math.PI;
	const toRad = (deg: number) => (deg * Math.PI) / 180;

	const renderVectorField = (
		key: string,
		value: BoundValue | undefined,
		fallback: [number, number, number],
		min: number,
		max: number,
		step: number,
		useDegrees = false,
		displayLabel?: string,
	) => {
		if (isBoundPath(value)) {
			return (
				<BoundValueEditor
					name={key}
					label={displayLabel}
					value={value}
					onChange={(newValue) => updateProp(key, newValue)}
					componentType="model3d"
				/>
			);
		}

		const vec = parseVector3(value, fallback);
		const display = useDegrees
			? ([toDeg(vec[0]), toDeg(vec[1]), toDeg(vec[2])] as [
					number,
					number,
					number,
				])
			: vec;

		const updateAxis = (index: number, newValue: number) => {
			const next = [...display] as [number, number, number];
			next[index] = newValue;
			const stored = useDegrees
				? ([toRad(next[0]), toRad(next[1]), toRad(next[2])] as [
						number,
						number,
						number,
					])
				: next;
			updateProp(key, vectorToBound(stored));
		};

		const axisColors = [
			"text-red-400",
			"text-green-400",
			"text-blue-400",
		] as const;

		return (
			<div className="space-y-1.5">
				<Label className="text-xs text-muted-foreground">
					{displayLabel ?? key}
				</Label>
				<div className="space-y-1">
					{(["X", "Y", "Z"] as const).map((axis, index) => (
						<div key={axis} className="flex items-center gap-1.5">
							<span
								className={cn(
									"w-3.5 text-[10px] font-medium",
									axisColors[index],
								)}
							>
								{axis}
							</span>
							<Slider
								value={[display[index]]}
								min={min}
								max={max}
								step={step}
								onValueChange={(v) => updateAxis(index, v[0] ?? 0)}
								className="flex-1"
							/>
							<Input
								type="number"
								value={display[index].toFixed(step < 1 ? 2 : 0)}
								onChange={(e) => updateAxis(index, e.target.valueAsNumber || 0)}
								className="w-16 h-6 text-[11px] text-center px-1"
							/>
						</div>
					))}
				</div>
			</div>
		);
	};

	const renderNumberField = (
		key: string,
		value: BoundValue | undefined,
		fallback: number,
		min: number,
		max: number,
		step: number,
		displayLabel?: string,
	) => {
		if (isBoundPath(value)) {
			return (
				<BoundValueEditor
					name={key}
					label={displayLabel}
					value={value}
					onChange={(newValue) => updateProp(key, newValue)}
					componentType="model3d"
				/>
			);
		}
		const current =
			value && "literalNumber" in value ? value.literalNumber : fallback;
		return (
			<div className="space-y-1.5">
				<Label className="text-xs text-muted-foreground">
					{displayLabel ?? key}
				</Label>
				<div className="flex items-center gap-1.5">
					<Slider
						value={[current]}
						min={min}
						max={max}
						step={step}
						onValueChange={(v) =>
							updateProp(key, { literalNumber: v[0] ?? fallback })
						}
						className="flex-1"
					/>
					<Input
						type="number"
						value={current.toFixed(step < 1 ? 2 : 0)}
						onChange={(e) =>
							updateProp(key, {
								literalNumber: e.target.valueAsNumber || fallback,
							})
						}
						className="w-16 h-6 text-[11px] text-center px-1"
					/>
				</div>
			</div>
		);
	};

	const renderToggle = (
		key: string,
		defaultValue: boolean,
		displayLabel: string,
	) => {
		const value = getBoundValue(key);
		const isPath = value && "path" in value;
		const current =
			value && "literalBool" in value ? value.literalBool : defaultValue;

		if (isPath) {
			return (
				<BoundValueEditor
					name={key}
					label={displayLabel}
					value={value}
					onChange={(v) => updateProp(key, v)}
					componentType="model3d"
				/>
			);
		}

		return (
			<div className="flex items-center justify-between py-0.5">
				<Label className="text-xs text-muted-foreground">{displayLabel}</Label>
				<Switch
					checked={current}
					onCheckedChange={(checked) =>
						updateProp(key, { literalBool: checked })
					}
					className="scale-75"
				/>
			</div>
		);
	};

	const renderSelect = (
		key: string,
		defaultValue: string,
		displayLabel: string,
		options: readonly string[],
	) => {
		const value = getBoundValue(key);
		const isPath = value && "path" in value;
		const current =
			value && "literalString" in value ? value.literalString : defaultValue;

		if (isPath) {
			return (
				<BoundValueEditor
					name={key}
					label={displayLabel}
					value={value}
					onChange={(v) => updateProp(key, v)}
					componentType="model3d"
					enumOptions={[...options]}
				/>
			);
		}

		return (
			<div className="space-y-1">
				<Label className="text-xs text-muted-foreground">{displayLabel}</Label>
				<Select
					value={current}
					onValueChange={(v) => updateProp(key, { literalString: v })}
				>
					<SelectTrigger className="h-7 text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{options.map((opt) => (
							<SelectItem key={opt} value={opt} className="text-xs">
								{opt.replace(/_/g, " ")}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
		);
	};

	const cameraPosition = getBoundValue("cameraPosition");
	const cameraTarget = getBoundValue("cameraTarget");
	const view = getModel3DView(component.id);

	const scaleValue = getBoundValue("scale");
	const scaleMode =
		scaleValue && "literalJson" in scaleValue ? "xyz" : "uniform";

	const section = (
		title: string,
		icon: ReactNode,
		children: ReactNode,
		defaultOpen = true,
	) => (
		<Collapsible defaultOpen={defaultOpen} className="group">
			<CollapsibleTrigger className="flex w-full items-center gap-2 rounded-md bg-muted/50 px-2.5 py-1.5 text-xs font-medium hover:bg-muted transition-colors">
				<span className="text-muted-foreground">{icon}</span>
				<span className="flex-1 text-left">{title}</span>
				<ChevronDown className="h-3.5 w-3.5 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
			</CollapsibleTrigger>
			<CollapsibleContent className="px-1 pt-3 pb-1 space-y-2.5">
				{children}
			</CollapsibleContent>
		</Collapsible>
	);

	// Icons for sections
	const icons = {
		transform: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<path d="M12 3v18M3 12h18M7.5 7.5l9 9M16.5 7.5l-9 9" />
			</svg>
		),
		camera: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<circle cx="12" cy="12" r="3" />
				<path d="M2 12h3M19 12h3M12 2v3M12 19v3" />
			</svg>
		),
		lighting: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<circle cx="12" cy="12" r="5" />
				<path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
			</svg>
		),
		environment: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<circle cx="12" cy="12" r="10" />
				<path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
			</svg>
		),
		ground: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<path d="M2 20h20M6 16l6-8 6 8" />
			</svg>
		),
		viewer: (
			<svg
				className="w-3.5 h-3.5"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
			>
				<rect x="2" y="3" width="20" height="14" rx="2" />
				<path d="M8 21h8M12 17v4" />
			</svg>
		),
	};

	return (
		<div className="space-y-3">
			<div className="space-y-1.5">
				<Label className="text-xs text-muted-foreground">
					{t("componentId", "Component ID")}
				</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-7 text-xs"
				/>
			</div>

			{section(
				"Transform",
				icons.transform,
				<>
					{renderVectorField(
						"position",
						getBoundValue("position"),
						[0, 0, 0],
						-10,
						10,
						0.1,
						false,
						"Position",
					)}
					{renderVectorField(
						"rotation",
						getBoundValue("rotation"),
						[0, 0, 0],
						-180,
						180,
						1,
						true,
						"Rotation (°)",
					)}
					<div className="space-y-1.5">
						<div className="flex items-center justify-between">
							<Label className="text-xs text-muted-foreground">
								{t("scale", "Scale")}
							</Label>
							<Select
								value={scaleMode}
								onValueChange={(mode) => {
									if (mode === "uniform") {
										const current = parseVector3(scaleValue, [1, 1, 1])[0];
										updateProp("scale", { literalNumber: current });
									} else {
										const current =
											scaleValue && "literalNumber" in scaleValue
												? scaleValue.literalNumber
												: 1;
										updateProp("scale", {
											literalJson: JSON.stringify([current, current, current]),
										});
									}
								}}
							>
								<SelectTrigger className="h-5 w-16 text-[10px]">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="uniform" className="text-xs">
										{t("uniform", "Uniform")}
									</SelectItem>
									<SelectItem value="xyz" className="text-xs">
										XYZ
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						{scaleMode === "uniform"
							? renderNumberField("scale", scaleValue, 1, 0.01, 10, 0.01)
							: renderVectorField(
									"scale",
									scaleValue,
									[1, 1, 1],
									0.01,
									10,
									0.01,
								)}
					</div>
					<div className="border-t pt-2 space-y-1">
						{renderToggle("castShadow", true, "Cast Shadow")}
						{renderToggle("receiveShadow", true, "Receive Shadow")}
					</div>
					<div className="border-t pt-2 space-y-1.5">
						{renderToggle(`autoRotate`, false, "Auto Rotate Model")}
						{renderNumberField(
							"rotateSpeed",
							getBoundValue("rotateSpeed"),
							1,
							0,
							10,
							0.1,
							"Rotation Speed",
						)}
					</div>
				</>,
			)}

			{section(
				"Camera",
				icons.camera,
				<>
					<div className="grid grid-cols-2 gap-2">
						{renderSelect(
							"cameraAngle",
							"front",
							t("anglePreset", "Angle Preset"),
							MODEL3D_ENUMS.cameraAngle,
						)}
						{renderNumberField(
							"cameraDistance",
							getBoundValue("cameraDistance"),
							3,
							0.1,
							50,
							0.1,
							"Distance",
						)}
					</div>
					{renderNumberField(
						"fov",
						getBoundValue("fov"),
						50,
						10,
						120,
						1,
						t("fieldOfView", "Field of View"),
					)}
					<div className="flex items-center justify-between py-1.5 border-y">
						<span className="text-xs text-muted-foreground">
							{t("captureCurrentView", "Capture current view")}
						</span>
						<Button
							size="sm"
							variant="secondary"
							className="h-6 text-[10px] px-2"
							disabled={!view}
							onClick={() => {
								if (!view) return;
								updateProp("cameraPosition", {
									literalJson: JSON.stringify(view.cameraPosition),
								});
								updateProp("cameraTarget", {
									literalJson: JSON.stringify(view.cameraTarget),
								});
							}}
						>
							{t("useView", "Use View")}
						</Button>
					</div>
					{renderVectorField(
						"cameraPosition",
						cameraPosition,
						[0, 0, 3],
						-50,
						50,
						0.1,
						false,
						"Camera Position",
					)}
					{renderVectorField(
						"cameraTarget",
						cameraTarget,
						[0, 0, 0],
						-50,
						50,
						0.1,
						false,
						"Camera Target",
					)}
					<div className="border-t pt-2 space-y-1.5">
						{renderToggle(`autoRotateCamera`, false, "Auto Orbit")}
						{renderNumberField(
							"cameraRotateSpeed",
							getBoundValue("cameraRotateSpeed"),
							2,
							0,
							10,
							0.1,
							"Orbit Speed",
						)}
					</div>
					<div className="border-t pt-2 space-y-1">
						{renderToggle("enableControls", true, "Enable Controls")}
						{renderToggle("enableZoom", true, "Enable Zoom")}
						{renderToggle("enablePan", false, "Enable Pan")}
					</div>
				</>,
				false,
			)}

			{section(
				"Lighting",
				icons.lighting,
				<>
					{renderSelect(
						"lightingPreset",
						"studio",
						"Preset",
						MODEL3D_ENUMS.lightingPreset,
					)}
					<div className="grid grid-cols-2 gap-x-3 gap-y-2">
						{renderNumberField(
							`ambientLight`,
							getBoundValue("ambientLight"),
							0.6,
							0,
							2,
							0.05,
							"Ambient",
						)}
						{renderNumberField(
							"directionalLight",
							getBoundValue("directionalLight"),
							1.2,
							0,
							3,
							0.05,
							"Key Light",
						)}
						{renderNumberField(
							"fillLight",
							getBoundValue("fillLight"),
							0.5,
							0,
							3,
							0.05,
							"Fill",
						)}
						{renderNumberField(
							"rimLight",
							getBoundValue("rimLight"),
							0.4,
							0,
							3,
							0.05,
							"Rim",
						)}
					</div>
					<BoundValueEditor
						name="lightColor"
						label={t("lightColor", "Light Color")}
						value={getBoundValue("lightColor") ?? { literalString: "#ffffff" }}
						onChange={(v) => updateProp("lightColor", v)}
						componentType="model3d"
					/>
				</>,
				false,
			)}

			{section(
				"Environment",
				icons.environment,
				<>
					{renderSelect(
						"environmentSource",
						"local",
						"Source",
						MODEL3D_ENUMS.environmentSource,
					)}
					{renderToggle("enableReflections", true, "Reflections")}
					{renderToggle("useHdrBackground", false, "Show as Background")}
					{(() => {
						const source = getBoundValue("environmentSource");
						const sourceValue =
							source && "literalString" in source
								? source.literalString
								: "local";
						if (sourceValue === "preset") {
							return renderSelect(
								"environment",
								"studio",
								"Environment",
								MODEL3D_ENUMS.environment,
							);
						}
						if (sourceValue === "custom") {
							return (
								<BoundValueEditor
									name="hdriUrl"
									label={t("hdriUrl", "HDRI URL")}
									value={getBoundValue("hdriUrl") ?? { literalString: "" }}
									onChange={(v) => updateProp("hdriUrl", v)}
									componentType="model3d"
								/>
							);
						}
						return (
							<>
								{renderSelect(
									"polyhavenHdri",
									"studio_small_03",
									"HDRI",
									MODEL3D_ENUMS.polyhavenHdri,
								)}
								{sourceValue === "polyhaven" &&
									renderSelect(
										"polyhavenResolution",
										"1k",
										"Resolution",
										MODEL3D_ENUMS.polyhavenResolution,
									)}
							</>
						);
					})()}
				</>,
				false,
			)}

			{section(
				"Ground",
				icons.ground,
				<>
					{renderToggle("showGround", false, "Show Ground")}
					<BoundValueEditor
						name="groundColor"
						label={t("groundColor", "Ground Color")}
						value={getBoundValue("groundColor") ?? { literalString: "#1a1a2e" }}
						onChange={(v) => updateProp("groundColor", v)}
						componentType="model3d"
					/>
					{renderNumberField(
						"groundSize",
						getBoundValue("groundSize"),
						200,
						10,
						1000,
						1,
						"Size",
					)}
					{renderNumberField(
						"groundOffsetY",
						getBoundValue("groundOffsetY"),
						-0.5,
						-10,
						10,
						0.01,
						"Vertical Offset",
					)}
					{renderToggle("groundFollowCamera", true, "Follow Camera")}
				</>,
				false,
			)}

			{section(
				"Viewer",
				icons.viewer,
				<>
					<BoundValueEditor
						name="viewerHeight"
						label="Height"
						value={getBoundValue("viewerHeight") ?? { literalString: "100%" }}
						onChange={(v) => updateProp("viewerHeight", v)}
						componentType="model3d"
					/>
					<BoundValueEditor
						name="backgroundColor"
						label={t("background", "Background")}
						value={
							getBoundValue("backgroundColor") ?? {
								literalString: "transparent",
							}
						}
						onChange={(v) => updateProp("backgroundColor", v)}
						componentType="model3d"
					/>
				</>,
				false,
			)}
		</div>
	);
}

// Chart Editor for PlotlyChart component
interface ChartEditorProps {
	component: SurfaceComponent;
	props: Record<string, unknown>;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
	updateProp: (key: string, value: unknown) => void;
}

const CHART_TYPES: { value: ChartType; label: string }[] = [
	{ value: "line", label: "Line" },
	{ value: "bar", label: "Bar" },
	{ value: "scatter", label: "Scatter" },
	{ value: "area", label: "Area" },
	{ value: "pie", label: "Pie" },
	{ value: "histogram", label: "Histogram" },
];

const CHART_COLORS = [
	"#6366f1",
	"#8b5cf6",
	"#ec4899",
	"#ef4444",
	"#f97316",
	"#eab308",
	"#22c55e",
	"#14b8a6",
	"#06b6d4",
	"#3b82f6",
];

function ChartEditor({
	component,
	props,
	onUpdate,
	updateProp,
}: ChartEditorProps) {
	const { t } = useTranslation("flow");
	const series = (props.series as ChartSeries[] | undefined) ?? [];
	const xAxis = (props.xAxis as ChartAxis | undefined) ?? {};
	const yAxis = (props.yAxis as ChartAxis | undefined) ?? {};

	const addSeries = useCallback(() => {
		const newSeries: ChartSeries = {
			name: t("seriesVal", "Series {{val}}", { val: series.length + 1 }),
			type: "line",
			dataSource: { csv: "Jan,10\nFeb,15\nMar,12\nApr,18" },
			color: CHART_COLORS[series.length % CHART_COLORS.length],
			mode: "lines+markers",
		};
		updateProp("series", [...series, newSeries]);
	}, [series, updateProp]);

	const removeSeries = useCallback(
		(index: number) => {
			updateProp(
				"series",
				series.filter((_, i) => i !== index),
			);
		},
		[series, updateProp],
	);

	const updateSeries = useCallback(
		(index: number, updates: Partial<ChartSeries>) => {
			const updated = [...series];
			updated[index] = { ...updated[index], ...updates };
			updateProp("series", updated);
		},
		[series, updateProp],
	);

	const updateXAxis = useCallback(
		(updates: Partial<ChartAxis>) => {
			updateProp("xAxis", { ...xAxis, ...updates });
		},
		[xAxis, updateProp],
	);

	const updateYAxis = useCallback(
		(updates: Partial<ChartAxis>) => {
			updateProp("yAxis", { ...yAxis, ...updates });
		},
		[yAxis, updateProp],
	);

	return (
		<div className="space-y-4">
			{/* Component ID */}
			<div className="space-y-2">
				<Label className="text-xs">{t("componentId", "Component ID")}</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* Chart Title */}
			<BoundValueEditor
				name="title"
				value={(props.title as BoundValue) ?? { literalString: "" }}
				onChange={(v) => updateProp("title", v)}
			/>

			{/* Data Series */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>
						{t("dataSeriesLength", "Data Series ({{length}})", {
							length: series.length,
						})}
					</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-3 pt-2">
					{series.map((s, idx) => (
						<div key={idx} className="space-y-2 rounded border p-2">
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium">
									{t("series", "Series")} {idx + 1}
								</span>
								<Button
									variant="ghost"
									size="icon"
									className="h-6 w-6"
									onClick={() => removeSeries(idx)}
								>
									<Trash2 className="h-3 w-3" />
								</Button>
							</div>
							<Input
								value={s.name}
								onChange={(e) => updateSeries(idx, { name: e.target.value })}
								placeholder={t("seriesName", "Series name")}
								className="h-7 text-xs"
							/>
							<div className="grid grid-cols-2 gap-2">
								<div>
									<Label className="text-xs">Type</Label>
									<Select
										value={s.type}
										onValueChange={(v) =>
											updateSeries(idx, { type: v as ChartType })
										}
									>
										<SelectTrigger className="h-7 text-xs">
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											{CHART_TYPES.map((t) => (
												<SelectItem key={t.value} value={t.value}>
													{t.label}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
								</div>
								<div>
									<Label className="text-xs">{t("color", "Color")}</Label>
									<Input
										type="color"
										value={s.color || "#6366f1"}
										onChange={(e) =>
											updateSeries(idx, { color: e.target.value })
										}
										className="h-7 w-full p-1"
									/>
								</div>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">
									{t("dataCsvLabelvalue", "Data (CSV: label,value)")}
								</Label>
								<Textarea
									value={
										s.dataSource && "csv" in s.dataSource
											? s.dataSource.csv
											: ""
									}
									onChange={(e) =>
										updateSeries(idx, { dataSource: { csv: e.target.value } })
									}
									placeholder="Jan,20&#10;Feb,14&#10;Mar,25"
									className="h-20 text-xs font-mono resize-none"
								/>
							</div>
							{(s.type === "line" ||
								s.type === "scatter" ||
								s.type === "area") && (
								<div>
									<Label className="text-xs">{t("mode", "Mode")}</Label>
									<Select
										value={s.mode || "lines+markers"}
										onValueChange={(v) =>
											updateSeries(idx, { mode: v as ChartSeries["mode"] })
										}
									>
										<SelectTrigger className="h-7 text-xs">
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value="lines">
												{t("lines", "Lines")}
											</SelectItem>
											<SelectItem value="markers">
												{t("markers", "Markers")}
											</SelectItem>
											<SelectItem value="lines+markers">
												{t("linesMarkers", "Lines + Markers")}
											</SelectItem>
										</SelectContent>
									</Select>
								</div>
							)}
						</div>
					))}
					<Button
						variant="outline"
						size="sm"
						className="w-full h-7 text-xs"
						onClick={addSeries}
					>
						<Plus className="h-3 w-3 mr-1" />
						{t("addSeries", "Add Series")}
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* X Axis */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("xAxis", "X Axis")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">{t("title", "Title")}</Label>
						<Input
							value={xAxis.title || ""}
							onChange={(e) => updateXAxis({ title: e.target.value })}
							placeholder={t("xAxisTitle", "X Axis Title")}
							className="h-7 text-xs"
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Type</Label>
						<Select
							value={xAxis.type || "category"}
							onValueChange={(v) =>
								updateXAxis({ type: v as ChartAxis["type"] })
							}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="category">
									{t("category", "Category")}
								</SelectItem>
								<SelectItem value="linear">{t("linear", "Linear")}</SelectItem>
								<SelectItem value="log">{t("log", "Log")}</SelectItem>
								<SelectItem value="date">{t("date", "Date")}</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("showGrid", "Show Grid")}</Label>
						<Switch
							checked={xAxis.showGrid ?? true}
							onCheckedChange={(v) => updateXAxis({ showGrid: v })}
						/>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Y Axis */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("yAxis", "Y Axis")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">{t("title", "Title")}</Label>
						<Input
							value={yAxis.title || ""}
							onChange={(e) => updateYAxis({ title: e.target.value })}
							placeholder={t("yAxisTitle", "Y Axis Title")}
							className="h-7 text-xs"
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Type</Label>
						<Select
							value={yAxis.type || "linear"}
							onValueChange={(v) =>
								updateYAxis({ type: v as ChartAxis["type"] })
							}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="linear">{t("linear", "Linear")}</SelectItem>
								<SelectItem value="log">{t("log", "Log")}</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("min", "Min")}</Label>
							<Input
								type="number"
								value={xAxis.min ?? ""}
								onChange={(e) =>
									updateYAxis({
										min: e.target.value ? Number(e.target.value) : undefined,
									})
								}
								placeholder={t("auto", "Auto")}
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("max", "Max")}</Label>
							<Input
								type="number"
								value={yAxis.max ?? ""}
								onChange={(e) =>
									updateYAxis({
										max: e.target.value ? Number(e.target.value) : undefined,
									})
								}
								placeholder={t("auto", "Auto")}
								className="h-7 text-xs"
							/>
						</div>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("showGrid", "Show Grid")}</Label>
						<Switch
							checked={yAxis.showGrid ?? true}
							onCheckedChange={(v) => updateYAxis({ showGrid: v })}
						/>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Display Options */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("display", "Display")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<BoundValueEditor
						name="width"
						value={(props.width as BoundValue) ?? { literalString: "100%" }}
						onChange={(v) => updateProp("width", v)}
					/>
					<BoundValueEditor
						name="height"
						value={(props.height as BoundValue) ?? { literalString: "400px" }}
						onChange={(v) => updateProp("height", v)}
					/>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("showLegend", "Show Legend")}</Label>
						<Switch
							checked={
								props.showLegend &&
								"literalBool" in (props.showLegend as BoundValue)
									? (props.showLegend as { literalBool: boolean }).literalBool
									: true
							}
							onCheckedChange={(v) =>
								updateProp("showLegend", { literalBool: v })
							}
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">
							{t("legendPosition", "Legend Position")}
						</Label>
						<Select
							value={
								props.legendPosition &&
								"literalString" in (props.legendPosition as BoundValue)
									? (props.legendPosition as { literalString: string })
											.literalString
									: "bottom"
							}
							onValueChange={(v) =>
								updateProp("legendPosition", { literalString: v })
							}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="top">{t("top", "Top")}</SelectItem>
								<SelectItem value="bottom">{t("bottom", "Bottom")}</SelectItem>
								<SelectItem value="left">{t("left", "Left")}</SelectItem>
								<SelectItem value="right">{t("right", "Right")}</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</CollapsibleContent>
			</Collapsible>
		</div>
	);
}

// Nivo Chart Editor component
interface NivoChartEditorProps {
	component: SurfaceComponent;
	props: Record<string, unknown>;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
	updateProp: (key: string, value: unknown) => void;
}

// Color scheme options
const NIVO_COLOR_SCHEMES = [
	{ value: "nivo", label: "Nivo" },
	{ value: "paired", label: "Paired" },
	{ value: "category10", label: "Category 10" },
	{ value: "accent", label: "Accent" },
	{ value: "dark2", label: "Dark 2" },
	{ value: "pastel1", label: "Pastel 1" },
	{ value: "pastel2", label: "Pastel 2" },
	{ value: "set1", label: "Set 1" },
	{ value: "set2", label: "Set 2" },
	{ value: "set3", label: "Set 3" },
	{ value: "spectral", label: "Spectral" },
	{ value: "blues", label: "Blues" },
	{ value: "greens", label: "Greens" },
	{ value: "reds", label: "Reds" },
	{ value: "purples", label: "Purples" },
];

function NivoChartEditor({
	component,
	props,
	onUpdate,
	updateProp,
}: NivoChartEditorProps) {
	const { t } = useTranslation("flow");
	const [dataMode, setDataMode] = useState<"json" | "csv">("json");
	const [csvInput, setCsvInput] = useState("");
	const [csvError, setCsvError] = useState<string | null>(null);

	// Get current chart type
	const chartType = useMemo(() => {
		const ct = props.chartType as BoundValue | undefined;
		if (ct && "literalString" in ct) return ct.literalString;
		return "bar";
	}, [props.chartType]);

	// Get current data
	const currentData = useMemo(() => {
		const d = props.data as BoundValue | undefined;
		if (d && "literalJson" in d) {
			try {
				return JSON.parse(d.literalJson as string);
			} catch {
				return [];
			}
		}
		return [];
	}, [props.data]);

	// Handle chart type change - apply new defaults in a single update
	const handleChartTypeChange = useCallback(
		(newType: string) => {
			const updates: Record<string, unknown> = {
				chartType: { literalString: newType },
			};

			// Apply default data for the new chart type
			const defaultData = NIVO_SAMPLE_DATA[newType];
			if (defaultData) {
				updates.data = { literalJson: JSON.stringify(defaultData, null, 2) };
			}

			// Apply default keys and indexBy
			const defaults = NIVO_CHART_DEFAULTS[newType];
			if (defaults) {
				if (defaults.indexBy) {
					updates.indexBy = { literalString: defaults.indexBy };
				}
				if (defaults.keys) {
					updates.keys = { literalJson: JSON.stringify(defaults.keys) };
				}
			} else {
				// Clear keys/indexBy for charts that don't use them
				updates.indexBy = { literalString: "" };
				updates.keys = { literalJson: "[]" };
			}

			// Apply all updates at once
			onUpdate({
				component: {
					...component.component,
					...updates,
				} as SurfaceComponent["component"],
			});
		},
		[component.component, onUpdate],
	);

	// Parse CSV to JSON data
	const parseCsvToData = useCallback((csv: string, type: string) => {
		setCsvError(null);
		try {
			const lines = csv
				.trim()
				.split("\n")
				.filter((l) => l.trim());
			if (lines.length === 0) return [];

			const headers = lines[0].split(",").map((h) => h.trim());

			if (type === "bar" || type === "radar") {
				// For bar/radar: first column is index, rest are data keys
				return lines.slice(1).map((line) => {
					const values = line.split(",").map((v) => v.trim());
					const row: Record<string, string | number> = {
						[headers[0]]: values[0],
					};
					for (let i = 1; i < headers.length; i++) {
						row[headers[i]] = Number(values[i]) || 0;
					}
					return row;
				});
			} else if (type === "line" || type === "scatter") {
				// For line/scatter: create series from columns
				const series: {
					id: string;
					data: { x: string | number; y: number }[];
				}[] = [];
				for (let i = 1; i < headers.length; i++) {
					series.push({
						id: headers[i],
						data: lines.slice(1).map((line) => {
							const values = line.split(",").map((v) => v.trim());
							return { x: values[0], y: Number(values[i]) || 0 };
						}),
					});
				}
				return series;
			} else if (type === "pie" || type === "funnel" || type === "waffle") {
				// For pie/funnel/waffle: label,value format
				return lines.slice(1).map((line) => {
					const [label, value] = line.split(",").map((v) => v.trim());
					return { id: label, value: Number(value) || 0, label };
				});
			} else if (type === "calendar") {
				// For calendar: date,value format
				return lines.slice(1).map((line) => {
					const [day, value] = line.split(",").map((v) => v.trim());
					return { day, value: Number(value) || 0 };
				});
			}

			// Default: try to parse as generic data
			return lines.slice(1).map((line) => {
				const values = line.split(",").map((v) => v.trim());
				const row: Record<string, string | number> = {};
				headers.forEach((h, i) => {
					const val = values[i];
					row[h] = Number.isNaN(Number(val)) ? val : Number(val);
				});
				return row;
			});
		} catch (err) {
			setCsvError("Failed to parse CSV data");
			return [];
		}
	}, []);

	// Apply CSV data
	const applyCsvData = useCallback(() => {
		const data = parseCsvToData(csvInput, chartType);
		if (data.length > 0) {
			updateProp("data", { literalJson: JSON.stringify(data, null, 2) });

			// Auto-detect keys for bar/radar charts
			if ((chartType === "bar" || chartType === "radar") && data[0]) {
				const firstRow = data[0] as Record<string, unknown>;
				const keys = Object.keys(firstRow).filter(
					(k) => typeof firstRow[k] === "number",
				);
				const indexBy = Object.keys(firstRow).find(
					(k) => typeof firstRow[k] === "string",
				);
				if (keys.length > 0) {
					updateProp("keys", { literalJson: JSON.stringify(keys) });
				}
				if (indexBy) {
					updateProp("indexBy", { literalString: indexBy });
				}
			}
		}
	}, [csvInput, chartType, parseCsvToData, updateProp]);

	// Check if chart type needs keys/indexBy
	const needsKeysAndIndex = ["bar", "radar", "stream", "marimekko"].includes(
		chartType,
	);

	return (
		<div className="space-y-4">
			{/* Component ID */}
			<div className="space-y-2">
				<Label className="text-xs">{t("componentId", "Component ID")}</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* Chart Type */}
			<div className="space-y-2">
				<Label className="text-xs">{t("chartType", "Chart Type")}</Label>
				<Select value={chartType} onValueChange={handleChartTypeChange}>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{NIVO_CHART_TYPES.map((ct) => (
							<SelectItem key={ct.value} value={ct.value}>
								{ct.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			{/* Data Input */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("data", "Data")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-3 pt-2">
					{/* Data mode toggle */}
					<div className="flex gap-1">
						<Button
							variant={dataMode === "json" ? "default" : "outline"}
							size="sm"
							className="h-7 text-xs flex-1"
							onClick={() => setDataMode("json")}
						>
							{`JSON`}
						</Button>
						<Button
							variant={dataMode === "csv" ? "default" : "outline"}
							size="sm"
							className="h-7 text-xs flex-1"
							onClick={() => setDataMode("csv")}
						>
							CSV
						</Button>
					</div>

					{dataMode === "json" ? (
						<div className="space-y-2">
							<Textarea
								value={
									typeof currentData === "object"
										? JSON.stringify(currentData, null, 2)
										: "[]"
								}
								onChange={(e) => {
									try {
										JSON.parse(e.target.value);
										updateProp("data", { literalJson: e.target.value });
									} catch {
										// Allow invalid JSON during editing
										updateProp("data", { literalJson: e.target.value });
									}
								}}
								placeholder={t("enterJsonData", "Enter JSON data...")}
								className="h-40 text-xs font-mono resize-none"
							/>
							<Button
								variant="outline"
								size="sm"
								className="w-full h-7 text-xs"
								onClick={() => {
									const defaultData = NIVO_SAMPLE_DATA[chartType];
									if (defaultData) {
										updateProp("data", {
											literalJson: JSON.stringify(defaultData, null, 2),
										});
									}
								}}
							>
								{t("resetToDefaultData", "Reset to Default Data")}
							</Button>
						</div>
					) : (
						<div className="space-y-2">
							<Textarea
								value={csvInput}
								onChange={(e) => setCsvInput(e.target.value)}
								placeholder={
									chartType === "bar" || chartType === "radar"
										? "category,series1,series2\nA,10,20\nB,15,25"
										: chartType === "pie" || chartType === "funnel"
											? "label,value\nCategory A,35\nCategory B,25"
											: chartType === "line"
												? "x,series1,series2\nJan,10,15\nFeb,20,18"
												: "header1,header2\nvalue1,value2"
								}
								className="h-32 text-xs font-mono resize-none"
							/>
							{csvError && <p className="text-xs text-red-500">{csvError}</p>}
							<Button
								variant="outline"
								size="sm"
								className="w-full h-7 text-xs"
								onClick={applyCsvData}
							>
								{t("applyCsvData", "Apply CSV Data")}
							</Button>
						</div>
					)}
				</CollapsibleContent>
			</Collapsible>

			{/* Keys & Index (for bar, radar, etc.) */}
			{needsKeysAndIndex && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>{t("dataKeys", "Data Keys")}</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">
								{t("indexByCategoryField", "Index By (Category Field)")}
							</Label>
							<Input
								value={
									props.indexBy &&
									"literalString" in (props.indexBy as BoundValue)
										? (props.indexBy as { literalString: string }).literalString
										: ""
								}
								onChange={(e) =>
									updateProp("indexBy", { literalString: e.target.value })
								}
								placeholder={t("egCountryCategory", "e.g. country, category")}
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">
								{t(
									"keysDataSeriesCommaSeparated",
									"Keys (Data Series - comma separated)",
								)}
							</Label>
							<Input
								value={(() => {
									const k = props.keys as BoundValue | undefined;
									if (k && "literalJson" in k) {
										try {
											const parsed = JSON.parse(k.literalJson as string);
											return Array.isArray(parsed) ? parsed.join(", ") : "";
										} catch {
											return "";
										}
									}
									return "";
								})()}
								onChange={(e) => {
									const keys = e.target.value
										.split(",")
										.map((k) => k.trim())
										.filter(Boolean);
									updateProp("keys", { literalJson: JSON.stringify(keys) });
								}}
								placeholder={t(
									"egSalesRevenueProfit",
									"e.g. sales, revenue, profit",
								)}
								className="h-7 text-xs"
							/>
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}

			{/* Styling */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("styling", "Styling")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">
							{t("colorScheme", "Color Scheme")}
						</Label>
						<Select
							value={
								props.colors && "literalString" in (props.colors as BoundValue)
									? (props.colors as { literalString: string }).literalString
									: "nivo"
							}
							onValueChange={(v) => updateProp("colors", { literalString: v })}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{NIVO_COLOR_SCHEMES.map((cs) => (
									<SelectItem key={cs.value} value={cs.value}>
										{cs.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("animate", "Animate")}</Label>
						<Switch
							checked={
								props.animate && "literalBool" in (props.animate as BoundValue)
									? (props.animate as { literalBool: boolean }).literalBool
									: true
							}
							onCheckedChange={(v) => updateProp("animate", { literalBool: v })}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("showLegend", "Show Legend")}</Label>
						<Switch
							checked={
								props.showLegend &&
								"literalBool" in (props.showLegend as BoundValue)
									? (props.showLegend as { literalBool: boolean }).literalBool
									: true
							}
							onCheckedChange={(v) =>
								updateProp("showLegend", { literalBool: v })
							}
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">
							{t("legendPosition", "Legend Position")}
						</Label>
						<Select
							value={
								props.legendPosition &&
								"literalString" in (props.legendPosition as BoundValue)
									? (props.legendPosition as { literalString: string })
											.literalString
									: "bottom"
							}
							onValueChange={(v) =>
								updateProp("legendPosition", { literalString: v })
							}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="top">{t("top", "Top")}</SelectItem>
								<SelectItem value="bottom">{t("bottom", "Bottom")}</SelectItem>
								<SelectItem value="left">{t("left", "Left")}</SelectItem>
								<SelectItem value="right">{t("right", "Right")}</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Chart-specific styling */}
			{chartType === "bar" && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>{t("barOptions", "Bar Options")}</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("layout", "Layout")}</Label>
							<Select
								value={(() => {
									const s = props.barStyle as { layout?: string } | undefined;
									return s?.layout || "vertical";
								})()}
								onValueChange={(v) =>
									updateProp("barStyle", {
										...((props.barStyle as object) || {}),
										layout: v,
									})
								}
							>
								<SelectTrigger className="h-7 text-xs">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="vertical">
										{t("vertical", "Vertical")}
									</SelectItem>
									<SelectItem value="horizontal">
										{t("horizontal", "Horizontal")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("groupMode", "Group Mode")}</Label>
							<Select
								value={(() => {
									const s = props.barStyle as
										| { groupMode?: string }
										| undefined;
									return s?.groupMode || "grouped";
								})()}
								onValueChange={(v) =>
									updateProp("barStyle", {
										...((props.barStyle as object) || {}),
										groupMode: v,
									})
								}
							>
								<SelectTrigger className="h-7 text-xs">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="grouped">
										{t("grouped", "Grouped")}
									</SelectItem>
									<SelectItem value="stacked">
										{t("stacked", "Stacked")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">
								{t("borderRadius", "Border Radius")}
							</Label>
							<Input
								type="number"
								value={(() => {
									const s = props.barStyle as
										| { borderRadius?: number }
										| undefined;
									return s?.borderRadius ?? 0;
								})()}
								onChange={(e) =>
									updateProp("barStyle", {
										...((props.barStyle as object) || {}),
										borderRadius: Number(e.target.value),
									})
								}
								className="h-7 text-xs"
							/>
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}

			{chartType === "pie" && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>{t("pieOptions", "Pie Options")}</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">
								{t(
									"innerRadius0PieGt0Donut",
									"Inner Radius (0 = pie, >0 = donut)",
								)}
							</Label>
							<Input
								type="number"
								step="0.1"
								min="0"
								max="0.9"
								value={(() => {
									const s = props.pieStyle as
										| { innerRadius?: number }
										| undefined;
									return s?.innerRadius ?? 0;
								})()}
								onChange={(e) =>
									updateProp("pieStyle", {
										...((props.pieStyle as object) || {}),
										innerRadius: Number(e.target.value),
									})
								}
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("padAngle", "Pad Angle")}</Label>
							<Input
								type="number"
								step="0.5"
								min="0"
								value={(() => {
									const s = props.pieStyle as { padAngle?: number } | undefined;
									return s?.padAngle ?? 0;
								})()}
								onChange={(e) =>
									updateProp("pieStyle", {
										...((props.pieStyle as object) || {}),
										padAngle: Number(e.target.value),
									})
								}
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">
								{t("cornerRadius", "Corner Radius")}
							</Label>
							<Input
								type="number"
								min="0"
								value={(() => {
									const s = props.pieStyle as
										| { cornerRadius?: number }
										| undefined;
									return s?.cornerRadius ?? 0;
								})()}
								onChange={(e) =>
									updateProp("pieStyle", {
										...((props.pieStyle as object) || {}),
										cornerRadius: Number(e.target.value),
									})
								}
								className="h-7 text-xs"
							/>
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}

			{chartType === "line" && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>{t("lineOptions", "Line Options")}</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("curve", "Curve")}</Label>
							<Select
								value={(() => {
									const s = props.lineStyle as { curve?: string } | undefined;
									return s?.curve || "linear";
								})()}
								onValueChange={(v) =>
									updateProp("lineStyle", {
										...((props.lineStyle as object) || {}),
										curve: v,
									})
								}
							>
								<SelectTrigger className="h-7 text-xs">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="linear">
										{t("linear", "Linear")}
									</SelectItem>
									<SelectItem value="monotoneX">
										{t("smooth", "Smooth")}
									</SelectItem>
									<SelectItem value="natural">
										{t("natural", "Natural")}
									</SelectItem>
									<SelectItem value="step">{t("step", "Step")}</SelectItem>
									<SelectItem value="stepBefore">
										{t("stepBefore", "Step Before")}
									</SelectItem>
									<SelectItem value="stepAfter">
										{t("stepAfter", "Step After")}
									</SelectItem>
									<SelectItem value="basis">{t("basis", "Basis")}</SelectItem>
									<SelectItem value="cardinal">
										{t("cardinal", "Cardinal")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="flex items-center justify-between">
							<Label className="text-xs">
								{t("enableArea", "Enable Area")}
							</Label>
							<Switch
								checked={(() => {
									const s = props.lineStyle as
										| { enableArea?: boolean }
										| undefined;
									return s?.enableArea ?? false;
								})()}
								onCheckedChange={(v) =>
									updateProp("lineStyle", {
										...((props.lineStyle as object) || {}),
										enableArea: v,
									})
								}
							/>
						</div>
						<div className="flex items-center justify-between">
							<Label className="text-xs">
								{t("showPoints", "Show Points")}
							</Label>
							<Switch
								checked={(() => {
									const s = props.lineStyle as
										| { enablePoints?: boolean }
										| undefined;
									return s?.enablePoints ?? true;
								})()}
								onCheckedChange={(v) =>
									updateProp("lineStyle", {
										...((props.lineStyle as object) || {}),
										enablePoints: v,
									})
								}
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("lineWidth", "Line Width")}</Label>
							<Input
								type="number"
								min="1"
								max="10"
								value={(() => {
									const s = props.lineStyle as
										| { lineWidth?: number }
										| undefined;
									return s?.lineWidth ?? 2;
								})()}
								onChange={(e) =>
									updateProp("lineStyle", {
										...((props.lineStyle as object) || {}),
										lineWidth: Number(e.target.value),
									})
								}
								className="h-7 text-xs"
							/>
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}

			{/* Display Options */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("display", "Display")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<BoundValueEditor
						name="width"
						value={(props.width as BoundValue) ?? { literalString: "100%" }}
						onChange={(v) => updateProp("width", v)}
					/>
					<BoundValueEditor
						name="height"
						value={(props.height as BoundValue) ?? { literalString: "400px" }}
						onChange={(v) => updateProp("height", v)}
					/>
				</CollapsibleContent>
			</Collapsible>
		</div>
	);
}

// Table Editor for Table component
interface TableEditorProps {
	component: SurfaceComponent;
	props: Record<string, unknown>;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
	updateProp: (key: string, value: unknown) => void;
}

function TableEditor({
	component,
	props,
	onUpdate,
	updateProp,
}: TableEditorProps) {
	const { t } = useTranslation("flow");
	const [csvInput, setCsvInput] = useState("");
	const [csvError, setCsvError] = useState<string | null>(null);

	// Extract columns and data from props
	const columns = useMemo(() => {
		const colsValue = props.columns as BoundValue | undefined;
		if (!colsValue) return [];
		if ("literalJson" in colsValue) {
			try {
				const parsed = JSON.parse(colsValue.literalJson as string);
				return Array.isArray(parsed) ? parsed : [];
			} catch {
				return [];
			}
		}
		return [];
	}, [props.columns]);

	const data = useMemo(() => {
		const dataValue = props.data as BoundValue | undefined;
		if (!dataValue) return [];
		if ("literalJson" in dataValue) {
			try {
				const parsed = JSON.parse(dataValue.literalJson as string);
				return Array.isArray(parsed) ? parsed : [];
			} catch {
				return [];
			}
		}
		return [];
	}, [props.data]);

	// Parse CSV and update table
	const handleCsvImport = useCallback(() => {
		setCsvError(null);
		if (!csvInput.trim()) {
			setCsvError("CSV input is empty");
			return;
		}

		try {
			const lines = csvInput.trim().split("\n");
			if (lines.length < 1) {
				setCsvError("CSV must have at least a header row");
				return;
			}

			// Parse header row
			const headers = lines[0].split(",").map((h) => h.trim());
			if (headers.some((h) => !h)) {
				setCsvError("All column headers must be non-empty");
				return;
			}

			// Create columns from headers
			const newColumns: TableColumn[] = headers.map((header, idx) => ({
				id: `col-${idx}`,
				header: { literalString: header },
				accessor: { literalString: header.toLowerCase().replace(/\s+/g, "_") },
				sortable: { literalBool: true },
			}));

			// Parse data rows
			const newData: Record<string, string>[] = [];
			for (let i = 1; i < lines.length; i++) {
				const values = lines[i].split(",").map((v) => v.trim());
				const row: Record<string, string> = {};
				for (let j = 0; j < headers.length; j++) {
					const accessor = headers[j].toLowerCase().replace(/\s+/g, "_");
					row[accessor] = values[j] ?? "";
				}
				newData.push(row);
			}

			// Update columns and data
			updateProp("columns", { literalJson: JSON.stringify(newColumns) });
			updateProp("data", { literalJson: JSON.stringify(newData) });
			setCsvInput("");
		} catch (err) {
			setCsvError(
				t("failedToParseCsvVal", "Failed to parse CSV: {{val}}", {
					val: err instanceof Error ? err.message : "Unknown error",
				}),
			);
		}
	}, [csvInput, updateProp]);

	// Add a new column
	const addColumn = useCallback(() => {
		const newColumn: TableColumn = {
			id: `col-${Date.now()}`,
			header: {
				literalString: t("columnVal", "Column {{val}}", {
					val: columns.length + 1,
				}),
			},
			accessor: { literalString: `column_${columns.length + 1}` },
			sortable: { literalBool: true },
		};
		updateProp("columns", {
			literalJson: JSON.stringify([...columns, newColumn]),
		});
	}, [columns, updateProp]);

	// Remove a column
	const removeColumn = useCallback(
		(index: number) => {
			const newColumns = columns.filter(
				(_: TableColumn, i: number) => i !== index,
			);
			updateProp("columns", { literalJson: JSON.stringify(newColumns) });
		},
		[columns, updateProp],
	);

	// Update a column
	const updateColumn = useCallback(
		(index: number, updates: Partial<TableColumn>) => {
			const newColumns = [...columns];
			newColumns[index] = { ...newColumns[index], ...updates };
			updateProp("columns", { literalJson: JSON.stringify(newColumns) });
		},
		[columns, updateProp],
	);

	// Add a new row
	const addRow = useCallback(() => {
		const newRow: Record<string, string> = {};
		for (const col of columns) {
			const accessor =
				typeof col.accessor === "string"
					? col.accessor
					: col.accessor &&
							typeof col.accessor === "object" &&
							"literalString" in col.accessor
						? col.accessor.literalString
						: col.id;
			newRow[accessor] = "";
		}
		updateProp("data", { literalJson: JSON.stringify([...data, newRow]) });
	}, [columns, data, updateProp]);

	// Remove a row
	const removeRow = useCallback(
		(index: number) => {
			const newData = data.filter((_: unknown, i: number) => i !== index);
			updateProp("data", { literalJson: JSON.stringify(newData) });
		},
		[data, updateProp],
	);

	// Update a cell
	const updateCell = useCallback(
		(rowIndex: number, accessor: string, value: string) => {
			const newData = [...data];
			newData[rowIndex] = { ...newData[rowIndex], [accessor]: value };
			updateProp("data", { literalJson: JSON.stringify(newData) });
		},
		[data, updateProp],
	);

	// Get accessor string from column
	const getAccessor = (col: TableColumn): string => {
		if (typeof col.accessor === "string") return col.accessor;
		if (
			col.accessor &&
			typeof col.accessor === "object" &&
			"literalString" in col.accessor
		) {
			return col.accessor.literalString;
		}
		return col.id;
	};

	// Get header string from column
	const getHeader = (col: TableColumn): string => {
		if (typeof col.header === "string") return col.header;
		if (
			col.header &&
			typeof col.header === "object" &&
			"literalString" in col.header
		) {
			return col.header.literalString;
		}
		return col.id;
	};

	return (
		<div className="space-y-4">
			{/* Component ID */}
			<div className="space-y-2">
				<Label className="text-xs">{t("componentId", "Component ID")}</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* CSV Import */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{`Import from CSV`}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<Textarea
						value={csvInput}
						onChange={(e) => setCsvInput(e.target.value)}
						placeholder="Name,Age,Email&#10;John,25,john@example.com&#10;Jane,30,jane@example.com"
						className="text-xs font-mono min-h-[100px]"
					/>
					{csvError && <p className="text-xs text-destructive">{csvError}</p>}
					<Button
						size="sm"
						className="w-full"
						onClick={handleCsvImport}
						disabled={!csvInput.trim()}
					>
						{t("importCsv", "Import CSV")}
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Columns */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>
						{t("columnsLength", "Columns ({{length}})", {
							length: columns.length,
						})}
					</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					{columns.map((col: TableColumn, idx: number) => (
						<div key={col.id} className="space-y-2 rounded border p-2">
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium truncate">
									{getHeader(col)}
								</span>
								<Button
									variant="ghost"
									size="icon"
									className="h-6 w-6 shrink-0"
									onClick={() => removeColumn(idx)}
								>
									<Trash2 className="h-3 w-3" />
								</Button>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">{t("header", "Header")}</Label>
								<Input
									value={getHeader(col)}
									onChange={(e) =>
										updateColumn(idx, {
											header: { literalString: e.target.value },
										})
									}
									className="h-7 text-xs"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">
									{t("accessorDataKey", "Accessor (data key)")}
								</Label>
								<Input
									value={getAccessor(col)}
									onChange={(e) =>
										updateColumn(idx, {
											accessor: { literalString: e.target.value },
										})
									}
									className="h-7 text-xs font-mono"
								/>
							</div>
							<div className="flex items-center justify-between">
								<Label className="text-xs">{t("sortable", "Sortable")}</Label>
								<Switch
									checked={
										col.sortable &&
										typeof col.sortable === "object" &&
										"literalBool" in col.sortable
											? col.sortable.literalBool
											: typeof col.sortable === "boolean"
												? col.sortable
												: false
									}
									onCheckedChange={(v) =>
										updateColumn(idx, { sortable: { literalBool: v } })
									}
								/>
							</div>
						</div>
					))}
					<Button
						variant="outline"
						size="sm"
						className="w-full"
						onClick={addColumn}
					>
						<Plus className="h-3 w-3 mr-1" />
						{t("addColumn", "Add Column")}
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Data Rows */}
			<Collapsible defaultOpen={data.length <= 5}>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>
						{t("dataCountRows", {
							defaultValue_one: "Data ({{count}} row)",
							defaultValue_other: "Data ({{count}} rows)",
							count: data.length,
						})}
					</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					{data.length === 0 && columns.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							{`Add columns first or import from CSV`}
						</p>
					) : data.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							{`No data rows. Add a row below or import from CSV.`}
						</p>
					) : (
						<div className="space-y-2 max-h-[300px] overflow-y-auto">
							{data.map((row: Record<string, string>, rowIdx: number) => (
								<div key={rowIdx} className="rounded border p-2 space-y-1">
									<div className="flex items-center justify-between mb-1">
										<span className="text-xs font-medium">
											{t("row", "Row")} {rowIdx + 1}
										</span>
										<Button
											variant="ghost"
											size="icon"
											className="h-5 w-5"
											onClick={() => removeRow(rowIdx)}
										>
											<Trash2 className="h-3 w-3" />
										</Button>
									</div>
									{columns.map((col: TableColumn) => {
										const accessor = getAccessor(col);
										return (
											<div key={col.id} className="space-y-0.5">
												<Label className="text-xs text-muted-foreground">
													{getHeader(col)}
												</Label>
												<Input
													value={row[accessor] ?? ""}
													onChange={(e) =>
														updateCell(rowIdx, accessor, e.target.value)
													}
													className="h-6 text-xs"
												/>
											</div>
										);
									})}
								</div>
							))}
						</div>
					)}
					<Button
						variant="outline"
						size="sm"
						className="w-full"
						onClick={addRow}
						disabled={columns.length === 0}
					>
						<Plus className="h-3 w-3 mr-1" />
						{t("addRow", "Add Row")}
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Table Options */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("options", "Options")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<BoundValueEditor
						name="caption"
						value={(props.caption as BoundValue) ?? { literalString: "" }}
						onChange={(v) => updateProp("caption", v)}
					/>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("striped", "Striped")}</Label>
						<Switch
							checked={
								props.striped && "literalBool" in (props.striped as BoundValue)
									? (props.striped as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) => updateProp("striped", { literalBool: v })}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("bordered", "Bordered")}</Label>
						<Switch
							checked={
								props.bordered &&
								"literalBool" in (props.bordered as BoundValue)
									? (props.bordered as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) =>
								updateProp("bordered", { literalBool: v })
							}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("hoverable", "Hoverable")}</Label>
						<Switch
							checked={
								props.hoverable &&
								"literalBool" in (props.hoverable as BoundValue)
									? (props.hoverable as { literalBool: boolean }).literalBool
									: true
							}
							onCheckedChange={(v) =>
								updateProp("hoverable", { literalBool: v })
							}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("compact", "Compact")}</Label>
						<Switch
							checked={
								props.compact && "literalBool" in (props.compact as BoundValue)
									? (props.compact as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) => updateProp("compact", { literalBool: v })}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">
							{t("stickyHeader", "Sticky Header")}
						</Label>
						<Switch
							checked={
								props.stickyHeader &&
								"literalBool" in (props.stickyHeader as BoundValue)
									? (props.stickyHeader as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) =>
								updateProp("stickyHeader", { literalBool: v })
							}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("sortable", "Sortable")}</Label>
						<Switch
							checked={
								props.sortable &&
								"literalBool" in (props.sortable as BoundValue)
									? (props.sortable as { literalBool: boolean }).literalBool
									: true
							}
							onCheckedChange={(v) =>
								updateProp("sortable", { literalBool: v })
							}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("searchable", "Searchable")}</Label>
						<Switch
							checked={
								props.searchable &&
								"literalBool" in (props.searchable as BoundValue)
									? (props.searchable as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) =>
								updateProp("searchable", { literalBool: v })
							}
						/>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("paginated", "Paginated")}</Label>
						<Switch
							checked={
								props.paginated &&
								"literalBool" in (props.paginated as BoundValue)
									? (props.paginated as { literalBool: boolean }).literalBool
									: false
							}
							onCheckedChange={(v) =>
								updateProp("paginated", { literalBool: v })
							}
						/>
					</div>
					{Boolean(
						props.paginated &&
							"literalBool" in (props.paginated as BoundValue) &&
							(props.paginated as { literalBool: boolean }).literalBool,
					) && (
						<div className="space-y-1">
							<Label className="text-xs">{t("pageSize", "Page Size")}</Label>
							<Input
								type="number"
								min={1}
								value={
									props.pageSize &&
									"literalNumber" in (props.pageSize as BoundValue)
										? (props.pageSize as { literalNumber: number })
												.literalNumber
										: 10
								}
								onChange={(e) =>
									updateProp("pageSize", {
										literalNumber: Number.parseInt(e.target.value, 10) || 10,
									})
								}
								className="h-7 text-xs"
							/>
						</div>
					)}
				</CollapsibleContent>
			</Collapsible>
		</div>
	);
}

interface PropertyFieldProps {
	name: string;
	value: unknown;
	onChange: (value: unknown) => void;
	isAssetProperty?: boolean;
	assetAccept?: AssetAccept;
	componentType?: string;
	enumOptions?: string[];
}

/** Picks one of the project's ontologies by name instead of pasting its id. */
function OntologyIdField({
	appId,
	value,
	onChange,
}: {
	appId: string;
	value: string;
	onChange: (value: unknown) => void;
}) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const ontologies = useInvoke(
		backend.graphState.listOverlays,
		backend.graphState,
		[appId],
		Boolean(appId),
	);

	return (
		<div className={INSPECTOR_FIELD_CLASS}>
			<Label className="text-xs">{t("ontology", "Ontology")}</Label>
			<Select
				value={value || undefined}
				onValueChange={(next) => onChange({ literalString: next })}
			>
				<SelectTrigger className="h-8 text-sm">
					<SelectValue
						placeholder={t("selectAnOntology", "Select an ontology...")}
					/>
				</SelectTrigger>
				<SelectContent>
					{(ontologies.data ?? []).map((ontology) => (
						<SelectItem key={ontology.id} value={ontology.id}>
							{ontology.name}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{ontologies.data?.length === 0 && (
				<p className="text-[11px] text-muted-foreground">
					{t(
						"thisProjectHasNoOntologiesYetCreateOneInDataStudio",
						"This project has no ontologies yet — create one in Data Studio.",
					)}
				</p>
			)}
		</div>
	);
}

function PropertyField({
	name,
	value,
	onChange,
	isAssetProperty,
	assetAccept = "all",
	componentType,
	enumOptions,
}: PropertyFieldProps) {
	const { t } = useTranslation("flow");
	const { actionContext } = useBuilder();
	const appId = actionContext?.appId;

	// The ontology element is a link to existing project data — pick it by name.
	// A path-bound id keeps the generic editor so bindings stay editable.
	const literalOntologyId =
		value === undefined
			? ""
			: typeof value === "object" &&
					value !== null &&
					"literalString" in value &&
					typeof (value as { literalString: unknown }).literalString ===
						"string"
				? (value as { literalString: string }).literalString
				: null;

	if (
		componentType === "ontologyGraph" &&
		name === "ontologyId" &&
		appId &&
		literalOntologyId !== null
	) {
		return (
			<OntologyIdField
				appId={appId}
				value={literalOntologyId}
				onChange={onChange}
			/>
		);
	}

	// Skip rendering complex objects for now
	if (typeof value === "object" && value !== null) {
		if (
			"literalString" in value ||
			"literalNumber" in value ||
			"literalBool" in value ||
			"literalJson" in value ||
			"literalOptions" in value ||
			"path" in value
		) {
			return (
				<BoundValueEditor
					name={name}
					value={value as BoundValue}
					onChange={onChange}
					isAssetProperty={isAssetProperty}
					appId={appId}
					assetAccept={assetAccept}
					componentType={componentType}
					enumOptions={enumOptions}
				/>
			);
		}
		return null;
	}

	if (typeof value === "string") {
		// Use AssetPicker for asset properties when appId is available
		if (isAssetProperty && appId) {
			return (
				<div className={INSPECTOR_FIELD_CLASS}>
					<Label className="text-xs capitalize">
						{name.replace(/([A-Z])/g, " $1")}
					</Label>
					<AssetPicker
						appId={appId}
						value={value}
						onChange={(newValue) => onChange(newValue)}
						accept={assetAccept}
						placeholder={`Select ${name}...`}
					/>
				</div>
			);
		}
		return (
			<div className={INSPECTOR_FIELD_CLASS}>
				<Label className="text-xs capitalize">
					{name.replace(/([A-Z])/g, " $1")}
				</Label>
				<Input
					value={value}
					onChange={(e) => onChange(e.target.value)}
					className="h-8 text-sm"
				/>
			</div>
		);
	}

	if (typeof value === "number") {
		return (
			<div className={INSPECTOR_FIELD_CLASS}>
				<Label className="text-xs capitalize">
					{name.replace(/([A-Z])/g, " $1")}
				</Label>
				<Input
					type="number"
					value={value}
					onChange={(e) => onChange(Number(e.target.value))}
					className="h-8 text-sm"
				/>
			</div>
		);
	}

	if (typeof value === "boolean") {
		return (
			<div className="flex items-center justify-between py-1">
				<Label className="text-xs capitalize">
					{name.replace(/([A-Z])/g, " $1")}
				</Label>
				<Switch checked={value} onCheckedChange={onChange} />
			</div>
		);
	}

	return null;
}

interface OptionsEditorProps {
	name: string;
	options: SelectOption[];
	onChange: (options: SelectOption[]) => void;
	onSwitchToBinding: () => void;
}

function OptionsEditor({
	name,
	options,
	onChange,
	onSwitchToBinding,
}: OptionsEditorProps) {
	const { t } = useTranslation("flow");
	const addOption = useCallback(() => {
		onChange([
			...options,
			{
				value: `option${options.length + 1}`,
				label: t("optionVal", "Option {{val}}", { val: options.length + 1 }),
			},
		]);
	}, [options, onChange]);

	const removeOption = useCallback(
		(index: number) => {
			onChange(options.filter((_, i) => i !== index));
		},
		[options, onChange],
	);

	const updateOption = useCallback(
		(index: number, field: "value" | "label", val: string) => {
			const updated = [...options];
			updated[index] = { ...updated[index], [field]: val };
			onChange(updated);
		},
		[options, onChange],
	);

	return (
		<div className={INSPECTOR_FIELD_CLASS}>
			<div className="flex items-center justify-between">
				<Label className="text-xs capitalize">
					{name.replace(/([A-Z])/g, " $1")}
				</Label>
				<Select
					value="literal"
					onValueChange={(v) => v === "binding" && onSwitchToBinding()}
				>
					<SelectTrigger className="h-6 w-20 text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="literal">{t("literal", "Literal")}</SelectItem>
						<SelectItem value="binding">{t("binding", "Binding")}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			<div className="min-w-0 max-w-full space-y-1.5 rounded border p-2">
				{options.map((opt, idx) => (
					<div key={idx} className="flex items-center gap-1">
						<Input
							value={opt.value}
							onChange={(e) => updateOption(idx, "value", e.target.value)}
							placeholder="value"
							className="h-7 text-xs flex-1"
						/>
						<Input
							value={opt.label}
							onChange={(e) => updateOption(idx, "label", e.target.value)}
							placeholder="label"
							className="h-7 text-xs flex-1"
						/>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7 shrink-0"
							onClick={() => removeOption(idx)}
						>
							<Trash2 className="h-3 w-3" />
						</Button>
					</div>
				))}
				<Button
					variant="outline"
					size="sm"
					className="w-full h-7 text-xs"
					onClick={addOption}
				>
					<Plus className="h-3 w-3 mr-1" />
					{t("addOption", "Add Option")}
				</Button>
			</div>
		</div>
	);
}

// Nivo chart types
const NIVO_CHART_TYPES = [
	{ value: "bar", label: "Bar" },
	{ value: "line", label: "Line" },
	{ value: "pie", label: "Pie" },
	{ value: "radar", label: "Radar" },
	{ value: "heatmap", label: "Heatmap" },
	{ value: "scatter", label: "Scatter" },
	{ value: "funnel", label: "Funnel" },
	{ value: "treemap", label: "Treemap" },
	{ value: "sunburst", label: "Sunburst" },
	{ value: "calendar", label: "Calendar" },
	{ value: "bump", label: "Bump" },
	{ value: "areaBump", label: "Area Bump" },
	{ value: "circlePacking", label: "Circle Packing" },
	{ value: "network", label: "Network" },
	{ value: "sankey", label: "Sankey" },
	{ value: "stream", label: "Stream" },
	{ value: "swarmplot", label: "Swarmplot" },
	{ value: "voronoi", label: "Voronoi" },
	{ value: "waffle", label: "Waffle" },
	{ value: "marimekko", label: "Marimekko" },
	{ value: "parallelCoordinates", label: "Parallel Coordinates" },
	{ value: "radialBar", label: "Radial Bar" },
	{ value: "boxplot", label: "Boxplot" },
	{ value: "bullet", label: "Bullet" },
	{ value: "chord", label: "Chord" },
];

// Plotly chart types
const PLOTLY_CHART_TYPES = [
	{ value: "line", label: "Line" },
	{ value: "bar", label: "Bar" },
	{ value: "scatter", label: "Scatter" },
	{ value: "area", label: "Area" },
	{ value: "pie", label: "Pie" },
	{ value: "histogram", label: "Histogram" },
];

interface BoundValueEditorProps {
	name: string;
	value: BoundValue;
	onChange: (value: BoundValue) => void;
	isAssetProperty?: boolean;
	appId?: string;
	assetAccept?: AssetAccept;
	componentType?: string;
	enumOptions?: string[];
	label?: string;
}

function BoundValueEditor({
	name,
	value,
	onChange,
	isAssetProperty,
	appId,
	assetAccept = "all",
	componentType,
	enumOptions,
	label,
}: BoundValueEditorProps) {
	const { t } = useTranslation("flow");
	const [mode, setMode] = useState<"literal" | "binding">(
		"path" in value ? "binding" : "literal",
	);

	// Track the original literal value for use as default when binding
	const [cachedLiteralValue, setCachedLiteralValue] = useState<
		string | number | boolean | undefined
	>(() => {
		if ("literalString" in value) return value.literalString;
		if ("literalNumber" in value) return value.literalNumber;
		if ("literalBool" in value) return value.literalBool;
		if ("literalJson" in value) return value.literalJson as string;
		if (
			"path" in value &&
			(typeof value.defaultValue === "string" ||
				typeof value.defaultValue === "number" ||
				typeof value.defaultValue === "boolean")
		) {
			return value.defaultValue;
		}
		return undefined;
	});

	// Determine original value type
	const originalType = useMemo(() => {
		if ("literalNumber" in value) return "number";
		if ("literalBool" in value) return "boolean";
		if ("literalString" in value) return "string";
		if ("literalJson" in value) return "json";
		if ("literalOptions" in value) return "options";
		// Infer from defaultValue if we're in binding mode
		if ("path" in value && value.defaultValue !== undefined) {
			if (typeof value.defaultValue === "number") return "number";
			if (typeof value.defaultValue === "boolean") return "boolean";
		}
		return "string";
	}, [value]);

	const currentValue = useMemo(() => {
		if ("literalString" in value) return value.literalString;
		if ("literalNumber" in value) return value.literalNumber;
		if ("literalBool" in value) return value.literalBool;
		if ("literalJson" in value) return value.literalJson as string;
		if ("literalOptions" in value) return value.literalOptions;
		if ("path" in value) return value.path;
		return "";
	}, [value]);

	// Update cached literal value when in literal mode
	const handleLiteralChange = useCallback(
		(newValue: string | number | boolean) => {
			setCachedLiteralValue(newValue);
			if (originalType === "number") {
				const num = typeof newValue === "number" ? newValue : Number(newValue);
				onChange({ literalNumber: Number.isNaN(num) ? 0 : num });
			} else if (originalType === "boolean") {
				onChange({ literalBool: Boolean(newValue) });
			} else if (originalType === "json") {
				onChange({ literalJson: String(newValue) });
			} else {
				onChange({ literalString: String(newValue) });
			}
		},
		[onChange, originalType],
	);

	// Handle path change while preserving default value
	const handlePathChange = useCallback(
		(path: string) => {
			onChange({ path, defaultValue: cachedLiteralValue });
		},
		[onChange, cachedLiteralValue],
	);

	// Handle mode switch
	const handleModeChange = useCallback(
		(newMode: "literal" | "binding") => {
			setMode(newMode);
			if (newMode === "binding") {
				// Switching to binding - create path with current literal as default
				onChange({ path: "", defaultValue: cachedLiteralValue });
			} else {
				// Switching to literal - restore cached value or use default
				const restoreValue =
					"path" in value ? value.defaultValue : cachedLiteralValue;
				if (originalType === "number") {
					onChange({
						literalNumber: typeof restoreValue === "number" ? restoreValue : 0,
					});
				} else if (originalType === "boolean") {
					onChange({
						literalBool:
							typeof restoreValue === "boolean" ? restoreValue : false,
					});
				} else if (originalType === "json") {
					onChange({
						literalJson: typeof restoreValue === "string" ? restoreValue : "[]",
					});
				} else {
					onChange({
						literalString: typeof restoreValue === "string" ? restoreValue : "",
					});
				}
			}
		},
		[value, cachedLiteralValue, onChange, originalType],
	);

	// For options type, render special editor
	if (originalType === "options" && mode === "literal") {
		return (
			<OptionsEditor
				name={name}
				options={"literalOptions" in value ? value.literalOptions : []}
				onChange={(opts) => onChange({ literalOptions: opts })}
				onSwitchToBinding={() => handleModeChange("binding")}
			/>
		);
	}

	// When switching to binding mode from options
	if (originalType === "options" && mode === "binding") {
		return (
			<div className={INSPECTOR_FIELD_CLASS}>
				<div className="flex items-center justify-between">
					<Label className="text-xs capitalize">
						{(label ?? name).replace(/([A-Z])/g, " $1")}
					</Label>
					<Select
						value={mode}
						onValueChange={(v) => handleModeChange(v as "literal" | "binding")}
					>
						<SelectTrigger className="h-6 w-20 text-xs">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="literal">{t("literal", "Literal")}</SelectItem>
							<SelectItem value="binding">{t("binding", "Binding")}</SelectItem>
						</SelectContent>
					</Select>
				</div>
				<Input
					value={"path" in value ? value.path : ""}
					onChange={(e) => handlePathChange(e.target.value)}
					placeholder="/path/to/options"
					className="h-8 text-sm"
				/>
			</div>
		);
	}

	return (
		<div className={INSPECTOR_FIELD_CLASS}>
			<div className="flex min-w-0 items-center justify-between gap-2">
				<Label className="text-xs capitalize">
					{(label ?? name).replace(/([A-Z])/g, " $1")}
				</Label>
				<Select
					value={mode}
					onValueChange={(v) => handleModeChange(v as "literal" | "binding")}
				>
					<SelectTrigger className="h-6 w-20 text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="literal">{t("literal", "Literal")}</SelectItem>
						<SelectItem value="binding">{t("binding", "Binding")}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			{mode === "literal" && originalType === "boolean" ? (
				<Select
					value={String(currentValue)}
					onValueChange={(v) => handleLiteralChange(v === "true")}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="true">{t("true", "True")}</SelectItem>
						<SelectItem value="false">{t("false", "False")}</SelectItem>
					</SelectContent>
				</Select>
			) : mode === "literal" && originalType === "json" ? (
				<Textarea
					value={String(currentValue)}
					onChange={(e) => handleLiteralChange(e.target.value)}
					placeholder={`[0, 0, 0]`}
					wrap="soft"
					style={FIXED_FIELD_SIZING_STYLE}
					className="min-h-20 max-h-56 min-w-0 max-w-full resize-y overflow-auto whitespace-pre-wrap break-all font-mono text-xs"
				/>
			) : mode === "literal" && enumOptions && enumOptions.length > 0 ? (
				<Select
					value={String(currentValue)}
					onValueChange={(v) => handleLiteralChange(v)}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{enumOptions.map((option) => (
							<SelectItem key={option} value={option}>
								{option}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : mode === "literal" &&
				isAssetProperty &&
				appId &&
				originalType === "string" ? (
				<AssetPicker
					appId={appId}
					value={String(currentValue)}
					onChange={(newValue) => handleLiteralChange(newValue)}
					accept={assetAccept}
					placeholder={`Select ${name}...`}
				/>
			) : mode === "literal" &&
				name === "chartType" &&
				componentType === "nivoChart" ? (
				<Select
					value={String(currentValue)}
					onValueChange={(v) => handleLiteralChange(v)}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{NIVO_CHART_TYPES.map((ct) => (
							<SelectItem key={ct.value} value={ct.value}>
								{ct.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : mode === "literal" &&
				name === "chartType" &&
				componentType === "plotlyChart" ? (
				<Select
					value={String(currentValue)}
					onValueChange={(v) => handleLiteralChange(v)}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{PLOTLY_CHART_TYPES.map((ct) => (
							<SelectItem key={ct.value} value={ct.value}>
								{ct.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : mode === "literal" &&
				originalType === "string" &&
				/color$/i.test(name) ? (
				<div className="flex items-center gap-2">
					<Input
						type="color"
						value={
							/^#[0-9a-fA-F]{6}$/.test(String(currentValue))
								? String(currentValue)
								: "#000000"
						}
						onChange={(e) => handleLiteralChange(e.target.value)}
						className="h-8 w-10 shrink-0 cursor-pointer p-1"
						aria-label={t("nameColor", "{{name}} color", { name })}
					/>
					<Input
						type="text"
						value={String(currentValue)}
						onChange={(e) => handleLiteralChange(e.target.value)}
						placeholder={`#8b5cf6`}
						className="h-8 flex-1 text-sm"
					/>
				</div>
			) : (
				<Input
					type={
						mode === "literal" && originalType === "number" ? "number" : "text"
					}
					value={String(currentValue)}
					onChange={(e) =>
						mode === "binding"
							? handlePathChange(e.target.value)
							: handleLiteralChange(
									originalType === "number"
										? e.target.valueAsNumber
										: e.target.value,
								)
					}
					placeholder={
						mode === "binding"
							? "/path/to/data"
							: t("enterValue", "Enter value...")
					}
					className="h-8 text-sm"
				/>
			)}
		</div>
	);
}

interface StyleEditorProps {
	component: SurfaceComponent;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
}

function StyleEditor({ component, onUpdate }: StyleEditorProps) {
	const { t } = useTranslation("flow");
	const style = component.style || {};
	const [responsiveBreakpoint, setResponsiveBreakpoint] =
		useState<keyof ResponsiveOverrides>("md");

	const updateStyle = useCallback(
		<K extends keyof Style>(key: K, value: Style[K]) => {
			onUpdate({
				style: normalizeStyleUpdate({
					...style,
					[key]: value,
				}),
			});
		},
		[style, onUpdate],
	);

	const updateBreakpointStyle = useCallback(
		<K extends keyof BreakpointStyle>(key: K, value: BreakpointStyle[K]) => {
			const responsive = style.responsiveOverrides ?? style.responsive ?? {};
			updateStyle("responsiveOverrides", {
				...responsive,
				[responsiveBreakpoint]: {
					...responsive[responsiveBreakpoint],
					[key]: value,
				},
			});
		},
		[
			responsiveBreakpoint,
			style.responsive,
			style.responsiveOverrides,
			updateStyle,
		],
	);

	const breakpointStyle =
		(style.responsiveOverrides ?? style.responsive)?.[responsiveBreakpoint] ??
		{};
	const backgroundMode = !style.background
		? "none"
		: "color" in style.background
			? "color"
			: "gradient" in style.background
				? "gradient"
				: "image" in style.background
					? "image"
					: "blur";
	const gradient =
		style.background && "gradient" in style.background
			? style.background.gradient
			: undefined;
	const gradientType = gradient?.type ?? gradient?.gradientType ?? "linear";
	const gradientStops = gradient?.stops
		? gradient.stops.map((stop) => ({
				...stop,
				position:
					gradient.type === undefined &&
					stop.position !== undefined &&
					stop.position >= 0 &&
					stop.position <= 1
						? stop.position * 100
						: stop.position,
			}))
		: [
				{ color: "#000000", position: 0 },
				{ color: "#ffffff", position: 100 },
			];
	const backgroundImage =
		style.background && "image" in style.background
			? style.background.image
			: undefined;

	return (
		<div className="space-y-4">
			{/* Spacing - Margin & Padding */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("spacing", "Spacing")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					{(["margin", "padding"] as const).map((property) => {
						const edges = getSpacingEdges(style[property]);
						return (
							<div className="space-y-1" key={property}>
								<Label className="text-xs capitalize">{property}</Label>
								<div className="grid grid-cols-4 gap-1">
									{(["top", "right", "bottom", "left"] as const).map((side) => (
										<Input
											key={side}
											value={edges[side] ?? ""}
											onChange={(e) =>
												updateStyle(
													property,
													withSpacingSide(
														style[property],
														side,
														e.target.value,
													),
												)
											}
											placeholder={side[0]?.toUpperCase()}
											className="h-7 text-xs text-center"
										/>
									))}
								</div>
								<p className="text-[10px] text-muted-foreground">
									{t("topRightBottomLeft", "Top, Right, Bottom, Left")}
								</p>
							</div>
						);
					})}
				</CollapsibleContent>
			</Collapsible>

			{/* Sizing */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("size", "Size")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("width", "Width")}</Label>
							<Input
								value={getStyleValue(style.width)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("width", e.target.value || undefined)
								}
								placeholder="auto"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("height", "Height")}</Label>
							<Input
								value={getStyleValue(style.height)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("height", e.target.value || undefined)
								}
								placeholder="auto"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("minWidth", "Min Width")}</Label>
							<Input
								value={getStyleValue(style.minWidth)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("minWidth", e.target.value || undefined)
								}
								placeholder="0"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("minHeight", "Min Height")}</Label>
							<Input
								value={getStyleValue(style.minHeight)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("minHeight", e.target.value || undefined)
								}
								placeholder="0"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("maxWidth", "Max Width")}</Label>
							<Input
								value={getStyleValue(style.maxWidth)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("maxWidth", e.target.value || undefined)
								}
								placeholder="none"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("maxHeight", "Max Height")}</Label>
							<Input
								value={getStyleValue(style.maxHeight)}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("maxHeight", e.target.value || undefined)
								}
								placeholder="none"
								className="h-7 text-xs"
							/>
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Tailwind Classes */}
			<div className="space-y-2">
				<Label className="text-xs">
					{t("tailwindClasses", "Tailwind Classes")}
				</Label>
				<Textarea
					value={style.className || ""}
					onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) =>
						updateStyle("className", e.target.value)
					}
					placeholder={t("enterTailwindClasses", "Enter Tailwind classes")}
					autoComplete="off"
					autoCorrect="off"
					autoCapitalize="off"
					className="text-sm min-h-[60px]"
				/>
				<p className="text-xs text-muted-foreground">
					{t("additionalUtilityClasses", "Additional utility classes")}
				</p>
			</div>

			{/* Position */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("position", "Position")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">Type</Label>
						<Select
							value={getPositionType(style.position)}
							onValueChange={(v) =>
								updateStyle(
									"position",
									withPositionType(
										style.position,
										v as NonNullable<Position["type"]>,
									),
								)
							}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue
									placeholder={t("selectPosition", "Select position")}
								/>
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="relative">
									{t("relative", "Relative")}
								</SelectItem>
								<SelectItem value="absolute">
									{t("absolute", "Absolute")}
								</SelectItem>
								<SelectItem value="fixed">{t("fixed", "Fixed")}</SelectItem>
								<SelectItem value="sticky">{t("sticky", "Sticky")}</SelectItem>
							</SelectContent>
						</Select>
					</div>
					{style.position && getPositionType(style.position) !== "relative" && (
						<div className="grid grid-cols-2 gap-2">
							<div className="space-y-1">
								<Label className="text-xs">{t("top", "Top")}</Label>
								<Input
									value={style.position?.top || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...(style.position ?? {}),
											type: getPositionType(style.position),
											top: e.target.value,
										})
									}
									placeholder="0"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">{t("right", "Right")}</Label>
								<Input
									value={style.position?.right || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...(style.position ?? {}),
											type: getPositionType(style.position),
											right: e.target.value,
										})
									}
									placeholder="auto"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">{t("bottom", "Bottom")}</Label>
								<Input
									value={style.position?.bottom || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...(style.position ?? {}),
											type: getPositionType(style.position),
											bottom: e.target.value,
										})
									}
									placeholder="auto"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">{t("left", "Left")}</Label>
								<Input
									value={style.position?.left || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...(style.position ?? {}),
											type: getPositionType(style.position),
											left: e.target.value,
										})
									}
									placeholder="auto"
									className="h-8 text-sm"
								/>
							</div>
						</div>
					)}
				</CollapsibleContent>
			</Collapsible>

			{/* Background */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("background", "Background")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">Type</Label>
						<Select
							value={backgroundMode}
							onValueChange={(value) => {
								switch (value) {
									case "color":
										updateStyle("background", { color: "#ffffff" });
										break;
									case "gradient":
										updateStyle("background", {
											gradient: {
												type: "linear",
												angle: 180,
												stops: gradientStops,
											},
										});
										break;
									case "image":
										updateStyle("background", {
											image: {
												url: { literalString: "" },
												size: "cover",
												position: "center",
												repeat: "no-repeat",
											},
										});
										break;
									case "blur":
										updateStyle("background", { blur: "4px" });
										break;
									default:
										updateStyle("background", undefined);
								}
							}}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="none">{t("none", "None")}</SelectItem>
								<SelectItem value="color">{t("color", "Color")}</SelectItem>
								<SelectItem value="gradient">
									{t("gradient", "Gradient")}
								</SelectItem>
								<SelectItem value="image">{t("image", "Image")}</SelectItem>
								<SelectItem value="blur">
									{t("backdropBlur", "Backdrop blur")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>

					{backgroundMode === "color" && (
						<div className="space-y-1">
							<Label className="text-xs">{t("color", "Color")}</Label>
							<Input
								value={
									style.background && "color" in style.background
										? style.background.color
										: ""
								}
								onChange={(e) =>
									updateStyle("background", { color: e.target.value })
								}
								placeholder={t("ffffffOrTransparent", "#ffffff or transparent")}
								className="h-8 text-sm"
							/>
						</div>
					)}

					{backgroundMode === "gradient" && (
						<div className="space-y-3">
							<div className="grid grid-cols-2 gap-2">
								<div className="space-y-1">
									<Label className="text-xs">
										{t("gradientType", "Gradient type")}
									</Label>
									<Select
										value={gradientType}
										onValueChange={(value) =>
											updateStyle("background", {
												gradient: {
													type: value as "linear" | "radial" | "conic",
													angle: gradient?.angle,
													direction: gradient?.direction,
													stops: gradientStops,
												},
											})
										}
									>
										<SelectTrigger className="h-8 text-sm">
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value="linear">
												{t("linear", "Linear")}
											</SelectItem>
											<SelectItem value="radial">
												{t("radial", "Radial")}
											</SelectItem>
											<SelectItem value="conic">
												{t("conic", "Conic")}
											</SelectItem>
										</SelectContent>
									</Select>
								</div>
								<div className="space-y-1">
									<Label className="text-xs">
										{t("direction", "Direction")}
									</Label>
									<Input
										value={
											gradient?.direction ??
											(gradient?.angle === undefined
												? ""
												: `${gradient.angle}deg`)
										}
										onChange={(e) =>
											updateStyle("background", {
												gradient: {
													type: gradientType,
													angle: undefined,
													direction: e.target.value || undefined,
													stops: gradientStops,
												},
											})
										}
										placeholder={t("toRightOr45deg", "to right or 45deg")}
										className="h-8 text-sm"
									/>
								</div>
							</div>
							{gradientStops.map((stop, index) => (
								<div key={`${index}-${stop.position}`} className="flex gap-2">
									<Input
										value={stop.color}
										onChange={(e) => {
											const stops = gradientStops.map((item, itemIndex) =>
												itemIndex === index
													? { ...item, color: e.target.value }
													: item,
											);
											updateStyle("background", {
												gradient: {
													type: gradientType,
													angle: gradient?.angle,
													direction: gradient?.direction,
													stops,
												},
											});
										}}
										placeholder="#000000"
										className="h-8 text-sm flex-1"
									/>
									<Input
										type="number"
										min="0"
										max="100"
										step="1"
										value={stop.position}
										onChange={(e) => {
											const stops = gradientStops.map((item, itemIndex) =>
												itemIndex === index
													? { ...item, position: Number(e.target.value) }
													: item,
											);
											updateStyle("background", {
												gradient: {
													type: gradientType,
													angle: gradient?.angle,
													direction: gradient?.direction,
													stops,
												},
											});
										}}
										className="h-8 text-sm w-20"
									/>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="h-8 w-8"
										disabled={gradientStops.length <= 2}
										onClick={() =>
											updateStyle("background", {
												gradient: {
													type: gradientType,
													angle: gradient?.angle,
													direction: gradient?.direction,
													stops: gradientStops.filter(
														(_, itemIndex) => itemIndex !== index,
													),
												},
											})
										}
									>
										<Trash2 className="h-3.5 w-3.5" />
									</Button>
								</div>
							))}
							<Button
								type="button"
								variant="outline"
								size="sm"
								className="h-7 text-xs"
								onClick={() =>
									updateStyle("background", {
										gradient: {
											type: gradientType,
											angle: gradient?.angle,
											direction: gradient?.direction,
											stops: [
												...gradientStops,
												{ color: "#ffffff", position: 100 },
											],
										},
									})
								}
							>
								<Plus className="mr-1 h-3.5 w-3.5" />
								{t("addStop", "Add stop")}
							</Button>
						</div>
					)}

					{backgroundMode === "image" && backgroundImage && (
						<div className="space-y-3">
							<div className="space-y-1">
								<Label className="text-xs">
									{t("urlSource", "URL source")}
								</Label>
								<Select
									value={"path" in backgroundImage.url ? "path" : "literal"}
									onValueChange={(value) =>
										updateStyle("background", {
											image: {
												...backgroundImage,
												url:
													value === "path"
														? { path: "", defaultValue: "" }
														: { literalString: "" },
											},
										})
									}
								>
									<SelectTrigger className="h-8 text-sm">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="literal">
											{t("literalUrl", "Literal URL")}
										</SelectItem>
										<SelectItem value="path">
											{t("dataPath", "Data path")}
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">
									{"path" in backgroundImage.url ? "Data path" : "Image URL"}
								</Label>
								<Input
									value={
										"path" in backgroundImage.url
											? backgroundImage.url.path
											: "literalString" in backgroundImage.url
												? backgroundImage.url.literalString
												: ""
									}
									onChange={(e) =>
										updateStyle("background", {
											image: {
												...backgroundImage,
												url:
													"path" in backgroundImage.url
														? { ...backgroundImage.url, path: e.target.value }
														: { literalString: e.target.value },
											},
										})
									}
									placeholder={
										"path" in backgroundImage.url
											? "/theme/heroImage"
											: "/path/to/image.jpg"
									}
									className="h-8 text-sm"
								/>
							</div>
							{"path" in backgroundImage.url && (
								<div className="space-y-1">
									<Label className="text-xs">
										{t("fallbackUrl", "Fallback URL")}
									</Label>
									<Input
										value={
											typeof backgroundImage.url.defaultValue === "string"
												? backgroundImage.url.defaultValue
												: ""
										}
										onChange={(e) =>
											updateStyle("background", {
												image: {
													...backgroundImage,
													url: {
														...backgroundImage.url,
														defaultValue: e.target.value,
													},
												},
											})
										}
										className="h-8 text-sm"
									/>
								</div>
							)}
							<div className="grid grid-cols-3 gap-2">
								{(["size", "position", "repeat"] as const).map((field) => (
									<div className="space-y-1" key={field}>
										<Label className="text-xs capitalize">{field}</Label>
										<Input
											value={backgroundImage[field] ?? ""}
											onChange={(e) =>
												updateStyle("background", {
													image: {
														...backgroundImage,
														[field]: e.target.value || undefined,
													},
												})
											}
											className="h-8 text-xs"
										/>
									</div>
								))}
							</div>
						</div>
					)}

					{backgroundMode === "blur" && (
						<div className="space-y-1">
							<Label className="text-xs">
								{t("backdropBlur2", "Backdrop Blur")}
							</Label>
							<Input
								value={
									style.background && "blur" in style.background
										? style.background.blur
										: ""
								}
								onChange={(e) =>
									updateStyle("background", { blur: e.target.value })
								}
								placeholder="4px"
								className="h-8 text-sm"
							/>
						</div>
					)}
					<div className="space-y-1">
						<Label className="text-xs">{t("opacity", "Opacity")}</Label>
						<Input
							type="number"
							step="0.1"
							min="0"
							max="1"
							value={style.opacity ?? ""}
							onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
								updateStyle(
									"opacity",
									e.target.value
										? Number.parseFloat(e.target.value)
										: undefined,
								)
							}
							placeholder="1"
							className="h-8 text-sm"
						/>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Border */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("border", "Border")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("width", "Width")}</Label>
							<Input
								value={style.border?.width || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("border", {
										...style.border,
										width: e.target.value,
									})
								}
								placeholder="1px"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("radius", "Radius")}</Label>
							<Input
								value={style.border?.radius || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("border", {
										...style.border,
										radius: e.target.value,
									})
								}
								placeholder="4px"
								className="h-8 text-sm"
							/>
						</div>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">{t("color", "Color")}</Label>
						<div className="flex gap-2">
							<Input
								type="color"
								value={style.border?.color || "#e5e7eb"}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("border", {
										...style.border,
										color: e.target.value,
									})
								}
								className="h-8 w-12 p-1"
							/>
							<Input
								value={style.border?.color || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("border", {
										...style.border,
										color: e.target.value,
									})
								}
								placeholder={`#e5e7eb`}
								className="h-8 text-sm flex-1"
							/>
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Shadow */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("shadow", "Shadow")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						{(
							[
								["x", "X offset", "0"],
								["y", "Y offset", "2px"],
								["blur", "Blur", "4px"],
								["spread", "Spread", "0"],
								["color", "Color", "rgba(0,0,0,0.1)"],
							] as const
						).map(([field, label, placeholder]) => (
							<div className="space-y-1" key={field}>
								<Label className="text-xs">{label}</Label>
								<Input
									value={style.shadow?.[field] ?? ""}
									onChange={(e) =>
										updateStyle("shadow", {
											...withoutLegacyBoxShadow(style.shadow),
											[field]: e.target.value || undefined,
										})
									}
									placeholder={placeholder}
									className="h-8 text-sm"
								/>
							</div>
						))}
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("inset", "Inset")}</Label>
						<Switch
							checked={style.shadow?.inset ?? false}
							onCheckedChange={(checked) =>
								updateStyle("shadow", {
									...withoutLegacyBoxShadow(style.shadow),
									inset: checked || undefined,
								})
							}
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">{t("textShadow", "Text Shadow")}</Label>
						<Input
							value={style.shadow?.textShadow || ""}
							onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
								updateStyle("shadow", {
									...withoutLegacyBoxShadow(style.shadow),
									textShadow: e.target.value || undefined,
								})
							}
							placeholder={`0 1px 2px rgba(0,0,0,0.2)`}
							className="h-8 text-sm"
						/>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Transform */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("transform", "Transform")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">{t("translate", "Translate")}</Label>
							<Input
								value={style.transform?.translate || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										translate: e.target.value,
									})
								}
								placeholder={`0, 0`}
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">
								{t("rotateDeg", "Rotate (deg)")}
							</Label>
							<Input
								type="number"
								value={style.transform?.rotate ?? ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										rotate: Number(e.target.value),
									})
								}
								placeholder="0"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("scale", "Scale")}</Label>
							<Input
								value={style.transform?.scale || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										scale: e.target.value,
									})
								}
								placeholder="1"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("skew", "Skew")}</Label>
							<Input
								value={style.transform?.skew || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										skew: e.target.value,
									})
								}
								placeholder={`10deg, 5deg`}
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("origin", "Origin")}</Label>
							<Input
								value={style.transform?.transformOrigin || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										transformOrigin: e.target.value,
									})
								}
								placeholder="center"
								className="h-8 text-sm"
							/>
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Responsive overrides */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>{t("responsive", "Responsive")}</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">{t("breakpoint", "Breakpoint")}</Label>
						<Select
							value={responsiveBreakpoint}
							onValueChange={(value) =>
								setResponsiveBreakpoint(value as keyof ResponsiveOverrides)
							}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="sm">{t("sm640px", "sm · 640px")}</SelectItem>
								<SelectItem value="md">{t("md768px", "md · 768px")}</SelectItem>
								<SelectItem value="lg">
									{t("lg1024px", "lg · 1024px")}
								</SelectItem>
								<SelectItem value="xl">
									{t("xl1280px", "xl · 1280px")}
								</SelectItem>
								<SelectItem value="xxl">
									{t("2xl1536px", "2xl · 1536px")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">
							{t("tailwindClasses2", "Tailwind classes")}
						</Label>
						<Input
							value={breakpointStyle.className ?? ""}
							onChange={(e) =>
								updateBreakpointStyle("className", e.target.value || undefined)
							}
							className="h-8 text-sm"
						/>
					</div>
					<div className="grid grid-cols-2 gap-2">
						{(
							[
								["display", "Display", "flex"],
								["flexDirection", "Flex direction", "row"],
								["justifyContent", "Justify", "center"],
								[`alignItems`, "Align", "center"],
								["gap", "Gap", "16px"],
								["fontSize", "Font size", "1rem"],
								["textAlign", "Text align", "center"],
							] as const
						).map(([field, label, placeholder]) => (
							<div className="space-y-1" key={field}>
								<Label className="text-xs">{label}</Label>
								<Input
									value={String(breakpointStyle[field] ?? "")}
									onChange={(e) =>
										updateBreakpointStyle(field, e.target.value || undefined)
									}
									placeholder={placeholder}
									className="h-8 text-xs"
								/>
							</div>
						))}
						{(
							[
								["width", "Width", "100%"],
								["height", "Height", "auto"],
							] as const
						).map(([field, label, placeholder]) => (
							<div className="space-y-1" key={field}>
								<Label className="text-xs">{label}</Label>
								<Input
									value={getStyleValue(breakpointStyle[field])}
									onChange={(e) =>
										updateBreakpointStyle(field, e.target.value || undefined)
									}
									placeholder={placeholder}
									className="h-8 text-xs"
								/>
							</div>
						))}
						{(
							[
								["padding", "Padding"],
								["margin", "Margin"],
							] as const
						).map(([field, label]) => (
							<div className="space-y-1" key={field}>
								<Label className="text-xs">{label}</Label>
								<Input
									value={getSpacingValue(breakpointStyle[field])}
									onChange={(e) =>
										updateBreakpointStyle(
											field,
											spacingFromShorthand(e.target.value),
										)
									}
									placeholder={`8px 16px`}
									className="h-8 text-xs"
								/>
							</div>
						))}
						<div className="space-y-1">
							<Label className="text-xs">
								{t("gridColumns", "Grid columns")}
							</Label>
							<Input
								type="number"
								min="1"
								value={breakpointStyle.gridCols ?? ""}
								onChange={(e) =>
									updateBreakpointStyle(
										"gridCols",
										e.target.value ? Number(e.target.value) : undefined,
									)
								}
								className="h-8 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">{t("order", "Order")}</Label>
							<Input
								type="number"
								value={breakpointStyle.order ?? ""}
								onChange={(e) =>
									updateBreakpointStyle(
										"order",
										e.target.value ? Number(e.target.value) : undefined,
									)
								}
								className="h-8 text-xs"
							/>
						</div>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">
							{t("hiddenAtThisBreakpoint", "Hidden at this breakpoint")}
						</Label>
						<Switch
							checked={breakpointStyle.hidden ?? false}
							onCheckedChange={(checked) =>
								updateBreakpointStyle("hidden", checked || undefined)
							}
						/>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Overflow */}
			<div className="space-y-2">
				<Label className="text-xs">{t("overflow", "Overflow")}</Label>
				<Select
					value={style.overflow || "visible"}
					onValueChange={(v) => updateStyle("overflow", v as Overflow)}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue placeholder={t("selectOverflow", "Select overflow")} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="visible">{t("visible", "Visible")}</SelectItem>
						<SelectItem value="hidden">{t("hidden", "Hidden")}</SelectItem>
						<SelectItem value="scroll">{t("scroll", "Scroll")}</SelectItem>
						<SelectItem value="auto">{t("auto", "Auto")}</SelectItem>
					</SelectContent>
				</Select>
			</div>
		</div>
	);
}

// Canvas settings editor - global canvas settings
function CanvasSettingsEditor() {
	const { t } = useTranslation("flow");
	const { canvasSettings, setCanvasSettings } = useBuilder();

	return (
		<div className="space-y-4">
			<p className="text-xs text-muted-foreground mb-4">
				{t(
					"theseSettingsApplyToTheEntireCanvasBackgroundNotIndividualComponents",
					"These settings apply to the entire canvas background, not individual components.",
				)}
			</p>

			{/* Canvas Background Color */}
			<div className="space-y-2">
				<Label className="text-xs">
					{t("canvasBackground", "Canvas Background")}
				</Label>
				<div className="flex gap-2">
					<Input
						type="color"
						value={canvasSettings.backgroundColor || "#ffffff"}
						onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
							setCanvasSettings({ backgroundColor: e.target.value })
						}
						className="h-8 w-12 p-1"
					/>
					<Input
						value={canvasSettings.backgroundColor || ""}
						onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
							setCanvasSettings({ backgroundColor: e.target.value })
						}
						placeholder="#ffffff"
						className="h-8 text-sm flex-1"
					/>
				</div>
			</div>

			{/* Canvas Background Image */}
			<div className="space-y-2">
				<Label className="text-xs">
					{t("backgroundImageUrl", "Background Image URL")}
				</Label>
				<Input
					value={canvasSettings.backgroundImage || ""}
					onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
						setCanvasSettings({ backgroundImage: e.target.value || undefined })
					}
					placeholder="/path/to/image.jpg"
					className="h-8 text-sm"
				/>
			</div>

			{/* Canvas Padding */}
			<div className="space-y-2">
				<Label className="text-xs">
					{t("canvasPadding", "Canvas Padding")}
				</Label>
				<Select
					value={canvasSettings.padding || "16px"}
					onValueChange={(v) => setCanvasSettings({ padding: v })}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue placeholder={t("selectPadding", "Select padding")} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="0">{t("none", "None")}</SelectItem>
						<SelectItem value="8px">{t("small8px", "Small (8px)")}</SelectItem>
						<SelectItem value="16px">
							{t("medium16px", "Medium (16px)")}
						</SelectItem>
						<SelectItem value="24px">
							{t("large24px", "Large (24px)")}
						</SelectItem>
						<SelectItem value="32px">
							{t("extraLarge32px", "Extra Large (32px)")}
						</SelectItem>
					</SelectContent>
				</Select>
			</div>

			{/* Custom CSS */}
			<div className="space-y-2">
				<Label className="text-xs">{t("customCss", "Custom CSS")}</Label>
				<p className="text-xs text-muted-foreground">
					{t(
						"cssIsAutomaticallyScopedToTheCanvasUseClassSelectorsLikeMyclass",
						"CSS is automatically scoped to the canvas. Use class selectors like .my-class",
					)}
				</p>
				<MonacoCodeEditor
					value={canvasSettings.customCss || ""}
					onChange={(value) =>
						setCanvasSettings({ customCss: value || undefined })
					}
					language="css"
					height="150px"
					allowFullscreen
				/>
			</div>
		</div>
	);
}

interface ActionsEditorProps {
	component: SurfaceComponent;
	onUpdate: (updates: Partial<SurfaceComponent>) => void;
}

type ActionType =
	| "widget_event"
	| "navigate_page"
	| "external_link"
	| "workflow_event";

interface ActionValue {
	name: string;
	context?: Record<string, unknown>;
}

type ComponentActionData = {
	actions?: ActionValue[];
	actionBindings?: Record<string, unknown>;
	eventHandlers?: Record<string, ActionValue[]>;
};

function ownsHandler(
	handlers: Record<string, unknown>,
	eventName: string,
): boolean {
	return Object.prototype.hasOwnProperty.call(handlers, eventName);
}

function cloneActions(actions: ActionValue[]): ActionValue[] {
	return actions.map((action) => ({
		...action,
		context: action.context ? { ...action.context } : {},
	}));
}

function createInitialAction(
	widgetActions: readonly { id: string }[] | undefined,
): ActionValue {
	return widgetActions !== undefined
		? {
				name: "widget_event",
				context: widgetActions[0] ? { actionId: widgetActions[0].id } : {},
			}
		: { name: "workflow_event", context: {} };
}

function actionTypeLabel(name: string): string {
	return (
		{
			widget_event: "Widget event",
			navigate_page: i18next.t("navigateToPage", "Navigate to page"),
			external_link: "External link",
			workflow_event: "Trigger workflow",
		}[name] ?? name
	);
}

function HandlerStatus({
	exact,
	actions,
	fallbackLabel,
}: {
	exact: boolean;
	actions: ActionValue[];
	fallbackLabel?: string;
}) {
	const { t } = useTranslation("flow");
	const label = exact
		? actions.length === 0
			? "Disabled"
			: t("countActions", {
					defaultValue_one: "{{count}} action",
					defaultValue_other: "{{count}} actions",
					count: actions.length,
				})
		: (fallbackLabel ?? "Uses default");

	return (
		<span className="shrink-0 rounded border bg-muted/40 px-1.5 py-0.5 text-[10px] font-normal text-muted-foreground">
			{label}
		</span>
	);
}

interface OrderedActionsEditorProps {
	actions: ActionValue[];
	onChange: (actions: ActionValue[]) => void;
}

function OrderedActionsEditor({
	actions,
	onChange,
}: OrderedActionsEditorProps) {
	const { t } = useTranslation("flow");
	const { actionContext } = useBuilder();
	const widgetActions = actionContext?.widgetActions;

	const addAction = () => {
		const nextAction = createInitialAction(widgetActions);
		onChange([...actions, nextAction]);
	};

	const updateAction = (index: number, action: ActionValue | null) => {
		if (action === null) {
			onChange(actions.filter((_, actionIndex) => actionIndex !== index));
			return;
		}
		onChange(
			actions.map((current, actionIndex) =>
				actionIndex === index ? action : current,
			),
		);
	};

	const moveAction = (index: number, direction: -1 | 1) => {
		const target = index + direction;
		if (target < 0 || target >= actions.length) return;
		const next = [...actions];
		[next[index], next[target]] = [next[target], next[index]];
		onChange(next);
	};

	return (
		<div className="space-y-3">
			{actions.length === 0 && (
				<div className="rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
					{t(
						"thisEventIsExplicitlyDisabledAddAnActionToEnableIt",
						"This event is explicitly disabled. Add an action to enable it.",
					)}
				</div>
			)}
			{actions.map((action, index) => (
				<div
					key={`${index}-${action.name}`}
					className="space-y-3 rounded-md border p-2.5 dark:border-white/10"
				>
					<div className="flex items-center gap-1">
						<span className="min-w-0 flex-1 truncate text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
							{index + 1}. {actionTypeLabel(action.name)}
						</span>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-6 w-6"
							disabled={index === 0}
							onClick={() => moveAction(index, -1)}
							aria-label={t("moveActionValUp", "Move action {{val}} up", {
								val: index + 1,
							})}
						>
							<ChevronUp className="h-3.5 w-3.5" />
						</Button>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-6 w-6"
							disabled={index === actions.length - 1}
							onClick={() => moveAction(index, 1)}
							aria-label={t("moveActionValDown", "Move action {{val}} down", {
								val: index + 1,
							})}
						>
							<ChevronDown className="h-3.5 w-3.5" />
						</Button>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-6 w-6 text-destructive hover:text-destructive"
							onClick={() => updateAction(index, null)}
							aria-label={t("removeActionVal", "Remove action {{val}}", {
								val: index + 1,
							})}
						>
							<Trash2 className="h-3.5 w-3.5" />
						</Button>
					</div>
					<ActionValueEditor
						action={action}
						onChange={(nextAction) => updateAction(index, nextAction)}
					/>
				</div>
			))}
			<Button
				type="button"
				variant="outline"
				size="sm"
				className="h-7 w-full text-xs"
				onClick={addAction}
			>
				<Plus className="mr-1 h-3.5 w-3.5" />
				{t("addAction", "Add action")}
			</Button>
		</div>
	);
}

function ActionValueEditor({
	action,
	onChange,
}: {
	action: ActionValue;
	onChange: (action: ActionValue | null) => void;
}) {
	const { t } = useTranslation("flow");
	const { actionContext } = useBuilder();
	const currentType = action.name as ActionType;
	const context = action.context ?? {};
	const widgetActions = actionContext?.widgetActions;
	const isWidgetMode = widgetActions !== undefined;
	const knownActionTypes: string[] = [
		"navigate_page",
		"external_link",
		"workflow_event",
	];

	return (
		<div className="space-y-3">
			<div className="space-y-2">
				<Label className="text-xs font-medium">
					{isWidgetMode ? "Widget Event" : t("actionType", "Action Type")}
				</Label>
				{isWidgetMode ? (
					widgetActions.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							{t(
								"noEventsDefinedAddEventsInWidgetSettingsEventsTab",
								"No events defined. Add events in Widget Settings → Events tab.",
							)}
						</p>
					) : (
						<Select
							value={
								currentType === "widget_event"
									? (context.actionId as string) || "none"
									: "none"
							}
							onValueChange={(value) => {
								if (value === "none") {
									onChange(null);
								} else {
									onChange({
										name: "widget_event",
										context: { actionId: value },
									});
								}
							}}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue placeholder={t("noEvent", "No event")} />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="none">{t("noEvent", "No event")}</SelectItem>
								{widgetActions.map((widgetAction) => (
									<SelectItem key={widgetAction.id} value={widgetAction.id}>
										{widgetAction.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					)
				) : (
					<Select
						value={currentType || "none"}
						onValueChange={(value) => {
							if (value === "none") {
								onChange(null);
							} else {
								onChange({ name: value, context: {} });
							}
						}}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue placeholder={t("noAction", "No action")} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">{t("noAction", "No action")}</SelectItem>
							{currentType && !knownActionTypes.includes(currentType) && (
								<SelectItem value={currentType}>
									{t("customCurrenttype", "Custom: {{currentType}}", {
										currentType,
									})}
								</SelectItem>
							)}
							<SelectItem value="navigate_page">
								{t("navigateToPage2", "Navigate to Page")}
							</SelectItem>
							<SelectItem value="external_link">
								{t("externalLink", "External Link")}
							</SelectItem>
							<SelectItem value="workflow_event">
								{t("triggerWorkflow", "Trigger Workflow")}
							</SelectItem>
						</SelectContent>
					</Select>
				)}
			</div>

			{currentType === "widget_event" && (
				<div className="space-y-1 border-l-2 border-muted pl-2">
					{widgetActions?.find(
						(widgetAction) => widgetAction.id === (context.actionId as string),
					)?.description && (
						<p className="text-xs text-muted-foreground">
							{
								widgetActions.find(
									(widgetAction) =>
										widgetAction.id === (context.actionId as string),
								)?.description
							}
						</p>
					)}
					<p className="text-xs text-muted-foreground">
						{t(
							"thisEventWillBeAvailableForBindingWhenTheWidgetIsInstantiated",
							"This event will be available for binding when the widget is instantiated.",
						)}
					</p>
				</div>
			)}

			{currentType === "navigate_page" && (
				<div className="space-y-2 border-l-2 border-muted pl-2">
					<Label className="text-xs text-muted-foreground">
						{t("route", "Route")}
					</Label>
					<Input
						className="h-8 text-sm"
						placeholder="/about"
						value={(context.route as string) ?? ""}
						onChange={(event) =>
							onChange({
								name: currentType,
								context: { ...context, route: event.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">
						{t(
							"relativePathToNavigateToEgContactProducts123",
							"Relative path to navigate to (e.g., /contact, /products/123)",
						)}
					</p>
					<Label className="mt-2 text-xs text-muted-foreground">
						{t("queryParamsJson", "Query Params (JSON)")}
					</Label>
					<Input
						className="h-8 text-sm font-mono"
						placeholder={`{"id": "123"}`}
						value={(context.queryParams as string) ?? ""}
						onChange={(event) =>
							onChange({
								name: currentType,
								context: { ...context, queryParams: event.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">
						{t(
							"optionalJsonObjectOfQueryParameters",
							"Optional JSON object of query parameters",
						)}
					</p>
				</div>
			)}

			{currentType === "external_link" && (
				<div className="space-y-2 border-l-2 border-muted pl-2">
					<Label className="text-xs text-muted-foreground">URL</Label>
					<Input
						className="h-8 text-sm"
						placeholder="https://example.com"
						value={(context.url as string) ?? ""}
						onChange={(event) =>
							onChange({
								name: currentType,
								context: { ...context, url: event.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">{`Opens in a new tab`}</p>
				</div>
			)}

			{currentType === "workflow_event" && (
				<div className="space-y-2 border-l-2 border-muted pl-2">
					<Label className="text-xs text-muted-foreground">
						{t("workflowEvent", "Workflow Event")}
					</Label>
					<Select
						value={(context.nodeId as string) ?? ""}
						onValueChange={(nodeId) =>
							onChange({
								name: currentType,
								context: {
									...context,
									nodeId,
									appId: actionContext?.appId,
									boardId: actionContext?.boardId,
								},
							})
						}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue placeholder={t("selectEvent", "Select event")} />
						</SelectTrigger>
						<SelectContent>
							{actionContext?.workflowEvents?.length ? (
								actionContext.workflowEvents.map((workflowEvent) => (
									<SelectItem
										key={workflowEvent.nodeId}
										value={workflowEvent.nodeId}
									>
										{workflowEvent.name}
									</SelectItem>
								))
							) : (
								<div className="p-2 text-center text-sm text-muted-foreground">
									{t(
										"noWorkflowEventsAvailable",
										"No workflow events available",
									)}
								</div>
							)}
						</SelectContent>
					</Select>
				</div>
			)}
		</div>
	);
}

function NamedEventHandlerEditor({
	definition,
	exact,
	actions,
	fallbackActions,
	fallbackLabel,
	hasExistingWorkflowBinding,
	onSet,
	onDelete,
}: {
	definition: ComponentEventDefinition;
	exact: boolean;
	actions: ActionValue[];
	fallbackActions: ActionValue[];
	fallbackLabel: string;
	hasExistingWorkflowBinding: boolean;
	onSet: (actions: ActionValue[]) => void;
	onDelete: () => void;
}) {
	const { t } = useTranslation("flow");
	const { actionContext } = useBuilder();
	const customizeActions = () =>
		onSet(
			fallbackActions.length > 0
				? cloneActions(fallbackActions)
				: [createInitialAction(actionContext?.widgetActions)],
		);

	return (
		<Collapsible defaultOpen={exact} className="group rounded-md border">
			<CollapsibleTrigger className="flex w-full items-start gap-2 px-2.5 py-2 text-left hover:bg-muted/40">
				<ChevronDown className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
				<Zap className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
				<div className="min-w-0 flex-1">
					<div className="truncate text-xs font-medium">{definition.label}</div>
					<div className="truncate font-mono text-[10px] text-muted-foreground">
						{definition.id}
					</div>
					{hasExistingWorkflowBinding && (
						<div className="mt-0.5 text-[10px] text-amber-600 dark:text-amber-400">
							{t("existingWorkflowBinding", "Existing workflow binding")}
						</div>
					)}
				</div>
				<HandlerStatus
					exact={exact}
					actions={actions}
					fallbackLabel={fallbackLabel}
				/>
			</CollapsibleTrigger>
			<CollapsibleContent className="space-y-3 border-t px-2.5 py-3">
				<p className="text-xs text-muted-foreground">
					{definition.description}
				</p>
				{hasExistingWorkflowBinding && (
					<div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-2.5 py-2 text-[10px] text-muted-foreground">
						{t(
							"thisLegacyWidgetWorkflowBindingIsPreservedAddingAnExactHandlerMayOverrideItAtRuntime",
							"This legacy widget workflow binding is preserved. Adding an exact handler may override it at runtime.",
						)}
					</div>
				)}
				{exact ? (
					<>
						<OrderedActionsEditor actions={actions} onChange={onSet} />
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-7 w-full text-xs"
							onClick={onDelete}
						>
							{t("useDefault", "Use default")}
						</Button>
					</>
				) : (
					<div className="space-y-2">
						<p className="text-[10px] text-muted-foreground">
							{t(
								"noExactHandlerIsStoredThisEventCurrentlyUsesFallbackLabel",
								"No exact handler is stored. Current fallback: {{fallbackLabel}}.",
								{ fallbackLabel },
							)}
						</p>
						<div className="grid grid-cols-2 gap-2">
							<Button
								type="button"
								variant="outline"
								size="sm"
								className="h-7 text-xs"
								onClick={customizeActions}
							>
								{t("customize", "Customize")}
							</Button>
							<Button
								type="button"
								variant="outline"
								size="sm"
								className="h-7 text-xs"
								onClick={() => onSet([])}
							>
								{t("disable", "Disable")}
							</Button>
						</div>
					</div>
				)}
			</CollapsibleContent>
		</Collapsible>
	);
}

function LegacyDefaultEditor({
	actions,
	onChange,
}: {
	actions: ActionValue[];
	onChange: (action: ActionValue | null) => void;
}) {
	const { t } = useTranslation("flow");
	const { actionContext } = useBuilder();
	const legacyAction = actions[0];
	const dormantCount = Math.max(0, actions.length - 1);
	const widgetActions = actionContext?.widgetActions;

	return (
		<Collapsible
			defaultOpen={Boolean(legacyAction)}
			className="group rounded-md border"
		>
			<CollapsibleTrigger className="flex w-full items-start gap-2 px-2.5 py-2 text-left hover:bg-muted/40">
				<ChevronDown className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
				<div className="min-w-0 flex-1">
					<div className="text-xs font-medium">
						{t("defaultLegacyFallback", "Default / legacy fallback")}
					</div>
					<div className="font-mono text-[10px] text-muted-foreground">
						{t(
							"actions0OnlyTheFirstLegacyActionRuns",
							"actions[0] · only the first legacy action runs",
						)}
					</div>
				</div>
				<span className="shrink-0 rounded border bg-muted/40 px-1.5 py-0.5 text-[10px] font-normal text-muted-foreground">
					{legacyAction ? actionTypeLabel(legacyAction.name) : "Unconfigured"}
				</span>
			</CollapsibleTrigger>
			<CollapsibleContent className="space-y-3 border-t px-2.5 py-3">
				<p className="text-xs text-muted-foreground">
					{t(
						"thisPreservesTheOriginalSingleactionBehaviorExactNamedAndWildcardHandlersTakePrecedence",
						"This preserves the original single-action behavior. Exact named and wildcard handlers take precedence.",
					)}
				</p>
				{dormantCount > 0 && (
					<p className="rounded-md bg-muted/40 px-2.5 py-2 text-[10px] text-muted-foreground">
						{t(
							"dormantcountInactiveLegacy",
							"{{dormantCount}} inactive legacy",
							{ dormantCount },
						)}{" "}
						{t(
							"entriesPreservedWhileThisActionIsEditedRemovingTheDefaultClearsThemToo",
							{
								defaultValue_one:
									"entry preserved while this action is edited. Removing the default clears them too.",
								defaultValue_other:
									"entries preserved while this action is edited. Removing the default clears them too.",
								count: dormantCount,
							},
						)}
					</p>
				)}
				{legacyAction ? (
					<>
						<ActionValueEditor action={legacyAction} onChange={onChange} />
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-7 w-full text-xs text-destructive hover:text-destructive"
							onClick={() => onChange(null)}
						>
							{t("removeLegacyDefault", "Remove legacy default")}
						</Button>
					</>
				) : (
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="h-7 w-full text-xs"
						onClick={() => onChange(createInitialAction(widgetActions))}
					>
						{t("configureLegacyDefault", "Configure legacy default")}
					</Button>
				)}
			</CollapsibleContent>
		</Collapsible>
	);
}

function ActionsEditor({ component, onUpdate }: ActionsEditorProps) {
	const { t } = useTranslation("flow");
	const componentData = component.component as ComponentActionData;
	const legacyActions = componentData.actions ?? [];
	const actionBindings = componentData.actionBindings ?? {};
	const handlers = componentData.eventHandlers ?? {};
	const wildcardExact = ownsHandler(handlers, WILDCARD_EVENT);
	const wildcardActions = wildcardExact ? (handlers[WILDCARD_EVENT] ?? []) : [];
	const legacyAction = legacyActions[0];

	const definitions = useMemo(() => {
		const declared = getComponentEventDefinitions(
			component.component as A2UIComponent,
		);
		const declaredIds = new Set(declared.map((definition) => definition.id));
		const savedEventIds = new Set([
			...Object.keys(handlers),
			...Object.keys(actionBindings),
		]);
		const configuredOnly = [...savedEventIds]
			.filter(
				(eventName) =>
					eventName !== WILDCARD_EVENT && !declaredIds.has(eventName),
			)
			.map(
				(eventName): ComponentEventDefinition => ({
					id: eventName,
					label: eventName,
					description: t(
						"thisHandlerIsConfiguredButIsNotDeclaredByTheCurrentComponentContract",
						"This handler is configured but is not declared by the current component contract.",
					),
					legacyFallback: true,
					wildcardFallback: true,
				}),
			);
		const definitions = [...declared, ...configuredOnly];
		if (definitions.length > 0 || savedEventIds.has(WILDCARD_EVENT)) {
			definitions.push({
				id: WILDCARD_EVENT,
				label: t("wildcardDefault", "Wildcard default"),
				description: t(
					"runsForNamedEventsThatDoNotHaveAnExactHandlerExceptEventsAddedAfterTheComponentShippedThoseNeedTheirOwnHandlerSupportsAnOrderedActionList",
					"Runs for named events that do not have an exact handler, except events added after the component shipped — those need their own handler. Supports an ordered action list.",
				),
				legacyFallback: true,
				wildcardFallback: true,
			});
		}
		return definitions;
	}, [actionBindings, component.component, handlers]);

	const updateHandler = (eventName: string, actions: ActionValue[] | null) => {
		const next = { ...handlers };
		if (actions === null) {
			delete next[eventName];
		} else {
			next[eventName] = actions;
		}

		onUpdate({
			component: {
				...component.component,
				eventHandlers: Object.keys(next).length > 0 ? next : undefined,
			} as SurfaceComponent["component"],
		});
	};

	const updateLegacyAction = (action: ActionValue | null) => {
		onUpdate({
			component: {
				...component.component,
				actions: action ? [action, ...legacyActions.slice(1)] : undefined,
			} as SurfaceComponent["component"],
		});
	};

	const hasAnyConfiguration =
		legacyActions.length > 0 ||
		Object.keys(handlers).length > 0 ||
		Object.keys(actionBindings).length > 0;

	if (definitions.length === 0 && !hasAnyConfiguration) {
		return (
			<div className="rounded-md border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
				{t(
					"thisComponentDoesNotExposeConfigurableEvents",
					"This component does not expose configurable events.",
				)}
			</div>
		);
	}

	return (
		<div className="space-y-4">
			{definitions.length > 0 && (
				<div className="space-y-2">
					<div>
						<Label className="text-xs font-semibold">
							{t("events", "Events")}
						</Label>
						<p className="mt-1 text-[10px] text-muted-foreground">
							{t(
								"eachEventCanRunAnOrderedListOfActionsAnExactEmptyListDisablesOnlyThatEvent",
								"Each event can run an ordered list of actions. An exact empty list disables only that event.",
							)}
						</p>
					</div>
					{definitions.map((definition) => {
						const exact = ownsHandler(handlers, definition.id);
						const actions = exact ? (handlers[definition.id] ?? []) : [];
						const hasExistingWorkflowBinding = ownsHandler(
							actionBindings,
							definition.id,
						);
						const fallbackActions = wildcardExact
							? wildcardActions
							: hasExistingWorkflowBinding
								? []
								: definition.legacyFallback && legacyAction
									? [legacyAction]
									: [];
						const fallbackLabel = wildcardExact
							? t("usesDefault", {
									defaultValue_zero: "Disabled by default",
									defaultValue_other: "Uses default",
									count: wildcardActions.length,
								})
							: hasExistingWorkflowBinding
								? t(
										"usesExistingWorkflowBinding",
										"Uses existing workflow binding",
									)
								: definition.legacyFallback && legacyAction
									? t("usesLegacyFallback", "Uses legacy fallback")
									: t("unconfigured", "Unconfigured");

						return (
							<NamedEventHandlerEditor
								key={definition.id}
								definition={definition}
								exact={exact}
								actions={actions}
								fallbackActions={fallbackActions}
								fallbackLabel={fallbackLabel}
								hasExistingWorkflowBinding={hasExistingWorkflowBinding}
								onSet={(nextActions) =>
									updateHandler(definition.id, nextActions)
								}
								onDelete={() => updateHandler(definition.id, null)}
							/>
						);
					})}
				</div>
			)}

			<div className="space-y-2 border-t pt-4">
				<LegacyDefaultEditor
					actions={legacyActions}
					onChange={updateLegacyAction}
				/>
			</div>
		</div>
	);
}
