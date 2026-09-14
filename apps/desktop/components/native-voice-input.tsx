"use client";

import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import { Textarea } from "@flow-like/flow-like-ui/components/ui/textarea";
import { useSpeechRecognition } from "@flow-like/flow-like-ui/components/voice/use-speech-recognition";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { Mic, Send, Square, X } from "lucide-react";
import { useEffect, useState } from "react";

/** Native voice entry keeps the transcript editable and uses the normal FlowPilot send path. */
export function NativeVoiceInput({
	onClose,
	scope,
}: { onClose: () => void; scope: string }) {
	const [text, setText] = useState("");
	const [error, setError] = useState<string>();
	const speech = useSpeechRecognition({
		continuous: false,
		onResult: (final, interim) =>
			setText([final, interim].filter(Boolean).join(" ")),
		onEnd: setText,
		onError: () =>
			setError(
				"Dictation could not start. Check microphone and speech permissions, or use keyboard dictation.",
			),
	});
	useEffect(() => {
		const stopWhenHidden = () => {
			if (document.visibilityState !== "visible") speech.cancel();
		};
		document.addEventListener("visibilitychange", stopWhenHidden);
		window.addEventListener("blur", speech.cancel);
		return () => {
			document.removeEventListener("visibilitychange", stopWhenHidden);
			window.removeEventListener("blur", speech.cancel);
			speech.cancel();
		};
	}, [speech.cancel]);
	return (
		<section
			className="mx-auto my-3 w-full max-w-3xl space-y-3 rounded-xl border bg-card p-4"
			aria-label="Voice input for FlowPilot"
		>
			<div className="flex items-center justify-between">
				<h2 className="text-sm font-medium">Speak to FlowPilot</h2>
				<Button
					variant="ghost"
					size="icon"
					aria-label="Close voice input"
					onClick={() => {
						speech.cancel();
						onClose();
					}}
				>
					<X className="size-4" />
				</Button>
			</div>
			<Textarea
				aria-label="Dictated message"
				placeholder="Your message"
				value={text}
				onChange={(event) => setText(event.target.value)}
			/>
			{(!speech.isSupported || error) && (
				<p role="status" className="text-xs text-muted-foreground">
					{error ||
						"Use your keyboard's dictation button to speak, or type a message."}
				</p>
			)}
			<div className="flex justify-between gap-2">
				<Button
					variant="outline"
					disabled={!speech.isSupported}
					onClick={() => {
						setError(undefined);
						if (speech.isListening) speech.stop();
						else {
							speech.reset();
							speech.start();
						}
					}}
				>
					{speech.isListening ? (
						<Square className="mr-2 size-4" />
					) : (
						<Mic className="mr-2 size-4" />
					)}
					{speech.isListening ? "Stop dictation" : "Start dictation"}
				</Button>
				<Button
					disabled={!text.trim() || speech.isListening}
					onClick={() => {
						speech.cancel();
						useGlobalChatStore
							.getState()
							.setDraft({ prompt: text.trim(), nativeScope: scope });
						onClose();
					}}
				>
					<Send className="mr-2 size-4" />
					Send
				</Button>
			</div>
		</section>
	);
}
