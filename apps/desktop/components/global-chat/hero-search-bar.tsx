"use client";

import { Button, Textarea } from "@flow-like/flow-like-ui";
import {
	ArrowUpIcon,
	ImageIcon,
	PaperclipIcon,
	SparklesIcon,
	XIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useRef, useState } from "react";
import { useGlobalChatStore } from "../../lib/global-chat-store";

const SUGGESTIONS = [
	"Create a new app",
	"What can I build with Flow-Like?",
	"Show me the package store",
	"Switch my profile",
];

export function HeroSearchBar() {
	const router = useRouter();
	const setDraft = useGlobalChatStore((s) => s.setDraft);
	const [value, setValue] = useState("");
	const [files, setFiles] = useState<File[]>([]);
	const fileInputRef = useRef<HTMLInputElement>(null);

	const submit = (prompt: string) => {
		const trimmed = prompt.trim();
		if (!trimmed && files.length === 0) return;
		setDraft({ prompt: trimmed, files: files.length > 0 ? files : undefined });
		router.push("/chat");
	};

	return (
		<div className="w-full flex flex-col items-center gap-4 px-4 pt-10 pb-5">
			<div className="flex flex-col items-center gap-1.5 text-center">
				<h1 className="text-2xl font-semibold tracking-tight">
					What do you want to build?
				</h1>
				<p className="text-sm text-muted-foreground">
					Ask FlowPilot to create apps, find packages, or navigate Flow-Like.
				</p>
			</div>
			<div className="relative w-full max-w-2xl">
				<div
					className="pointer-events-none absolute -inset-1 rounded-3xl bg-primary/10 blur-xl opacity-70"
					aria-hidden="true"
				/>
				<div className="relative flex flex-col w-full rounded-2xl border border-border/60 bg-background/80 backdrop-blur-xl shadow-lg px-3 py-2 focus-within:border-primary/50 focus-within:shadow-primary/10 transition-all">
					{files.length > 0 && (
						<div className="flex flex-wrap items-center gap-1.5 pb-2">
							{files.map((file, index) => (
								<span
									key={`${file.name}-${index}`}
									className="flex items-center gap-1 rounded-full bg-primary/10 text-primary px-2 py-0.5 text-xs"
								>
									<ImageIcon className="size-3" />
									<span className="max-w-32 truncate">{file.name}</span>
									<button
										type="button"
										aria-label={`Remove ${file.name}`}
										className="hover:text-destructive"
										onClick={() =>
											setFiles((prev) => prev.filter((_, i) => i !== index))
										}
									>
										<XIcon className="size-3" />
									</button>
								</span>
							))}
						</div>
					)}
					<div className="flex items-end gap-2 w-full">
						<SparklesIcon className="size-5 text-primary shrink-0 mb-2" />
						<Textarea
							value={value}
							onChange={(e) => setValue(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter" && !e.shiftKey) {
									e.preventDefault();
									submit(value);
								}
							}}
							placeholder="Ask FlowPilot anything, or describe what you want to build…"
							rows={1}
							className="min-h-9 max-h-40 resize-none border-0 bg-transparent shadow-none focus-visible:ring-0 py-1.5"
						/>
						<input
							ref={fileInputRef}
							type="file"
							accept="image/*"
							multiple
							className="hidden"
							onChange={(e) => {
								const selected = Array.from(e.target.files ?? []);
								if (selected.length > 0)
									setFiles((prev) => [...prev, ...selected]);
								e.target.value = "";
							}}
						/>
						<Button
							variant="ghost"
							size="icon"
							className="rounded-full shrink-0 mb-0.5 text-muted-foreground"
							onClick={() => fileInputRef.current?.click()}
							aria-label="Attach images"
						>
							<PaperclipIcon className="size-4" />
						</Button>
						<Button
							size="icon"
							className="rounded-full shrink-0 mb-0.5"
							onClick={() => submit(value)}
							disabled={!value.trim() && files.length === 0}
							aria-label="Send"
						>
							<ArrowUpIcon className="size-4" />
						</Button>
					</div>
				</div>
			</div>
			<div className="flex flex-wrap items-center justify-center gap-2 max-w-2xl">
				{SUGGESTIONS.map((suggestion) => (
					<Button
						key={suggestion}
						variant="outline"
						size="sm"
						className="h-8 rounded-full text-xs text-foreground/80 border-border/60 bg-background/60 backdrop-blur-sm hover:bg-primary/10 hover:text-primary hover:border-primary/40 transition-colors"
						onClick={() => submit(suggestion)}
					>
						<SparklesIcon className="size-3 mr-1 opacity-70" />
						{suggestion}
					</Button>
				))}
			</div>
		</div>
	);
}
