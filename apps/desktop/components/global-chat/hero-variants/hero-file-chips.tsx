"use client";

import { ImageIcon, XIcon } from "lucide-react";
import type { RefObject } from "react";

export function HeroFileChips({
	files,
	onRemove,
}: Readonly<{ files: File[]; onRemove: (index: number) => void }>) {
	if (files.length === 0) return null;
	return (
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
						onClick={() => onRemove(index)}
					>
						<XIcon className="size-3" />
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
