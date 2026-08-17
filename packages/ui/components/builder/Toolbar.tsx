"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Clipboard,
	Code,
	Copy,
	Eye,
	FileText,
	Save,
	Scissors,
	Trash2,
} from "lucide-react";
import { cn } from "../../lib";
import { Button } from "../ui/button";
import { Separator } from "../ui/separator";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";
import { useBuilder } from "./BuilderContext";

export interface PageInfo {
	id: string;
	name: string;
}

export interface ToolbarProps {
	className?: string;
	onSave?: () => void;
	onPreview?: () => void;
	pages?: PageInfo[];
	currentPageId?: string;
	onPageChange?: (pageId: string) => void;
}

export function Toolbar({
	className,
	onSave,
	onPreview,
	pages,
	currentPageId,
	onPageChange,
}: ToolbarProps) {
	const { t } = useTranslation("flow");
	const { copy, cut, paste, selection, deleteComponents, devMode, setDevMode } =
		useBuilder();

	const hasSelection = selection.componentIds.length > 0;
	const hasPages = pages && pages.length > 0;

	// Debug logging - check console for these values
	if (hasPages) {
		console.log("[Toolbar] Pages array:", JSON.stringify(pages, null, 2));
		console.log("[Toolbar] currentPageId:", currentPageId);
		console.log("[Toolbar] First page id:", pages[0]?.id);
	}

	return (
		<TooltipProvider delayDuration={300}>
			<div
				className={cn(
					"flex items-center gap-1 h-10 px-2 border-b bg-background",
					className,
				)}
			>
				{/* Page Switcher */}
				{hasPages && (
					<>
						<div className="flex items-center gap-2">
							<FileText className="h-4 w-4 text-muted-foreground" />
							<select
								value={currentPageId ?? ""}
								onChange={(e) => {
									const selectedId = e.target.value;
									if (selectedId && onPageChange) {
										onPageChange(selectedId);
									}
								}}
								className="h-7 w-[180px] text-xs px-2 rounded-md border border-input bg-background cursor-pointer"
							>
								{pages.map((page) => (
									<option key={page.id} value={page.id}>
										{page.name}
									</option>
								))}
							</select>
						</div>
						<Separator orientation="vertical" className="h-6" />
					</>
				)}

				{/* Clipboard */}
				<div className="flex items-center gap-0.5">
					<ToolbarButton
						icon={Copy}
						label="Copy"
						shortcut="⌘C"
						onClick={copy}
						disabled={!hasSelection}
					/>
					<ToolbarButton
						icon={Scissors}
						label="Cut"
						shortcut="⌘X"
						onClick={cut}
						disabled={!hasSelection}
					/>
					<ToolbarButton
						icon={Clipboard}
						label="Paste"
						shortcut="⌘V"
						onClick={() => paste()}
					/>
					<ToolbarButton
						icon={Trash2}
						label={t('delete', 'Delete')}
						shortcut="⌫"
						onClick={() => deleteComponents(selection.componentIds)}
						disabled={!hasSelection}
					/>
				</div>

				<div className="flex-1" />

				{/* Dev Mode */}
				<ToolbarButton
					icon={Code}
					label={t('devModeJsonEditor', 'Dev Mode (JSON Editor)')}
					onClick={() => setDevMode(!devMode)}
					active={devMode}
				/>

				<Separator orientation="vertical" className="h-6" />

				{/* Save */}
				<ToolbarButton
					icon={Save}
					label={t('save', 'Save')}
					shortcut="⌘S"
					onClick={onSave}
				/>

				<Separator orientation="vertical" className="h-6" />

				{/* Preview */}
				<ToolbarButton
					icon={Eye}
					label="Preview"
					shortcut="⌘P"
					onClick={onPreview}
				/>
			</div>
		</TooltipProvider>
	);
}

interface ToolbarButtonProps {
	icon: React.ComponentType<{ className?: string }>;
	label: string;
	shortcut?: string;
	onClick?: () => void;
	disabled?: boolean;
	active?: boolean;
}

function ToolbarButton({
	icon: Icon,
	label,
	shortcut,
	onClick,
	disabled,
	active,
}: ToolbarButtonProps) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant={active ? "secondary" : "ghost"}
					size="sm"
					onClick={onClick}
					disabled={disabled}
					className="h-7 w-7 p-0"
				>
					<Icon className="h-4 w-4" />
				</Button>
			</TooltipTrigger>
			<TooltipContent side="bottom">
				<p className="font-medium">{label}</p>
				{shortcut && (
					<p className="text-xs text-muted-foreground">{shortcut}</p>
				)}
			</TooltipContent>
		</Tooltip>
	);
}
