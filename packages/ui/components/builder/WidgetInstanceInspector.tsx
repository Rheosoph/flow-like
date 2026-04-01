"use client";

import { ChevronRight, Info, Link, Play, Unlink } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { cn } from "../../lib";
import type {
	ActionBinding,
	BoundValue,
	Widget,
	WidgetAction,
	WidgetInstance,
	WidgetRef,
} from "../a2ui/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

export interface WorkflowEventOption {
	nodeId: string;
	name: string;
	flowId: string;
}

export interface WidgetInstanceInspectorProps {
	instance: WidgetInstance;
	widget: Widget;
	availableWorkflows?: WorkflowEventOption[];
	className?: string;
	onInstanceChange: (instance: WidgetInstance) => void;
}

export function WidgetInstanceInspector({
	instance,
	widget,
	availableWorkflows = [],
	className,
	onInstanceChange,
}: WidgetInstanceInspectorProps) {
	const updateCustomization = useCallback(
		(key: string, value: unknown) => {
			onInstanceChange({
				...instance,
				customizationValues: {
					...instance.customizationValues,
					[key]: value,
				},
			});
		},
		[instance, onInstanceChange],
	);

	const updateActionBinding = useCallback(
		(actionId: string, binding: ActionBinding | null) => {
			const newBindings = { ...instance.actionBindings };
			if (binding === null) {
				delete newBindings[actionId];
			} else {
				newBindings[actionId] = binding;
			}
			onInstanceChange({
				...instance,
				actionBindings: newBindings,
			});
		},
		[instance, onInstanceChange],
	);

	return (
		<div
			className={cn(
				"flex flex-col h-full bg-background border-l overflow-hidden",
				className,
			)}
		>
			<div className="p-4 border-b shrink-0">
				<div className="flex items-center gap-2">
					{widget.thumbnail && (
						<img
							src={widget.thumbnail}
							alt=""
							className="h-8 w-8 rounded object-cover"
						/>
					)}
					<div className="min-w-0">
						<h3 className="font-medium text-sm truncate">{widget.name}</h3>
						<p className="text-xs text-muted-foreground truncate">
							{instance.instanceId}
						</p>
					</div>
				</div>
				{instance.widgetRef && (
					<WidgetRefBadge widgetRef={instance.widgetRef} />
				)}
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
						value="actions"
						className="rounded-none border-b-2 border-transparent data-[state=active]:border-primary px-3 text-xs"
					>
						Actions
						{widget.actions.length > 0 && (
							<Badge variant="secondary" className="ml-1 h-4 text-[10px]">
								{widget.actions.length}
							</Badge>
						)}
					</TabsTrigger>
				</TabsList>

				<ScrollArea className="flex-1 min-h-0">
					<TabsContent value="properties" className="m-0 p-4">
						<CustomizationEditor
							options={widget.customizationOptions}
							values={instance.customizationValues}
							onChange={updateCustomization}
						/>
					</TabsContent>

					<TabsContent value="actions" className="m-0 p-4">
						<ActionBindingsEditor
							actions={widget.actions}
							bindings={instance.actionBindings}
							availableWorkflows={availableWorkflows}
							onBindingChange={updateActionBinding}
						/>
					</TabsContent>
				</ScrollArea>
			</Tabs>
		</div>
	);
}

interface WidgetRefBadgeProps {
	widgetRef: WidgetRef;
}

function WidgetRefBadge({ widgetRef }: WidgetRefBadgeProps) {
	return (
		<div className="flex items-center gap-1 mt-2 text-xs text-muted-foreground">
			<Link className="h-3 w-3" />
			<span className="truncate">
				{widgetRef.appId}/{widgetRef.widgetId}
				{widgetRef.version && `@${widgetRef.version}`}
			</span>
		</div>
	);
}

interface CustomizationEditorProps {
	options: Widget["customizationOptions"];
	values: Record<string, unknown>;
	onChange: (key: string, value: unknown) => void;
}

function CustomizationEditor({
	options,
	values,
	onChange,
}: CustomizationEditorProps) {
	const groups = useMemo(() => {
		const grouped: Record<string, typeof options> = { default: [] };
		for (const opt of options) {
			const group = opt.group ?? "default";
			grouped[group] = grouped[group] ?? [];
			grouped[group].push(opt);
		}
		return grouped;
	}, [options]);

	const groupNames = Object.keys(groups).sort((a, b) =>
		a === "default" ? -1 : b === "default" ? 1 : a.localeCompare(b),
	);

	if (options.length === 0) {
		return (
			<div className="text-sm text-muted-foreground text-center py-4">
				No customization options
			</div>
		);
	}

	return (
		<div className="space-y-4">
			{groupNames.map((groupName) => {
				const groupOptions = groups[groupName];
				if (groupOptions.length === 0) return null;

				return (
					<Collapsible key={groupName} defaultOpen>
						<CollapsibleTrigger className="flex w-full items-center justify-between text-xs font-medium text-muted-foreground hover:text-foreground">
							<span className="capitalize">
								{groupName === "default" ? "General" : groupName}
							</span>
							<ChevronRight className="h-3 w-3 transition-transform duration-200 data-[state=open]:rotate-90" />
						</CollapsibleTrigger>
						<CollapsibleContent className="pt-2 space-y-3">
							{groupOptions.map((opt) => (
								<CustomizationField
									key={opt.id}
									option={opt}
									value={values[opt.id] ?? opt.defaultValue}
									onChange={(v) => onChange(opt.id, v)}
								/>
							))}
						</CollapsibleContent>
					</Collapsible>
				);
			})}
		</div>
	);
}

interface CustomizationFieldProps {
	option: Widget["customizationOptions"][number];
	value: unknown;
	onChange: (value: unknown) => void;
}

function CustomizationField({
	option,
	value,
	onChange,
}: CustomizationFieldProps) {
	const renderField = () => {
		switch (option.type) {
			case "string":
				return (
					<Input
						className="h-8 text-sm"
						value={(value as string) ?? ""}
						onChange={(e) => onChange(e.target.value)}
					/>
				);
			case "number":
				return (
					<Input
						type="number"
						className="h-8 text-sm"
						value={(value as number) ?? 0}
						onChange={(e) => onChange(Number.parseFloat(e.target.value) || 0)}
					/>
				);
			case "boolean":
				return (
					<Select
						value={String(value ?? false)}
						onValueChange={(v) => onChange(v === "true")}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="true">True</SelectItem>
							<SelectItem value="false">False</SelectItem>
						</SelectContent>
					</Select>
				);
			case "color":
				return (
					<div className="flex gap-2">
						<Input
							type="color"
							className="h-8 w-12 p-1 cursor-pointer"
							value={(value as string) ?? "#000000"}
							onChange={(e) => onChange(e.target.value)}
						/>
						<Input
							className="h-8 text-sm flex-1"
							value={(value as string) ?? ""}
							onChange={(e) => onChange(e.target.value)}
							placeholder="#000000"
						/>
					</div>
				);
			case "imageUrl":
				return (
					<Input
						type="url"
						className="h-8 text-sm"
						value={(value as string) ?? ""}
						onChange={(e) => onChange(e.target.value)}
						placeholder="https://..."
					/>
				);
			case "json":
				return (
					<Input
						className="h-8 text-sm font-mono"
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
						placeholder="{}"
					/>
				);
			default:
				return (
					<Input
						className="h-8 text-sm"
						value={String(value ?? "")}
						onChange={(e) => onChange(e.target.value)}
					/>
				);
		}
	};

	return (
		<div className="space-y-1.5">
			<div className="flex items-center gap-1">
				<Label className="text-xs font-medium">{option.label}</Label>
				{option.description && (
					<TooltipProvider>
						<Tooltip>
							<TooltipTrigger asChild>
								<Info className="h-3 w-3 text-muted-foreground cursor-help" />
							</TooltipTrigger>
							<TooltipContent side="right" className="max-w-xs">
								<p className="text-xs">{option.description}</p>
							</TooltipContent>
						</Tooltip>
					</TooltipProvider>
				)}
			</div>
			{renderField()}
		</div>
	);
}

interface ActionBindingsEditorProps {
	actions: WidgetAction[];
	bindings: Record<string, ActionBinding>;
	availableWorkflows: WorkflowEventOption[];
	onBindingChange: (actionId: string, binding: ActionBinding | null) => void;
}

function ActionBindingsEditor({
	actions,
	bindings,
	availableWorkflows,
	onBindingChange,
}: ActionBindingsEditorProps) {
	if (actions.length === 0) {
		return (
			<div className="text-sm text-muted-foreground text-center py-4">
				No actions defined
			</div>
		);
	}

	return (
		<div className="space-y-4">
			{actions.map((action) => (
				<ActionBindingField
					key={action.id}
					action={action}
					binding={bindings[action.id]}
					availableWorkflows={availableWorkflows}
					onChange={(binding) => onBindingChange(action.id, binding)}
				/>
			))}
		</div>
	);
}

interface ActionBindingFieldProps {
	action: WidgetAction;
	binding?: ActionBinding;
	availableWorkflows: WorkflowEventOption[];
	onChange: (binding: ActionBinding | null) => void;
}

function ActionBindingField({
	action,
	binding,
	availableWorkflows,
	onChange,
}: ActionBindingFieldProps) {
	const [isOpen, setIsOpen] = useState(!!binding);

	const bindingType = binding
		? "workflow" in binding
			? "workflow"
			: "command"
		: "none";

	const workflowBinding =
		binding && "workflow" in binding ? binding.workflow : null;

	return (
		<Collapsible open={isOpen} onOpenChange={setIsOpen}>
			<div className="flex items-center justify-between">
				<CollapsibleTrigger className="flex items-center gap-2 text-sm font-medium hover:text-foreground">
					<ChevronRight
						className={cn(
							"h-4 w-4 transition-transform duration-200",
							isOpen && "rotate-90",
						)}
					/>
					<Play className="h-3 w-3 text-muted-foreground" />
					<span>{action.label}</span>
				</CollapsibleTrigger>
				{binding ? (
					<Button
						variant="ghost"
						size="icon"
						className="h-6 w-6"
						onClick={() => onChange(null)}
					>
						<Unlink className="h-3 w-3" />
					</Button>
				) : (
					<Badge variant="outline" className="text-[10px]">
						Unbound
					</Badge>
				)}
			</div>

			<CollapsibleContent className="pt-2 pl-6 space-y-3">
				{action.description && (
					<p className="text-xs text-muted-foreground">{action.description}</p>
				)}

				{action.contextSchema.length > 0 && (
					<div className="text-xs">
						<span className="text-muted-foreground">Context: </span>
						{action.contextSchema.map((f) => f.name).join(", ")}
					</div>
				)}

				<div className="space-y-2">
					<Label className="text-xs">Bind to</Label>
					<Select
						value={bindingType}
						onValueChange={(v) => {
							if (v === "none") {
								onChange(null);
							} else if (v === "workflow") {
								onChange({
									workflow: { flowId: "", inputMappings: {} },
								});
							} else if (v === "command") {
								onChange({
									command: { commandName: "", args: {} },
								});
							}
						}}
					>
						<SelectTrigger className="h-8 text-sm">
							<SelectValue placeholder="Select binding type" />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="none">No binding</SelectItem>
							<SelectItem value="workflow">Workflow</SelectItem>
							<SelectItem value="command">Command</SelectItem>
						</SelectContent>
					</Select>
				</div>

				{bindingType === "workflow" && (
					<div className="space-y-2 border-l-2 border-muted pl-3">
						<Label className="text-xs text-muted-foreground">
							Select Workflow
						</Label>
						<Select
							value={workflowBinding?.flowId ?? ""}
							onValueChange={(flowId) => {
								onChange({
									workflow: {
										flowId,
										inputMappings: workflowBinding?.inputMappings ?? {},
									},
								});
							}}
						>
							<SelectTrigger className="h-8 text-sm">
								<SelectValue placeholder="Choose workflow..." />
							</SelectTrigger>
							<SelectContent>
								{availableWorkflows.length === 0 ? (
									<div className="p-2 text-sm text-muted-foreground text-center">
										No workflows available
									</div>
								) : (
									availableWorkflows.map((wf) => (
										<SelectItem key={wf.flowId} value={wf.flowId}>
											{wf.name}
										</SelectItem>
									))
								)}
							</SelectContent>
						</Select>

						{workflowBinding?.flowId && action.contextSchema.length > 0 && (
							<div className="space-y-2 mt-2">
								<Label className="text-xs text-muted-foreground">
									Input Mappings
								</Label>
								{action.contextSchema.map((field) => (
									<InputMappingField
										key={field.name}
										fieldName={field.name}
										fieldType={field.fieldType}
										value={workflowBinding.inputMappings[field.name]}
										onChange={(v) => {
											const newMappings = {
												...workflowBinding.inputMappings,
											};
											if (v === null) {
												delete newMappings[field.name];
											} else {
												newMappings[field.name] = v;
											}
											onChange({
												workflow: {
													...workflowBinding,
													inputMappings: newMappings,
												},
											});
										}}
									/>
								))}
							</div>
						)}
					</div>
				)}

				{bindingType === "command" && binding && "command" in binding && (
					<div className="space-y-2 border-l-2 border-muted pl-3">
						<Label className="text-xs text-muted-foreground">
							Command Name
						</Label>
						<Input
							className="h-8 text-sm"
							value={binding.command.commandName}
							onChange={(e) =>
								onChange({
									command: {
										...binding.command,
										commandName: e.target.value,
									},
								})
							}
							placeholder="command_name"
						/>
					</div>
				)}
			</CollapsibleContent>
		</Collapsible>
	);
}

interface InputMappingFieldProps {
	fieldName: string;
	fieldType: string;
	value?: BoundValue;
	onChange: (value: BoundValue | null) => void;
}

function InputMappingField({
	fieldName,
	fieldType,
	value,
	onChange,
}: InputMappingFieldProps) {
	const valueType = value ? Object.keys(value)[0] : "literalString";

	return (
		<div className="flex gap-2 items-start">
			<div className="flex-1">
				<Label className="text-xs">{fieldName}</Label>
				<div className="flex gap-1 mt-1">
					<Select
						value={valueType}
						onValueChange={(type) => {
							if (type === "path") {
								onChange({ path: "" });
							} else if (type === "literalString") {
								onChange({ literalString: "" });
							} else if (type === "literalNumber") {
								onChange({ literalNumber: 0 });
							} else if (type === "literalBool") {
								onChange({ literalBool: false });
							}
						}}
					>
						<SelectTrigger className="h-7 w-20 text-xs">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="path">Path</SelectItem>
							<SelectItem value="literalString">Text</SelectItem>
							<SelectItem value="literalNumber">Num</SelectItem>
							<SelectItem value="literalBool">Bool</SelectItem>
						</SelectContent>
					</Select>
					{valueType === "path" && (
						<Input
							className="h-7 text-xs flex-1 font-mono"
							placeholder="$.context.field"
							value={(value && "path" in value ? value.path : "") ?? ""}
							onChange={(e) => onChange({ path: e.target.value })}
						/>
					)}
					{valueType === "literalString" && (
						<Input
							className="h-7 text-xs flex-1"
							value={
								(value && "literalString" in value
									? value.literalString
									: "") ?? ""
							}
							onChange={(e) => onChange({ literalString: e.target.value })}
						/>
					)}
					{valueType === "literalNumber" && (
						<Input
							type="number"
							className="h-7 text-xs flex-1"
							value={
								(value && "literalNumber" in value ? value.literalNumber : 0) ??
								0
							}
							onChange={(e) =>
								onChange({
									literalNumber: Number.parseFloat(e.target.value) || 0,
								})
							}
						/>
					)}
					{valueType === "literalBool" && (
						<Select
							value={String(
								value && "literalBool" in value ? value.literalBool : false,
							)}
							onValueChange={(v) => onChange({ literalBool: v === "true" })}
						>
							<SelectTrigger className="h-7 text-xs flex-1">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="true">True</SelectItem>
								<SelectItem value="false">False</SelectItem>
							</SelectContent>
						</Select>
					)}
				</div>
			</div>
			<Badge variant="outline" className="text-[10px] mt-5">
				{fieldType}
			</Badge>
		</div>
	);
}
