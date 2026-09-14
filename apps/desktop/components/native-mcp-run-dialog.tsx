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
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import { useState } from "react";
import {
	executeNativeMcpOperation,
	type NativeMcpTool,
} from "../lib/native-integration";

export interface NativeMcpRequest {
	id: string;
	scope: string;
	appId: string;
	eventId: string;
	title: string;
	tool: NativeMcpTool;
	input?: string;
}

export function NativeMcpRunDialog({
	request,
	isCurrent,
	onClose,
}: {
	request: NativeMcpRequest;
	isCurrent: () => boolean;
	onClose: () => void;
}) {
	const backend = useBackend();
	const [args, setArgs] = useState(request.input || "{}");
	const [running, setRunning] = useState(false);
	const [result, setResult] = useState<unknown>();
	const [error, setError] = useState<string>();
	const run = async () => {
		if (running || result !== undefined) return;
		setRunning(true);
		setError(undefined);
		try {
			const value = await executeNativeMcpOperation(
				backend,
				request.appId,
				request.eventId,
				request.tool.name,
				JSON.parse(args),
				isCurrent,
			);
			if (isCurrent()) setResult(value);
		} catch (error) {
			if (isCurrent())
				setError(
					error instanceof Error ? error.message : "The tool could not run.",
				);
		} finally {
			setRunning(false);
		}
	};
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[85vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle>
						{request.title}: {request.tool.name}
					</DialogTitle>
					<DialogDescription>
						{request.tool.description ||
							"Enter the arguments for this tool, then run it with your current account."}
					</DialogDescription>
				</DialogHeader>
				{request.tool.inputSchema && (
					<details className="text-xs">
						<summary className="cursor-pointer">Tool input schema</summary>
						<pre className="mt-2 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3">
							{JSON.stringify(request.tool.inputSchema, null, 2)}
						</pre>
					</details>
				)}
				<label className="space-y-2 text-sm">
					Arguments (JSON object)
					<Textarea
						className="min-h-36 font-mono"
						value={args}
						disabled={running || result !== undefined}
						onChange={(event) => setArgs(event.target.value)}
					/>
				</label>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				{result !== undefined && (
					<div className="space-y-2">
						<p className="text-sm font-medium">Tool result</p>
						<pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">
							{JSON.stringify(result, null, 2)}
						</pre>
					</div>
				)}
				<div className="flex justify-end gap-2">
					<Button variant="outline" onClick={onClose}>
						{running ? "Hide" : "Close"}
					</Button>
					<Button
						disabled={running || result !== undefined}
						onClick={() => void run()}
					>
						{running ? "Running…" : "Run tool"}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
