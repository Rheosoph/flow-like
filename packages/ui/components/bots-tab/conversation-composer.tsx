"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpIcon } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { Button } from "../ui/button";

const MAX_HEIGHT = 160;

function fitHeight(input: HTMLTextAreaElement | null, value: string) {
	if (!input) return;
	input.style.height = "auto";
	if (value) {
		input.style.height = `${Math.min(input.scrollHeight, MAX_HEIGHT)}px`;
	}
}

export function ConversationComposer({
	recipient,
	hint,
	disabledReason,
	onSend,
}: Readonly<{
	recipient: string;
	hint?: string | null;
	/** Shown instead of a hint while writing is not possible. */
	disabledReason?: string | null;
	onSend: (text: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const [value, setValue] = useState("");
	const inputRef = useRef<HTMLTextAreaElement>(null);
	const disabled = Boolean(disabledReason);
	const placeholder = t("crew.composer.placeholder", "Message {{name}}", {
		name: recipient,
	});

	const submit = useCallback(() => {
		const text = value.trim();
		if (!text || disabled) return;
		onSend(text);
		setValue("");
		fitHeight(inputRef.current, "");
	}, [disabled, onSend, value]);

	return (
		<form
			className="shrink-0 border-t px-3.5 pt-2.5 pb-3.5"
			onSubmit={(event) => {
				event.preventDefault();
				submit();
			}}
		>
			<p className="mx-1 mb-2 truncate text-xs text-muted-foreground">
				{disabledReason ??
					hint ??
					t("crew.composer.to", "To {{name}}", { name: recipient })}
			</p>
			<div className="flex items-end gap-1.5 rounded-[14px] border bg-card p-1.5 focus-within:border-primary">
				<textarea
					ref={inputRef}
					rows={1}
					value={value}
					disabled={disabled}
					onChange={(event) => {
						setValue(event.target.value);
						fitHeight(event.target, event.target.value);
					}}
					onKeyDown={(event) => {
						if (
							event.key === "Enter" &&
							!event.shiftKey &&
							!event.nativeEvent.isComposing
						) {
							event.preventDefault();
							submit();
						}
					}}
					aria-label={placeholder}
					placeholder={placeholder}
					className="min-h-8.5 flex-1 resize-none bg-transparent px-2 py-1.5 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
				/>
				<Button
					type="submit"
					size="icon"
					className="size-8.5 shrink-0 rounded-[10px]"
					disabled={disabled || !value.trim()}
					aria-label={t("crew.composer.send", "Send")}
					title={t("crew.composer.send", "Send")}
				>
					<ArrowUpIcon aria-hidden />
				</Button>
			</div>
		</form>
	);
}
