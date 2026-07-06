"use client";

import { useRouter } from "next/navigation";
import { useCallback, useRef, useState } from "react";
import { useGlobalChatStore } from "../../../lib/global-chat-store";

export const HERO_SUGGESTIONS = [
	"Create a new app",
	"What can I build with Flow-Like?",
	"Show me the package store",
	"Switch my profile",
];

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
