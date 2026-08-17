"use client";

import { useTranslation } from "@flow-like/locales";
import { useRouter } from "next/navigation";
import { useCallback, useMemo, useRef, useState } from "react";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";

/** English source order; `useHeroSuggestions` renders the localized labels. */
export const HERO_SUGGESTIONS = [
	"Create a new app",
	"What can I build with Flow-Like?",
	"Show me the package store",
	"Switch my profile",
];

/**
 * The label is also the prompt that gets sent, so it has to be the localized
 * string — FlowPilot answers in the language it was asked in.
 */
export function useHeroSuggestions(): string[] {
	const { t } = useTranslation("chat");
	return useMemo(
		() => [
			t("createANewApp", "Create a new app"),
			t("whatCanIBuildWithFlowlike", "What can I build with Flow-Like?"),
			t("showMeThePackageStore", "Show me the package store"),
			t("switchMyProfile", "Switch my profile"),
		],
		[t],
	);
}

export function useHeroComposer() {
	const router = useRouter();
	const setDraft = useGlobalChatStore((s) => s.setDraft);
	const [value, setValue] = useState("");
	const [files, setFiles] = useState<File[]>([]);
	const fileInputRef = useRef<HTMLInputElement>(null);

	const submit = useCallback(
		(prompt: string) => {
			const trimmed = prompt.trim();
			if (!trimmed && files.length === 0) return;
			setDraft({
				prompt: trimmed,
				files: files.length > 0 ? files : undefined,
			});
			router.push("/chat");
		},
		[files, router, setDraft],
	);

	const addFiles = useCallback((selected: File[]) => {
		if (selected.length > 0) setFiles((prev) => [...prev, ...selected]);
	}, []);

	const removeFile = useCallback((index: number) => {
		setFiles((prev) => prev.filter((_, i) => i !== index));
	}, []);

	const openFilePicker = useCallback(() => {
		fileInputRef.current?.click();
	}, []);

	const canSend = value.trim().length > 0 || files.length > 0;

	return {
		value,
		setValue,
		files,
		addFiles,
		removeFile,
		openFilePicker,
		fileInputRef,
		submit,
		canSend,
	};
}
