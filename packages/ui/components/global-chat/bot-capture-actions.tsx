"use client";

import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Button } from "../ui/button";

interface Capabilities {
	voice?: boolean;
	document?: boolean;
	photo?: boolean;
	localAiAvailable?: boolean;
	localSummarizationAvailable?: boolean;
	localExtractionAvailable?: boolean;
}

export function BotCaptureActions({ botId }: { botId?: string | null }) {
	const [capabilities, setCapabilities] = useState<Capabilities>({});
	useEffect(() => {
		if (!botId || !isTauri()) return;
		let active = true;
		void invoke<Capabilities>("native_bot_capabilities")
			.then((value) => {
				if (active) setCapabilities(value);
			})
			.catch(() => {});
		return () => {
			active = false;
		};
	}, [botId]);
	if (!botId) return null;
	const items = [
		["voice", "Dictate a message", capabilities.voice],
		["document", "Scan a document", capabilities.document],
		["photo", "Take a photo", capabilities.photo],
		[
			"summarize",
			"Summarize on this device",
			capabilities.localSummarizationAvailable ?? capabilities.localAiAvailable,
		],
		[
			"extract",
			"Extract details on this device",
			capabilities.localExtractionAvailable ?? capabilities.localAiAvailable,
		],
	] as const;
	return (
		<div className="flex flex-col items-start">
			{items
				.filter(([, , available]) => available)
				.map(([operation, label]) => (
					<Button
						key={operation}
						type="button"
						variant="ghost"
						size="sm"
						onClick={() =>
							window.dispatchEvent(
								new CustomEvent("native-bot-input", {
									detail: { botId, operation },
								}),
							)
						}
					>
						{label}
					</Button>
				))}
		</div>
	);
}
