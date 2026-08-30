"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, Send, Wand2, X } from "lucide-react";
import { useCallback, useState } from "react";
import { useA2UIGeneration } from "../../lib/flowpilot/use-a2ui-generation";
import { cn } from "../../lib/utils";
import type { A2UIComponent, Children, SurfaceComponent } from "../a2ui/types";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { useBuilder } from "./BuilderContext";
import { ROOT_ID } from "./WidgetBuilder";
import { normalizeComponents } from "./componentDefaults";

export interface FlowPilotActionProps {
	onGenerate?: (request: string) => Promise<string>;
	className?: string;
}

export function FlowPilotAction({
	onGenerate,
	className,
}: FlowPilotActionProps) {
	const { t } = useTranslation("flow");
	const [isOpen, setIsOpen] = useState(false);
	const [prompt, setPrompt] = useState("");
	const { selection, components, updateComponent, addComponent, getComponent } =
		useBuilder();

	const hasSelection = selection.componentIds.length > 0;
	const selectedComponents = selection.componentIds
		.map((id) => getComponent(id))
		.filter((c): c is SurfaceComponent => c !== undefined);

	const { isGenerating, progress, error, preview, generate, clearPreview } =
		useA2UIGeneration({
			onGenerate,
			existingComponents: selectedComponents.map((c) => JSON.stringify(c)),
			onComplete: (rawComponents) => {
				// Validate and normalize AI-generated components
				const newComponents = normalizeComponents(rawComponents, true);

				// Get root component BEFORE adding new components (to avoid stale closure)
				const rootComponent = getComponent(ROOT_ID);

				// Collect all child IDs referenced within the new components
				const referencedChildIds = new Set<string>();
				for (const comp of newComponents) {
					const childrenData = (
						comp.component as unknown as Record<string, unknown>
					)?.children as Children | undefined;
					if (childrenData && "explicitList" in childrenData) {
						for (const childId of childrenData.explicitList) {
							referencedChildIds.add(childId);
						}
					}
				}

				// Find top-level components (not referenced as children of other new components)
				const topLevelIds: string[] = [];
				for (const comp of newComponents) {
					if (!referencedChildIds.has(comp.id) && comp.id !== ROOT_ID) {
						topLevelIds.push(comp.id);
					}
				}

				// Add all components to the map
				for (const comp of newComponents) {
					const existing = getComponent(comp.id);
					if (existing) {
						updateComponent(comp.id, comp);
					} else {
						addComponent(comp);
					}
				}

				// Add top-level components to the root's children list
				if (topLevelIds.length > 0 && rootComponent) {
					const rootChildrenData = (
						rootComponent.component as unknown as Record<string, unknown>
					)?.children as Children | undefined;
					const existingChildren =
						rootChildrenData && "explicitList" in rootChildrenData
							? [...rootChildrenData.explicitList]
							: [];

					const newChildren = [...existingChildren];
					for (const id of topLevelIds) {
						if (!newChildren.includes(id)) {
							newChildren.push(id);
						}
					}

					updateComponent(ROOT_ID, {
						component: {
							...rootComponent.component,
							children: { explicitList: newChildren },
						} as A2UIComponent,
					});
				}

				setPrompt("");
				setIsOpen(false);
			},
			onError: (error) => {
				console.error("FlowPilot error:", error);
			},
		});

	const handleSubmit = useCallback(async () => {
		if (!prompt.trim()) return;

		const context = hasSelection
			? `Modify the following selected component(s):\n${JSON.stringify(selectedComponents, null, 2)}\n\nUser request: ${prompt}`
			: prompt;

		await generate(context);
	}, [prompt, hasSelection, selectedComponents, generate]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				handleSubmit();
			}
		},
		[handleSubmit],
	);

	return (
		<Popover open={isOpen} onOpenChange={setIsOpen}>
			<Tooltip>
				<TooltipTrigger asChild>
					<PopoverTrigger asChild>
						<Button
							variant={hasSelection ? "secondary" : "ghost"}
							size="sm"
							className={cn("h-7 px-2 gap-1.5", className)}
						>
							<Wand2 className="h-4 w-4" />
							<span className="text-xs">{t("flowpilot", "FlowPilot")}</span>
						</Button>
					</PopoverTrigger>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					<p className="font-medium">
						{t("editWithFlowpilot", "Edit with FlowPilot")}
					</p>
					<p className="text-xs text-muted-foreground">{`⌘⇧F`}</p>
				</TooltipContent>
			</Tooltip>
			<PopoverContent className="w-80 p-3" align="end">
				<div className="space-y-3">
					<div className="space-y-1.5">
						<Label className="text-xs font-medium flex items-center gap-1.5">
							<Wand2 className="h-3.5 w-3.5" />
							{hasSelection
								? t("editSelection", "Edit Selection")
								: "Generate UI"}
						</Label>
						{hasSelection && (
							<p className="text-xs text-muted-foreground">
								{t("countComponentsSelected", {
									defaultValue_one: "{{count}} component selected",
									defaultValue_other: "{{count}} components selected",
									count: selection.componentIds.length,
								})}
							</p>
						)}
					</div>

					<div className="relative">
						<Input
							placeholder={
								hasSelection
									? t(
											"egMakeThisButtonLargerAndBlue",
											"e.g., Make this button larger and blue...",
										)
									: t(
											"egCreateALoginFormWithEmailAndPassword",
											"e.g., Create a login form with email and password...",
										)
							}
							value={prompt}
							onChange={(e) => setPrompt(e.target.value)}
							onKeyDown={handleKeyDown}
							disabled={isGenerating}
							className="pr-16 text-sm"
						/>
						<div className="absolute right-1 top-1/2 -translate-y-1/2 flex gap-0.5">
							{prompt && (
								<Button
									variant="ghost"
									size="sm"
									className="h-6 w-6 p-0"
									onClick={() => setPrompt("")}
									disabled={isGenerating}
								>
									<X className="h-3.5 w-3.5" />
								</Button>
							)}
							<Button
								variant="ghost"
								size="sm"
								className="h-6 w-6 p-0"
								onClick={handleSubmit}
								disabled={!prompt.trim() || isGenerating}
							>
								{isGenerating ? (
									<Loader2 className="h-3.5 w-3.5 animate-spin" />
								) : (
									<Send className="h-3.5 w-3.5" />
								)}
							</Button>
						</div>
					</div>

					{isGenerating && (
						<div className="space-y-1.5">
							<div className="flex items-center justify-between text-xs">
								<span className="text-muted-foreground">Generating...</span>
								<span className="text-muted-foreground">{`${progress}%`}</span>
							</div>
							<div className="h-1.5 bg-muted rounded-full overflow-hidden">
								<div
									className="h-full bg-primary transition-all duration-300"
									style={{ width: `${progress}%` }}
								/>
							</div>
						</div>
					)}

					{error && <p className="text-xs text-destructive">{error}</p>}

					<div className="text-xs text-muted-foreground border-t pt-2">
						<p className="font-medium mb-1">{t("tips", "Tips:")}</p>
						<ul className="space-y-0.5 list-disc list-inside">
							<li>
								{t(
									"selectComponentsFirstToEditThem",
									"Select components first to edit them",
								)}
							</li>
							<li>
								{t(
									"beSpecificAboutColorsSizesAndLayout",
									"Be specific about colors, sizes, and layout",
								)}
							</li>
							<li>
								{t(
									"mentionTailwindClassesForPreciseStyling",
									"Mention Tailwind classes for precise styling",
								)}
							</li>
						</ul>
					</div>
				</div>
			</PopoverContent>
		</Popover>
	);
}
