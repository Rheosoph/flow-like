"use client";

import { ArrowUpIcon, ImageIcon, PaperclipIcon, XIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useRef, useState } from "react";
import { Button, Textarea } from "../../index";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";

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
		<div className="w-full flex flex-col items-center gap-5 px-4 pt-14 pb-8 shrink-0">
			<div className="flex flex-col items-center gap-2 text-center">
				<h1 className="text-3xl md:text-4xl font-bold tracking-tight">
					What do you want to{" "}
					<span className="bg-linear-to-r from-primary via-purple-500 to-primary bg-clip-text text-transparent">
						build
					</span>
					?
				</h1>
				<p className="text-sm md:text-base text-muted-foreground">
					Ask FlowPilot to create apps, find packages, or navigate Flow-Like.
				</p>
			</div>
			<div className="relative w-full max-w-2xl">
				{/* Aurora halo: two soft gradient blobs breathing behind the composer. */}
				<div
					className="pointer-events-none absolute -left-12 -top-10 size-48 rounded-full bg-primary/20 blur-3xl motion-safe:animate-pulse animation-duration-[6s]"
					aria-hidden="true"
				/>
				<div
					className="pointer-events-none absolute -right-12 -bottom-10 size-48 rounded-full bg-purple-600/15 blur-3xl motion-safe:animate-pulse animation-duration-[8s]"
					aria-hidden="true"
				/>
				<div className="relative flex flex-col w-full rounded-2xl border border-border/70 bg-card/80 backdrop-blur-2xl px-3 py-2.5 shadow-lg shadow-black/5 transition-all duration-300 focus-within:border-primary/50 focus-within:shadow-xl focus-within:shadow-primary/10">
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
										className="rounded-full hover:text-destructive outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
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
							className="min-h-9 max-h-40 resize-none border-0 bg-transparent dark:bg-transparent shadow-none focus-visible:ring-0 py-1.5 px-2"
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
							className="rounded-full shrink-0 mb-0.5 text-muted-foreground hover:text-foreground outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
							onClick={() => fileInputRef.current?.click()}
							aria-label="Attach images"
						>
							<PaperclipIcon className="size-4" />
						</Button>
						<Button
							size="icon"
							className="rounded-full shrink-0 mb-0.5 shadow-sm transition-all duration-200 hover:shadow-md outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
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
						className="h-8 rounded-full text-xs text-muted-foreground border-border/60 bg-background/60 backdrop-blur-sm transition-all hover:bg-primary/10 hover:text-primary hover:border-primary/40 hover:shadow-sm motion-safe:hover:-translate-y-px outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						onClick={() => submit(suggestion)}
					>
						{suggestion}
					</Button>
				))}
			</div>
		</div>
	);
}
