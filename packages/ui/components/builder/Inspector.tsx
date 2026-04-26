"use client";

import { ChevronDown, Plus, Trash2 } from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { cn } from "../../lib";
import {
	NIVO_CHART_DEFAULTS,
	NIVO_SAMPLE_DATA,
} from "../a2ui/display/nivo-data";
import { getModel3DView } from "../a2ui/game/model3d-view-registry";
import type {
	BoundValue,
	ChartAxis,
	ChartSeries,
	ChartType,
	Overflow,
	Position,
	SelectOption,
	Style,
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
import { AssetPicker } from "./AssetPicker";
import { useBuilder } from "./BuilderContext";
import { getDefaultProps } from "./componentDefaults";
import { getComponentSchema } from "./componentSchema";

// Component types that have asset properties
const ASSET_COMPONENT_TYPES = new Set(["image", "sprite", "model3d", "video"]);
// Property names that should use the asset picker
const ASSET_PROPERTY_NAMES = new Set(["src", "poster", "fallback"]);

export interface InspectorProps {
	className?: string;
}

// Helper to get component type from SurfaceComponent
function getComponentType(sc: SurfaceComponent): string {
	return sc.component?.type ?? "Unknown";
}

export function Inspector({ className }: InspectorProps) {
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
					<h3 className="font-medium text-sm">Inspector</h3>
				</div>
				<div className="flex-1 flex items-center justify-center p-4 text-sm text-muted-foreground">
					Select a component to edit
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
					<h3 className="font-medium text-sm">Inspector</h3>
					<p className="text-xs text-muted-foreground mt-1">
						{selectedComponents.length} components selected
					</p>
				</div>
				<div className="p-4">
					<p className="text-sm text-muted-foreground">
						Multi-selection editing coming soon
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
					ID: {singleSelected?.id}
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
						Props
					</TabsTrigger>
					<TabsTrigger
						value="style"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						Style
					</TabsTrigger>
					<TabsTrigger
						value="canvas"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						Canvas
					</TabsTrigger>
					<TabsTrigger
						value="actions"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						Actions
					</TabsTrigger>
				</TabsList>

				<ScrollArea className="flex-1 min-h-0">
					<TabsContent value="properties" className="m-0 p-4">
						{singleSelected && (
							<PropertyEditor
								component={singleSelected}
								onUpdate={(updates) =>
									updateComponent(singleSelected.id, updates)
								}
							/>
						)}
					</TabsContent>

					<TabsContent value="style" className="m-0 p-4">
						{singleSelected && (
							<StyleEditor
								component={singleSelected}
								onUpdate={(updates) =>
									updateComponent(singleSelected.id, updates)
								}
							/>
						)}
					</TabsContent>

					<TabsContent value="canvas" className="m-0 p-4">
						<CanvasSettingsEditor />
					</TabsContent>

					<TabsContent value="actions" className="m-0 p-4">
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

	// Check if this is an asset component type
	const isAssetComponent = ASSET_COMPONENT_TYPES.has(componentType ?? "");

	// Render different editors based on component type
	return (
		<div className="space-y-4">
			{/* Common ID field */}
			<div className="space-y-2">
				<Label className="text-xs">Component ID</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* Type-specific properties */}
			{Object.entries(props).map(([key, value]) => (
				<PropertyField
					key={key}
					name={key}
					value={value}
					onChange={(newValue) => updateProp(key, newValue)}
					isAssetProperty={isAssetComponent && ASSET_PROPERTY_NAMES.has(key)}
					componentType={componentType}
					enumOptions={schema?.[key]?.enum}
				/>
			))}
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
				<Label className="text-xs text-muted-foreground">Component ID</Label>
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
							<Label className="text-xs text-muted-foreground">Scale</Label>
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
										Uniform
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
						{renderToggle("autoRotate", false, "Auto Rotate Model")}
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
							"Angle Preset",
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
						"Field of View",
					)}
					<div className="flex items-center justify-between py-1.5 border-y">
						<span className="text-xs text-muted-foreground">
							Capture current view
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
							Use View
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
						{renderToggle("autoRotateCamera", false, "Auto Orbit")}
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
							"ambientLight",
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
						label="Light Color"
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
									label="HDRI URL"
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
						label="Ground Color"
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
						label="Background"
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
	const series = (props.series as ChartSeries[] | undefined) ?? [];
	const xAxis = (props.xAxis as ChartAxis | undefined) ?? {};
	const yAxis = (props.yAxis as ChartAxis | undefined) ?? {};

	const addSeries = useCallback(() => {
		const newSeries: ChartSeries = {
			name: `Series ${series.length + 1}`,
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
				<Label className="text-xs">Component ID</Label>
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
					<span>Data Series ({series.length})</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-3 pt-2">
					{series.map((s, idx) => (
						<div key={idx} className="space-y-2 rounded border p-2">
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium">Series {idx + 1}</span>
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
								placeholder="Series name"
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
									<Label className="text-xs">Color</Label>
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
								<Label className="text-xs">Data (CSV: label,value)</Label>
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
									<Label className="text-xs">Mode</Label>
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
											<SelectItem value="lines">Lines</SelectItem>
											<SelectItem value="markers">Markers</SelectItem>
											<SelectItem value="lines+markers">
												Lines + Markers
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
						Add Series
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* X Axis */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>X Axis</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">Title</Label>
						<Input
							value={xAxis.title || ""}
							onChange={(e) => updateXAxis({ title: e.target.value })}
							placeholder="X Axis Title"
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
								<SelectItem value="category">Category</SelectItem>
								<SelectItem value="linear">Linear</SelectItem>
								<SelectItem value="log">Log</SelectItem>
								<SelectItem value="date">Date</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">Show Grid</Label>
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
					<span>Y Axis</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">Title</Label>
						<Input
							value={yAxis.title || ""}
							onChange={(e) => updateYAxis({ title: e.target.value })}
							placeholder="Y Axis Title"
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
								<SelectItem value="linear">Linear</SelectItem>
								<SelectItem value="log">Log</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">Min</Label>
							<Input
								type="number"
								value={xAxis.min ?? ""}
								onChange={(e) =>
									updateYAxis({
										min: e.target.value ? Number(e.target.value) : undefined,
									})
								}
								placeholder="Auto"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Max</Label>
							<Input
								type="number"
								value={yAxis.max ?? ""}
								onChange={(e) =>
									updateYAxis({
										max: e.target.value ? Number(e.target.value) : undefined,
									})
								}
								placeholder="Auto"
								className="h-7 text-xs"
							/>
						</div>
					</div>
					<div className="flex items-center justify-between">
						<Label className="text-xs">Show Grid</Label>
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
					<span>Display</span>
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
						<Label className="text-xs">Show Legend</Label>
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
						<Label className="text-xs">Legend Position</Label>
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
								<SelectItem value="top">Top</SelectItem>
								<SelectItem value="bottom">Bottom</SelectItem>
								<SelectItem value="left">Left</SelectItem>
								<SelectItem value="right">Right</SelectItem>
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
				<Label className="text-xs">Component ID</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* Chart Type */}
			<div className="space-y-2">
				<Label className="text-xs">Chart Type</Label>
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
					<span>Data</span>
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
							JSON
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
								placeholder="Enter JSON data..."
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
								Reset to Default Data
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
								Apply CSV Data
							</Button>
						</div>
					)}
				</CollapsibleContent>
			</Collapsible>

			{/* Keys & Index (for bar, radar, etc.) */}
			{needsKeysAndIndex && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>Data Keys</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">Index By (Category Field)</Label>
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
								placeholder="e.g. country, category"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">
								Keys (Data Series - comma separated)
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
								placeholder="e.g. sales, revenue, profit"
								className="h-7 text-xs"
							/>
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}

			{/* Styling */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Styling</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<div className="space-y-1">
						<Label className="text-xs">Color Scheme</Label>
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
						<Label className="text-xs">Animate</Label>
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
						<Label className="text-xs">Show Legend</Label>
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
						<Label className="text-xs">Legend Position</Label>
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
								<SelectItem value="top">Top</SelectItem>
								<SelectItem value="bottom">Bottom</SelectItem>
								<SelectItem value="left">Left</SelectItem>
								<SelectItem value="right">Right</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Chart-specific styling */}
			{chartType === "bar" && (
				<Collapsible>
					<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
						<span>Bar Options</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">Layout</Label>
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
									<SelectItem value="vertical">Vertical</SelectItem>
									<SelectItem value="horizontal">Horizontal</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Group Mode</Label>
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
									<SelectItem value="grouped">Grouped</SelectItem>
									<SelectItem value="stacked">Stacked</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Border Radius</Label>
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
						<span>Pie Options</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">
								Inner Radius (0 = pie, &gt;0 = donut)
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
							<Label className="text-xs">Pad Angle</Label>
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
							<Label className="text-xs">Corner Radius</Label>
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
						<span>Line Options</span>
						<ChevronDown className="h-4 w-4" />
					</CollapsibleTrigger>
					<CollapsibleContent className="space-y-2 pt-2">
						<div className="space-y-1">
							<Label className="text-xs">Curve</Label>
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
									<SelectItem value="linear">Linear</SelectItem>
									<SelectItem value="monotoneX">Smooth</SelectItem>
									<SelectItem value="natural">Natural</SelectItem>
									<SelectItem value="step">Step</SelectItem>
									<SelectItem value="stepBefore">Step Before</SelectItem>
									<SelectItem value="stepAfter">Step After</SelectItem>
									<SelectItem value="basis">Basis</SelectItem>
									<SelectItem value="cardinal">Cardinal</SelectItem>
								</SelectContent>
							</Select>
						</div>
						<div className="flex items-center justify-between">
							<Label className="text-xs">Enable Area</Label>
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
							<Label className="text-xs">Show Points</Label>
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
							<Label className="text-xs">Line Width</Label>
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
					<span>Display</span>
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
				`Failed to parse CSV: ${err instanceof Error ? err.message : "Unknown error"}`,
			);
		}
	}, [csvInput, updateProp]);

	// Add a new column
	const addColumn = useCallback(() => {
		const newColumn: TableColumn = {
			id: `col-${Date.now()}`,
			header: { literalString: `Column ${columns.length + 1}` },
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
				<Label className="text-xs">Component ID</Label>
				<Input
					value={component.id}
					onChange={(e) => onUpdate({ id: e.target.value })}
					className="h-8 text-sm"
				/>
			</div>

			{/* CSV Import */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Import from CSV</span>
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
						Import CSV
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Columns */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Columns ({columns.length})</span>
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
								<Label className="text-xs">Header</Label>
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
								<Label className="text-xs">Accessor (data key)</Label>
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
								<Label className="text-xs">Sortable</Label>
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
						Add Column
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Data Rows */}
			<Collapsible defaultOpen={data.length <= 5}>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Data ({data.length} rows)</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					{data.length === 0 && columns.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							Add columns first or import from CSV
						</p>
					) : data.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							No data rows. Add a row below or import from CSV.
						</p>
					) : (
						<div className="space-y-2 max-h-[300px] overflow-y-auto">
							{data.map((row: Record<string, string>, rowIdx: number) => (
								<div key={rowIdx} className="rounded border p-2 space-y-1">
									<div className="flex items-center justify-between mb-1">
										<span className="text-xs font-medium">
											Row {rowIdx + 1}
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
						Add Row
					</Button>
				</CollapsibleContent>
			</Collapsible>

			{/* Table Options */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Options</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="space-y-2 pt-2">
					<BoundValueEditor
						name="caption"
						value={(props.caption as BoundValue) ?? { literalString: "" }}
						onChange={(v) => updateProp("caption", v)}
					/>
					<div className="flex items-center justify-between">
						<Label className="text-xs">Striped</Label>
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
						<Label className="text-xs">Bordered</Label>
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
						<Label className="text-xs">Hoverable</Label>
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
						<Label className="text-xs">Compact</Label>
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
						<Label className="text-xs">Sticky Header</Label>
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
						<Label className="text-xs">Sortable</Label>
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
						<Label className="text-xs">Searchable</Label>
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
						<Label className="text-xs">Paginated</Label>
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
							<Label className="text-xs">Page Size</Label>
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
	componentType?: string;
	enumOptions?: string[];
}

function PropertyField({
	name,
	value,
	onChange,
	isAssetProperty,
	componentType,
	enumOptions,
}: PropertyFieldProps) {
	const { actionContext } = useBuilder();
	const appId = actionContext?.appId;

	// Determine asset accept type based on component type
	const getAssetAccept = (): "image" | "model" | "video" | "all" => {
		if (componentType === "image" || componentType === "sprite") return "image";
		if (componentType === "model3d") return "model";
		if (componentType === "video") return "video";
		return "all";
	};

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
					assetAccept={getAssetAccept()}
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
				<div className="space-y-2">
					<Label className="text-xs capitalize">
						{name.replace(/([A-Z])/g, " $1")}
					</Label>
					<AssetPicker
						appId={appId}
						value={value}
						onChange={(newValue) => onChange(newValue)}
						accept={getAssetAccept()}
						placeholder={`Select ${name}...`}
					/>
				</div>
			);
		}
		return (
			<div className="space-y-2">
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
			<div className="space-y-2">
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
	const addOption = useCallback(() => {
		onChange([
			...options,
			{
				value: `option${options.length + 1}`,
				label: `Option ${options.length + 1}`,
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
		<div className="space-y-2">
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
						<SelectItem value="literal">Literal</SelectItem>
						<SelectItem value="binding">Binding</SelectItem>
					</SelectContent>
				</Select>
			</div>
			<div className="space-y-1.5 rounded border p-2">
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
					Add Option
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
	assetAccept?: "image" | "model" | "video" | "all";
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
			<div className="space-y-2">
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
							<SelectItem value="literal">Literal</SelectItem>
							<SelectItem value="binding">Binding</SelectItem>
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
		<div className="space-y-2">
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
						<SelectItem value="literal">Literal</SelectItem>
						<SelectItem value="binding">Binding</SelectItem>
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
						<SelectItem value="true">True</SelectItem>
						<SelectItem value="false">False</SelectItem>
					</SelectContent>
				</Select>
			) : mode === "literal" && originalType === "json" ? (
				<Textarea
					value={String(currentValue)}
					onChange={(e) => handleLiteralChange(e.target.value)}
					placeholder="[0, 0, 0]"
					className="min-h-20 text-xs"
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
					placeholder={mode === "binding" ? "/path/to/data" : "Enter value..."}
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
	const style = component.style || {};

	const updateStyle = useCallback(
		<K extends keyof Style>(key: K, value: Style[K]) => {
			onUpdate({
				style: {
					...style,
					[key]: value,
				},
			});
		},
		[style, onUpdate],
	);

	return (
		<div className="space-y-4">
			{/* Spacing - Margin & Padding */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Spacing</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">Margin</Label>
						<div className="grid grid-cols-4 gap-1">
							<Input
								value={style.margin?.top || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("margin", {
										...style.margin,
										top: e.target.value,
									})
								}
								placeholder="T"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.margin?.right || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("margin", {
										...style.margin,
										right: e.target.value,
									})
								}
								placeholder="R"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.margin?.bottom || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("margin", {
										...style.margin,
										bottom: e.target.value,
									})
								}
								placeholder="B"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.margin?.left || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("margin", {
										...style.margin,
										left: e.target.value,
									})
								}
								placeholder="L"
								className="h-7 text-xs text-center"
							/>
						</div>
						<p className="text-[10px] text-muted-foreground">
							Top, Right, Bottom, Left (e.g., 8px, 1rem, auto)
						</p>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Padding</Label>
						<div className="grid grid-cols-4 gap-1">
							<Input
								value={style.padding?.top || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("padding", {
										...style.padding,
										top: e.target.value,
									})
								}
								placeholder="T"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.padding?.right || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("padding", {
										...style.padding,
										right: e.target.value,
									})
								}
								placeholder="R"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.padding?.bottom || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("padding", {
										...style.padding,
										bottom: e.target.value,
									})
								}
								placeholder="B"
								className="h-7 text-xs text-center"
							/>
							<Input
								value={style.padding?.left || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("padding", {
										...style.padding,
										left: e.target.value,
									})
								}
								placeholder="L"
								className="h-7 text-xs text-center"
							/>
						</div>
						<p className="text-[10px] text-muted-foreground">
							Top, Right, Bottom, Left (e.g., 8px, 1rem)
						</p>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Sizing */}
			<Collapsible defaultOpen>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Size</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">Width</Label>
							<Input
								value={style.width || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("width", e.target.value || undefined)
								}
								placeholder="auto"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Height</Label>
							<Input
								value={style.height || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("height", e.target.value || undefined)
								}
								placeholder="auto"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Min Width</Label>
							<Input
								value={style.minWidth || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("minWidth", e.target.value || undefined)
								}
								placeholder="0"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Min Height</Label>
							<Input
								value={style.minHeight || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("minHeight", e.target.value || undefined)
								}
								placeholder="0"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Max Width</Label>
							<Input
								value={style.maxWidth || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("maxWidth", e.target.value || undefined)
								}
								placeholder="none"
								className="h-7 text-xs"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Max Height</Label>
							<Input
								value={style.maxHeight || ""}
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
				<Label className="text-xs">Tailwind Classes</Label>
				<Textarea
					value={style.className || ""}
					onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) =>
						updateStyle("className", e.target.value)
					}
					placeholder="Enter Tailwind classes"
					autoComplete="off"
					autoCorrect="off"
					autoCapitalize="off"
					className="text-sm min-h-[60px]"
				/>
				<p className="text-xs text-muted-foreground">
					Additional utility classes
				</p>
			</div>

			{/* Position */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Position</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">Type</Label>
						<Select
							value={style.position?.type || "relative"}
							onValueChange={(v) =>
								updateStyle("position", {
									...style.position,
									type: v as Position["type"],
								})
							}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue placeholder="Select position" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="relative">Relative</SelectItem>
								<SelectItem value="absolute">Absolute</SelectItem>
								<SelectItem value="fixed">Fixed</SelectItem>
								<SelectItem value="sticky">Sticky</SelectItem>
							</SelectContent>
						</Select>
					</div>
					{style.position?.type && style.position.type !== "relative" && (
						<div className="grid grid-cols-2 gap-2">
							<div className="space-y-1">
								<Label className="text-xs">Top</Label>
								<Input
									value={style.position?.top || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...style.position!,
											top: e.target.value,
										})
									}
									placeholder="0"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">Right</Label>
								<Input
									value={style.position?.right || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...style.position!,
											right: e.target.value,
										})
									}
									placeholder="auto"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">Bottom</Label>
								<Input
									value={style.position?.bottom || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...style.position!,
											bottom: e.target.value,
										})
									}
									placeholder="auto"
									className="h-8 text-sm"
								/>
							</div>
							<div className="space-y-1">
								<Label className="text-xs">Left</Label>
								<Input
									value={style.position?.left || ""}
									onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
										updateStyle("position", {
											...style.position!,
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
					<span>Background</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="space-y-1">
						<Label className="text-xs">Color</Label>
						<div className="flex gap-2">
							<Input
								type="color"
								value={
									style.background && "color" in style.background
										? style.background.color
										: "#ffffff"
								}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("background", { color: e.target.value })
								}
								className="h-8 w-12 p-1"
							/>
							<Input
								value={
									style.background && "color" in style.background
										? style.background.color
										: ""
								}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("background", { color: e.target.value })
								}
								placeholder="#ffffff or transparent"
								className="h-8 text-sm flex-1"
							/>
						</div>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Image URL</Label>
						<Input
							value={
								style.background &&
								"image" in style.background &&
								style.background.image?.url
									? "literalString" in style.background.image.url
										? style.background.image.url.literalString
										: ""
									: ""
							}
							onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
								updateStyle("background", {
									image: {
										url: { literalString: e.target.value },
										size: "cover",
										position: "center",
										repeat: "no-repeat",
									},
								})
							}
							placeholder="/path/to/image.jpg"
							className="h-8 text-sm"
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Backdrop Blur</Label>
						<Input
							value={
								style.background && "blur" in style.background
									? style.background.blur
									: ""
							}
							onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
								updateStyle("background", { blur: e.target.value })
							}
							placeholder="4px"
							className="h-8 text-sm"
						/>
					</div>
					<div className="space-y-1">
						<Label className="text-xs">Opacity</Label>
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
					<span>Border</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">Width</Label>
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
							<Label className="text-xs">Radius</Label>
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
						<Label className="text-xs">Color</Label>
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
								placeholder="#e5e7eb"
								className="h-8 text-sm flex-1"
							/>
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Shadow */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Shadow</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">X Offset</Label>
							<Input
								value={style.shadow?.x || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("shadow", { ...style.shadow, x: e.target.value })
								}
								placeholder="0"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Y Offset</Label>
							<Input
								value={style.shadow?.y || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("shadow", { ...style.shadow, y: e.target.value })
								}
								placeholder="2px"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Blur</Label>
							<Input
								value={style.shadow?.blur || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("shadow", {
										...style.shadow,
										blur: e.target.value,
									})
								}
								placeholder="4px"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Color</Label>
							<Input
								value={style.shadow?.color || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("shadow", {
										...style.shadow,
										color: e.target.value,
									})
								}
								placeholder="rgba(0,0,0,0.1)"
								className="h-8 text-sm"
							/>
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Transform */}
			<Collapsible>
				<CollapsibleTrigger className="flex w-full items-center justify-between py-2 text-sm font-medium">
					<span>Transform</span>
					<ChevronDown className="h-4 w-4" />
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2 space-y-3">
					<div className="grid grid-cols-2 gap-2">
						<div className="space-y-1">
							<Label className="text-xs">Translate</Label>
							<Input
								value={style.transform?.translate || ""}
								onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
									updateStyle("transform", {
										...style.transform,
										translate: e.target.value,
									})
								}
								placeholder="0, 0"
								className="h-8 text-sm"
							/>
						</div>
						<div className="space-y-1">
							<Label className="text-xs">Rotate (deg)</Label>
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
							<Label className="text-xs">Scale</Label>
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
					</div>
				</CollapsibleContent>
			</Collapsible>

			{/* Overflow */}
			<div className="space-y-2">
				<Label className="text-xs">Overflow</Label>
				<Select
					value={style.overflow || "visible"}
					onValueChange={(v) => updateStyle("overflow", v as Overflow)}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue placeholder="Select overflow" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="visible">Visible</SelectItem>
						<SelectItem value="hidden">Hidden</SelectItem>
						<SelectItem value="scroll">Scroll</SelectItem>
						<SelectItem value="auto">Auto</SelectItem>
					</SelectContent>
				</Select>
			</div>
		</div>
	);
}

// Canvas settings editor - global canvas settings
function CanvasSettingsEditor() {
	const { canvasSettings, setCanvasSettings } = useBuilder();

	return (
		<div className="space-y-4">
			<p className="text-xs text-muted-foreground mb-4">
				These settings apply to the entire canvas background, not individual
				components.
			</p>

			{/* Canvas Background Color */}
			<div className="space-y-2">
				<Label className="text-xs">Canvas Background</Label>
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
				<Label className="text-xs">Background Image URL</Label>
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
				<Label className="text-xs">Canvas Padding</Label>
				<Select
					value={canvasSettings.padding || "16px"}
					onValueChange={(v) => setCanvasSettings({ padding: v })}
				>
					<SelectTrigger className="h-8 text-sm">
						<SelectValue placeholder="Select padding" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="0">None</SelectItem>
						<SelectItem value="8px">Small (8px)</SelectItem>
						<SelectItem value="16px">Medium (16px)</SelectItem>
						<SelectItem value="24px">Large (24px)</SelectItem>
						<SelectItem value="32px">Extra Large (32px)</SelectItem>
					</SelectContent>
				</Select>
			</div>

			{/* Custom CSS */}
			<div className="space-y-2">
				<Label className="text-xs">Custom CSS</Label>
				<p className="text-xs text-muted-foreground">
					CSS is automatically scoped to the canvas. Use class selectors like
					.my-class
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

function ActionsEditor({ component, onUpdate }: ActionsEditorProps) {
	const { actionContext } = useBuilder();
	const componentData = component.component as { actions?: ActionValue[] };
	const actions = componentData.actions ?? [];
	const action = actions[0] ?? null;

	const setAction = (newAction: ActionValue | null) => {
		onUpdate({
			component: {
				...component.component,
				actions: newAction ? [newAction] : undefined,
			} as SurfaceComponent["component"],
		});
	};

	const currentType = action?.name as ActionType | undefined;
	const context = action?.context ?? {};
	const widgetActions = actionContext?.widgetActions;
	const isWidgetMode = widgetActions !== undefined;

	return (
		<div className="space-y-4">
			<div className="space-y-2">
				<Label className="text-xs font-medium">
					{isWidgetMode ? "Widget Event" : "Action Type"}
				</Label>
				{isWidgetMode ? (
					widgetActions.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							No events defined. Add events in Widget Settings → Events tab.
						</p>
					) : (
						<Select
							value={
								currentType === "widget_event"
									? ((context.actionId as string) ?? "")
									: "none"
							}
							onValueChange={(v) => {
								if (v === "none") {
									setAction(null);
								} else {
									setAction({ name: "widget_event", context: { actionId: v } });
								}
							}}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue placeholder="No event" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="none">No event</SelectItem>
								{widgetActions.map((wa) => (
									<SelectItem key={wa.id} value={wa.id}>
										{wa.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					)
				) : (
					<Select
						value={currentType ?? "none"}
						onValueChange={(v) => {
							if (v === "none") {
								setAction(null);
							} else {
								setAction({ name: v, context: {} });
							}
						}}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue placeholder="No action" />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">No action</SelectItem>
							<SelectItem value="navigate_page">Navigate to Page</SelectItem>
							<SelectItem value="external_link">External Link</SelectItem>
							<SelectItem value="workflow_event">Trigger Workflow</SelectItem>
						</SelectContent>
					</Select>
				)}
			</div>

			{currentType === "widget_event" && (
				<div className="space-y-1 pl-2 border-l-2 border-muted">
					{widgetActions?.find((wa) => wa.id === (context.actionId as string))
						?.description && (
						<p className="text-xs text-muted-foreground">
							{
								widgetActions.find(
									(wa) => wa.id === (context.actionId as string),
								)?.description
							}
						</p>
					)}
					<p className="text-xs text-muted-foreground">
						This event will be available for binding when the widget is
						instantiated.
					</p>
				</div>
			)}

			{currentType === "navigate_page" && (
				<div className="space-y-2 pl-2 border-l-2 border-muted">
					<Label className="text-xs text-muted-foreground">Route</Label>
					<Input
						className="h-8 text-sm"
						placeholder="/about"
						value={(context.route as string) ?? ""}
						onChange={(e) =>
							setAction({
								name: currentType,
								context: { ...context, route: e.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">
						Relative path to navigate to (e.g., /contact, /products/123)
					</p>
					<Label className="text-xs text-muted-foreground mt-2">
						Query Params (JSON)
					</Label>
					<Input
						className="h-8 text-sm font-mono"
						placeholder='{"id": "123"}'
						value={(context.queryParams as string) ?? ""}
						onChange={(e) =>
							setAction({
								name: currentType,
								context: { ...context, queryParams: e.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">
						Optional JSON object of query parameters
					</p>
				</div>
			)}

			{currentType === "external_link" && (
				<div className="space-y-2 pl-2 border-l-2 border-muted">
					<Label className="text-xs text-muted-foreground">URL</Label>
					<Input
						className="h-8 text-sm"
						placeholder="https://example.com"
						value={(context.url as string) ?? ""}
						onChange={(e) =>
							setAction({
								name: currentType,
								context: { url: e.target.value },
							})
						}
					/>
					<p className="text-xs text-muted-foreground">Opens in a new tab</p>
				</div>
			)}

			{currentType === "workflow_event" && (
				<div className="space-y-2 pl-2 border-l-2 border-muted">
					<Label className="text-xs text-muted-foreground">
						Workflow Event
					</Label>
					<Select
						value={(context.nodeId as string) ?? ""}
						onValueChange={(nodeId) =>
							setAction({
								name: currentType,
								context: {
									nodeId,
									appId: actionContext?.appId,
									boardId: actionContext?.boardId,
									boardVersion: actionContext?.boardVersion,
								},
							})
						}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue placeholder="Select event" />
						</SelectTrigger>
						<SelectContent>
							{actionContext?.workflowEvents?.length ? (
								actionContext.workflowEvents.map((event) => (
									<SelectItem key={event.nodeId} value={event.nodeId}>
										{event.name}
									</SelectItem>
								))
							) : (
								<div className="p-2 text-sm text-muted-foreground text-center">
									No workflow events available
								</div>
							)}
						</SelectContent>
					</Select>
					{actionContext?.boardVersion && (
						<p className="text-xs text-muted-foreground">
							Uses board version v{actionContext.boardVersion.join(".")}
						</p>
					)}
				</div>
			)}
		</div>
	);
}
