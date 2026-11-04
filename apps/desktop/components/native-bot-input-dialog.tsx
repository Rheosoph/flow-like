"use client";

import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "@flow-like/flow-like-ui/components/ui/dialog";
import { Textarea } from "@flow-like/flow-like-ui/components/ui/textarea";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

export type NativeBotInputOperation =
	| "voice"
	| "document"
	| "photo"
	| "summarize"
	| "extract";
export interface NativeBotInput {
	id: string;
	scope: string;
	botId: string;
	operation: NativeBotInputOperation;
	text?: string;
}
export interface NativeBotInputResult {
	text?: string;
	files?: string[];
	available?: boolean;
	cancelled?: boolean;
}

const titles: Record<NativeBotInputOperation, string> = {
	voice: "Dictate a message",
	document: "Scan a document",
	photo: "Take a photo",
	summarize: "Summarize on this device",
	extract: "Extract details on this device",
};

/** Capture and local processing return editable content. Sending is a separate, explicit action. */
export function NativeBotInputDialog({
	input,
	isCurrent,
	onSend,
	onClose,
}: {
	input: NativeBotInput;
	isCurrent: () => boolean;
	onSend: (result: NativeBotInputResult) => Promise<void>;
	onClose: () => void;
}) {
	const [text, setText] = useState(input.text ?? "");
	const [files, setFiles] = useState<string[]>([]);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [processed, setProcessed] = useState(false);
	const active = useRef(true);
	const current = useRef(isCurrent);
	current.current = isCurrent;
	const local =
		input.operation === "summarize" || input.operation === "extract";
	useEffect(() => {
		active.current = true;
		return () => {
			active.current = false;
			void invoke("native_bot_cancel_request", { requestId: input.id }).catch(
				() => {},
			);
		};
	}, [input.id]);
	const run = async () => {
		if (busy || !current.current()) return;
		if (local && new TextEncoder().encode(text).byteLength > 8192) {
			setError(
				"Process up to 8 KiB of text at a time. Select a shorter passage.",
			);
			return;
		}
		setBusy(true);
		setError(undefined);
		try {
			const result = await invoke<NativeBotInputResult>("native_bot_request", {
				requestId: input.id,
				options: {
					scope: input.scope,
					botId: input.botId,
					operation: input.operation,
					...(local ? { text } : {}),
				},
			});
			if (!active.current || !current.current()) return;
			if (result.cancelled) return;
			if (result.available === false)
				throw new Error(
					"On-device processing is unavailable. Your text was not processed. Try again when the model is ready.",
				);
			if (result.text !== undefined) setText(result.text);
			if (result.files) setFiles(result.files);
			setProcessed(true);
		} catch (cause) {
			if (active.current && current.current())
				setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			if (active.current) setBusy(false);
		}
	};
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>{titles[input.operation]}</DialogTitle>
					<DialogDescription>
						{local
							? "Processing stays on this device. Review the result before sending it to your bot."
							: "Review your message and attachments before sending them to your bot."}
					</DialogDescription>
				</DialogHeader>
				<Textarea
					aria-label={
						local && !processed ? "Text to process" : "Message to bot"
					}
					placeholder={
						local ? "Paste text to process locally" : "Add a message"
					}
					value={text}
					maxLength={local ? 8192 : 65_536}
					onChange={(event) => setText(event.target.value)}
					rows={8}
					disabled={busy}
				/>
				{files.length > 0 && (
					<ul className="space-y-1 text-sm">
						{files.map((path) => (
							<li
								key={path}
								className="flex items-center justify-between gap-2"
							>
								<span className="truncate">{path.split("/").pop()}</span>
								<Button
									variant="ghost"
									size="sm"
									onClick={() =>
										setFiles((items) => items.filter((item) => item !== path))
									}
								>
									Remove
								</Button>
							</li>
						))}
					</ul>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<div className="flex flex-wrap justify-between gap-2">
					<Button
						variant="outline"
						disabled={busy || (local && !text.trim())}
						onClick={() => void run()}
					>
						{busy
							? "Working…"
							: local
								? "Process locally"
								: input.operation === "voice"
									? "Start dictation"
									: "Open camera"}
					</Button>
					<Button
						disabled={busy || (!text.trim() && files.length === 0)}
						onClick={async () => {
							if (!current.current()) return;
							setBusy(true);
							try {
								await onSend({ text: text.trim(), files });
								if (active.current && current.current()) onClose();
							} catch (cause) {
								if (active.current && current.current())
									setError(
										cause instanceof Error ? cause.message : String(cause),
									);
							} finally {
								if (active.current) setBusy(false);
							}
						}}
					>
						Send to bot
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
