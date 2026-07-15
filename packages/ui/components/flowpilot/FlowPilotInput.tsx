/**
 * FlowPilot A2UI Input Component
 * Input field with UI generation mode toggle
 */

"use client";

import { MessageSquare, Send, Sparkles, Wand2 } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export type FlowPilotMode = "chat" | "ui";

export interface FlowPilotInputProps {
	mode: FlowPilotMode;
	onModeChange?: (mode: FlowPilotMode) => void;
	onSubmit: (message: string, mode: FlowPilotMode) => void;
	placeholder?: string;
	disabled?: boolean;
	isGenerating?: boolean;
	className?: string;
}

export function FlowPilotInput({
	mode,
	onModeChange,
	onSubmit,
	placeholder,
	disabled = false,
	isGenerating = false,
	className,
}: FlowPilotInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);

	const defaultPlaceholder =
		mode === "ui"
			? "Describe the UI you want to create..."
			: "Ask FlowPilot anything...";

	const handleSubmit = useCallback(() => {
		if (!value.trim() || disabled || isGenerating) return;
		onSubmit(value.trim(), mode);
		setValue("");
	}, [value, disabled, isGenerating, onSubmit, mode]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				handleSubmit();
			}
		},
		[handleSubmit],
	);

	useEffect(() => {
		void value;
		if (textareaRef.current) {
			textareaRef.current.style.height = "auto";
			textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 150)}px`;
		}
	}, [value]);

	return (
		<div className={cn("flex flex-col gap-2", className)}>
			<div className="flex items-end gap-2">
				<div className="flex-1 relative">
					<Textarea
						ref={textareaRef}
						value={value}
						onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) =>
							setValue(e.target.value)
						}
						onKeyDown={handleKeyDown}
						placeholder={placeholder ?? defaultPlaceholder}
						disabled={disabled || isGenerating}
						className={cn(
							"min-h-[44px] max-h-[150px] resize-none pr-10",
							mode === "ui" && "border-primary/50",
						)}
						rows={1}
					/>

					{mode === "ui" && (
						<div className="absolute right-2 top-2 pointer-events-none">
							<Sparkles className="w-4 h-4 text-primary/50" />
						</div>
					)}
				</div>

				<Button
					aria-label={mode === "ui" ? "Generate UI" : "Send message"}
					size="icon"
					onClick={handleSubmit}
					disabled={!value.trim() || disabled || isGenerating}
				>
					{mode === "ui" ? (
						<Wand2 className="w-4 h-4" />
					) : (
						<Send className="w-4 h-4" />
					)}
				</Button>
			</div>

			{onModeChange && (
				<div className="flex items-center justify-between px-1">
					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={mode === "chat" ? "secondary" : "ghost"}
									size="sm"
									onClick={() => onModeChange("chat")}
									className="h-7 px-2"
								>
									<MessageSquare className="w-3.5 h-3.5 mr-1" />
									Chat
								</Button>
							</TooltipTrigger>
							<TooltipContent>Ask questions or get help</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={mode === "ui" ? "secondary" : "ghost"}
									size="sm"
									onClick={() => onModeChange("ui")}
									className="h-7 px-2"
								>
									<Sparkles className="w-3.5 h-3.5 mr-1" />
									Generate UI
								</Button>
							</TooltipTrigger>
							<TooltipContent>Generate A2UI components</TooltipContent>
						</Tooltip>
					</div>

					<span className="text-xs text-muted-foreground">
						{mode === "ui" ? "UI Generation Mode" : "Chat Mode"}
					</span>
				</div>
			)}
		</div>
	);
}
