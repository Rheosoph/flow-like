/**
 * FlowPilot A2UI Preview Component
 * Renders a live preview of generated UI in the Spotlight
 */

"use client";

import { Check, Code, Eye, Loader2, RefreshCw, Wand2, X } from "lucide-react";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import { cn } from "../../lib/utils";
import type { DataEntry, SurfaceComponent } from "../a2ui/types";
import { Button } from "../ui/button";
import { ScrollArea } from "../ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";

export interface A2UIPreviewProps {
	rootComponentId: string | null;
	components: SurfaceComponent[];
	dataModel: DataEntry[];
	isGenerating?: boolean;
	progress?: number;
	error?: string | null;
	onAccept?: () => void;
	onReject?: () => void;
	onRegenerate?: () => void;
	onEdit?: (request: string) => void;
	className?: string;
}

export function A2UIPreview({
	rootComponentId,
	components,
	dataModel,
	isGenerating = false,
	progress = 0,
	error,
	onAccept,
	onReject,
	onRegenerate,
	className,
}: A2UIPreviewProps) {
	const [activeTab, setActiveTab] = useState<"preview" | "json">("preview");

	const jsonOutput = useMemo(() => {
		return JSON.stringify({ rootComponentId, components, dataModel }, null, 2);
	}, [rootComponentId, components, dataModel]);

	const hasContent = rootComponentId && components.length > 0;

	if (isGenerating) {
		return (
			<div
				className={cn(
					"flex flex-col items-center justify-center p-6 gap-3",
					className,
				)}
			>
				<Loader2 className="w-8 h-8 animate-spin text-primary" />
				<span className="text-sm text-muted-foreground">Generating UI...</span>
				<div className="w-full max-w-xs h-2 bg-muted rounded-full overflow-hidden">
					<div
						className="h-full bg-primary transition-all duration-300"
						style={{ width: `${progress}%` }}
					/>
				</div>
			</div>
		);
	}

	if (error) {
		return (
			<div
				className={cn(
					"flex flex-col items-center justify-center p-6 gap-3",
					className,
				)}
			>
				<X className="w-8 h-8 text-destructive" />
				<span className="text-sm text-destructive">{error}</span>
				{onRegenerate && (
					<Button variant="outline" size="sm" onClick={onRegenerate}>
						<RefreshCw className="w-4 h-4 mr-2" />
						Try Again
					</Button>
				)}
			</div>
		);
	}

	if (!hasContent) {
		return (
			<div
				className={cn(
					"flex flex-col items-center justify-center p-6 gap-3 text-muted-foreground",
					className,
				)}
			>
				<Wand2 className="w-8 h-8" />
				<span className="text-sm">Describe a UI to generate</span>
			</div>
		);
	}

	return (
		<div className={cn("flex flex-col gap-3", className)}>
			<Tabs
				value={activeTab}
				onValueChange={(v: string) => setActiveTab(v as "preview" | "json")}
			>
				<div className="flex items-center justify-between">
					<TabsList>
						<TabsTrigger value="preview" className="gap-1.5">
							<Eye className="w-3.5 h-3.5" />
							Preview
						</TabsTrigger>
						<TabsTrigger value="json" className="gap-1.5">
							<Code className="w-3.5 h-3.5" />
							JSON
						</TabsTrigger>
					</TabsList>

					<div className="flex gap-2">
						{onRegenerate && (
							<Button variant="ghost" size="sm" onClick={onRegenerate}>
								<RefreshCw className="w-4 h-4" />
							</Button>
						)}
						{onReject && (
							<Button variant="ghost" size="sm" onClick={onReject}>
								<X className="w-4 h-4" />
							</Button>
						)}
						{onAccept && (
							<Button variant="default" size="sm" onClick={onAccept}>
								<Check className="w-4 h-4 mr-1" />
								Accept
							</Button>
						)}
					</div>
				</div>

				<TabsContent value="preview" className="mt-3">
					<ScrollArea className="h-[300px] rounded-lg border bg-background">
						<div className="p-4">
							<PreviewRenderer
								rootComponentId={rootComponentId}
								components={components}
								dataModel={dataModel}
							/>
						</div>
					</ScrollArea>
				</TabsContent>

				<TabsContent value="json" className="mt-3">
					<ScrollArea className="h-[300px] rounded-lg border bg-muted/30">
						<pre className="p-4 text-xs font-mono whitespace-pre-wrap">
							{jsonOutput}
						</pre>
					</ScrollArea>
				</TabsContent>
			</Tabs>

			<div className="text-xs text-muted-foreground text-center">
				{components.length} components • {dataModel.length} data entries
			</div>
		</div>
	);
}

interface PreviewRendererProps {
	rootComponentId: string;
	components: SurfaceComponent[];
	dataModel: DataEntry[];
}

function PreviewRenderer({
	rootComponentId,
	components,
	dataModel,
}: PreviewRendererProps) {
	const componentMap = useMemo(() => {
		const map: Record<string, SurfaceComponent> = {};
		for (const comp of components) {
			map[comp.id] = comp;
		}
		return map;
	}, [components]);

	const dataMap = useMemo(() => {
		const map: Record<string, unknown> = {};
		for (const entry of dataModel) {
			map[entry.path] = entry.value;
		}
		return map;
	}, [dataModel]);

	const renderComponent = useCallback(
		(id: string): React.ReactNode => {
			const surfaceComp = componentMap[id];
			if (!surfaceComp) return null;

			const { component, style } = surfaceComp;
			const className = style?.className ?? "";

			const resolveValue = (
				bound:
					| {
							literalString?: string;
							literalNumber?: number;
							literalBool?: boolean;
							path?: string;
					  }
					| undefined,
			): unknown => {
				if (!bound) return undefined;
				if ("literalString" in bound) return bound.literalString;
				if ("literalNumber" in bound) return bound.literalNumber;
				if ("literalBool" in bound) return bound.literalBool;
				if ("path" in bound && bound.path)
					return dataMap[bound.path] ?? bound.path;
				return undefined;
			};

			const getChildren = (): string[] => {
				const children = (
					component as { children?: { explicitList?: string[] } }
				).children;
				if (children?.explicitList) return children.explicitList;
				return [];
			};

			switch (component.type) {
				case "column":
					return (
						<div
							key={id}
							className={cn("flex flex-col", className)}
							style={{ gap: (component as { gap?: string }).gap }}
						>
							{getChildren().map(renderComponent)}
						</div>
					);

				case "row":
					return (
						<div
							key={id}
							className={cn("flex flex-row", className)}
							style={{ gap: (component as { gap?: string }).gap }}
						>
							{getChildren().map(renderComponent)}
						</div>
					);

				case "text": {
					const textComp = component as {
						content?: { literalString?: string; path?: string };
						variant?: string;
						size?: string;
					};
					const content = String(resolveValue(textComp.content) ?? "");
					return (
						<span key={id} className={cn(className)}>
							{content}
						</span>
					);
				}

				case "button": {
					const btnComp = component as {
						label?: { literalString?: string; path?: string };
						variant?: string;
					};
					const label = String(resolveValue(btnComp.label) ?? "Button");
					return (
						<button
							type="button"
							key={id}
							className={cn(
								"inline-flex items-center justify-center rounded-md text-sm font-medium px-4 py-2",
								"bg-primary text-primary-foreground hover:bg-primary/90",
								className,
							)}
						>
							{label}
						</button>
					);
				}

				case "textField": {
					const tfComp = component as {
						placeholder?: { literalString?: string; path?: string };
						label?: string;
					};
					const placeholder = String(resolveValue(tfComp.placeholder) ?? "");
					const inputId = `${id}-preview-input`;
					return (
						<div key={id} className={cn("flex flex-col gap-1.5", className)}>
							{tfComp.label && (
								<label className="text-sm font-medium" htmlFor={inputId}>
									{tfComp.label}
								</label>
							)}
							<input
								id={inputId}
								type="text"
								placeholder={placeholder}
								className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
							/>
						</div>
					);
				}

				case "card":
					return (
						<div
							key={id}
							className={cn("rounded-lg border bg-card p-4", className)}
						>
							{getChildren().map(renderComponent)}
						</div>
					);

				case "badge": {
					const badgeComp = component as {
						content?: { literalString?: string; path?: string };
					};
					const content = String(resolveValue(badgeComp.content) ?? "");
					return (
						<span
							key={id}
							className={cn(
								"inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-semibold",
								"bg-secondary text-secondary-foreground",
								className,
							)}
						>
							{content}
						</span>
					);
				}

				case "avatar": {
					const avatarComp = component as {
						fallback?: { literalString?: string; path?: string };
						size?: string;
					};
					const fallback = String(resolveValue(avatarComp.fallback) ?? "?");
					const sizeClasses = {
						sm: "w-8 h-8 text-xs",
						md: "w-10 h-10 text-sm",
						lg: "w-12 h-12 text-base",
						xl: "w-16 h-16 text-lg",
					};
					const size = (avatarComp.size ?? "md") as keyof typeof sizeClasses;
					return (
						<div
							key={id}
							className={cn(
								"rounded-full bg-muted flex items-center justify-center font-medium",
								sizeClasses[size],
								className,
							)}
						>
							{fallback.slice(0, 2).toUpperCase()}
						</div>
					);
				}

				case "userProfile": {
					const userProfileComp = component as {
						value?: { literalString?: string; path?: string } | string;
						fallbackLabel?: { literalString?: string; path?: string };
					};
					const resolvedProfileValue =
						typeof userProfileComp.value === "string"
							? userProfileComp.value
							: resolveValue(userProfileComp.value);
					const label = String(
						resolvedProfileValue ??
							resolveValue(userProfileComp.fallbackLabel) ??
							"User",
					);
					const initials = label.slice(0, 2).toUpperCase();
					return (
						<div
							key={id}
							className={cn(
								"inline-flex min-w-0 max-w-full items-center gap-2 rounded-lg border bg-card px-3 py-2 text-sm",
								className,
							)}
						>
							<div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium">
								{initials}
							</div>
							<div className="min-w-0">
								<div className="truncate font-medium">{label}</div>
								<div className="truncate text-xs text-muted-foreground">
									User profile
								</div>
							</div>
						</div>
					);
				}

				case "switch":
					return (
						<button
							type="button"
							key={id}
							className={cn(
								"relative inline-flex h-6 w-11 items-center rounded-full bg-primary",
								className,
							)}
						>
							<span className="inline-block h-4 w-4 transform rounded-full bg-white translate-x-6" />
						</button>
					);

				case "divider":
					return <hr key={id} className={cn("border-border", className)} />;

				case "spinner":
					return (
						<Loader2
							key={id}
							className={cn("w-6 h-6 animate-spin", className)}
						/>
					);

				default:
					return (
						<div
							key={id}
							className={cn(
								"p-2 border border-dashed rounded text-xs text-muted-foreground",
								className,
							)}
						>
							{component.type}
							{getChildren().length > 0 && (
								<div className="mt-1">{getChildren().map(renderComponent)}</div>
							)}
						</div>
					);
			}
		},
		[componentMap, dataMap],
	);

	return <>{renderComponent(rootComponentId)}</>;
}
