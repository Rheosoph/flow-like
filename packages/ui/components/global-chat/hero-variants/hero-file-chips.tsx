"use client";

import { useTranslation } from "@flow-like/locales";
import { ImageIcon, XIcon } from "lucide-react";
import type { RefObject } from "react";

export function HeroFileChips({
	files,
	onRemove,
}: Readonly<{ files: File[]; onRemove: (index: number) => void }>) {
	const { t } = useTranslation("chat");
	if (files.length === 0) return null;
	return (
		<div className="flex flex-wrap items-center gap-1.5 pb-2">
			{files.map((file, index) => (
				<span
					key={`${file.name}-${index}`}
					className="flex items-center gap-1 rounded-full bg-primary/10 text-primary py-0.5 pl-2 pr-0.5 text-xs"
				>
					<ImageIcon className="size-3 shrink-0" />
					<span className="max-w-32 truncate">{file.name}</span>
					<button
						type="button"
						aria-label={t("removeName", "Remove {{name}}", { name: file.name })}
						className="grid size-6 shrink-0 place-items-center rounded-full hover:text-destructive outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0 extend-touch-target"
						onClick={() => onRemove(index)}
					>
						<XIcon className="size-3.5" />
					</button>
				</span>
			))}
		</div>
	);
}

export function HeroFileInput({
	inputRef,
	onAdd,
}: Readonly<{
	inputRef: RefObject<HTMLInputElement | null>;
	onAdd: (files: File[]) => void;
}>) {
	return (
		<input
			ref={inputRef}
			type="file"
			accept="image/*"
			multiple
			className="hidden"
			onChange={(e) => {
				const selected = Array.from(e.target.files ?? []);
				onAdd(selected);
				e.target.value = "";
			}}
		/>
	);
}
