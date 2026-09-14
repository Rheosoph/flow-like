"use client";

import { Interaction } from "@flow-like/flow-like-ui/components/interfaces/chat-default/interaction";
import { submitInteractionResponse } from "@flow-like/flow-like-ui/components/interfaces/chat-default/respond-interaction";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "@flow-like/flow-like-ui/components/ui/dialog";
import type { IInteractionRequest } from "@flow-like/flow-like-ui/lib/schema/interaction";
import { LoaderCircle } from "lucide-react";
import { useState } from "react";
import type { NativeEventOutput } from "../lib/native-event-execution";

export interface NativeEventRunView {
	id: string;
	scope: string;
	appId: string;
	eventId: string;
	title: string;
	input?: string;
	status: "running" | "complete" | "error";
	output: NativeEventOutput;
	interactions: IInteractionRequest[];
	error?: string;
}

export function NativeEventResultDialog({
	run,
	isCurrent,
	onClose,
	onOpen,
}: {
	run: NativeEventRunView;
	isCurrent: () => boolean;
	onClose: () => void;
	onOpen: () => void;
}) {
	const [answered, setAnswered] = useState<Set<string>>(new Set());
	const [replyError, setReplyError] = useState<string>();
	const respond = async (id: string, value: unknown) => {
		const interaction = run.interactions.find((item) => item.id === id);
		if (!interaction || !isCurrent()) return;
		try {
			await submitInteractionResponse(interaction, value);
			if (isCurrent()) setAnswered((ids) => new Set([...ids, id]));
		} catch (error) {
			if (isCurrent())
				setReplyError(
					error instanceof Error
						? error.message
						: "Could not send your answer.",
				);
		}
	};
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>{run.title}</DialogTitle>
					<DialogDescription>
						{run.status === "running"
							? "Running from Siri or Shortcuts"
							: run.status === "complete"
								? "Event response"
								: "The Event could not complete"}
					</DialogDescription>
				</DialogHeader>
				{run.input && (
					<p className="rounded-lg bg-muted p-3 text-sm whitespace-pre-wrap break-words">
						{run.input}
					</p>
				)}
				{run.status === "running" && (
					<output className="flex items-center gap-2 text-sm text-muted-foreground">
						<LoaderCircle aria-hidden className="size-4 animate-spin" />
						Waiting for the Event
					</output>
				)}
				{run.output.text && (
					<div className="whitespace-pre-wrap break-words text-sm leading-relaxed">
						{run.output.text}
					</div>
				)}
				{run.output.json !== undefined && (
					<details open={!run.output.text} className="text-sm">
						<summary className="cursor-pointer font-medium">
							Structured output
						</summary>
						<pre className="mt-2 max-h-80 overflow-auto rounded-lg bg-muted p-3 text-xs whitespace-pre-wrap break-words">
							{run.output.json}
						</pre>
					</details>
				)}
				{run.status === "complete" &&
					!run.output.text &&
					run.output.json === undefined && (
						<p className="text-sm text-muted-foreground">
							The Event finished without returning an output.
						</p>
					)}
				{run.interactions
					.filter((item) => !answered.has(item.id))
					.map((interaction) => (
						<Interaction
							key={interaction.id}
							interaction={interaction}
							onRespond={respond}
							forceExpanded
						/>
					))}
				{(run.error || replyError) && (
					<p role="alert" className="text-sm text-destructive">
						{run.error || replyError}
					</p>
				)}
				<div className="flex justify-end gap-2">
					<Button variant="outline" onClick={onOpen}>
						Open app
					</Button>
					<Button onClick={onClose}>Close</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
