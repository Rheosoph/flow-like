import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useRef, useState } from "react";
import { Input } from "../../../components/ui/input";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { cn } from "../../../lib/utils";

const MIN_ROWS = 1;
const MAX_ROWS = 15;
const LINE_HEIGHT = 22; // px

export function StringVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [isFocused, setIsFocused] = useState(false);
	const value = parseUint8ArrayToJson(variable.default_value);

	const adjustHeight = useCallback(() => {
		const textarea = textareaRef.current;
		if (!textarea) return;

		textarea.style.height = "auto";
		const scrollHeight = textarea.scrollHeight;
		const minHeight = MIN_ROWS * LINE_HEIGHT;
		const maxHeight = MAX_ROWS * LINE_HEIGHT;
		const newHeight = Math.min(Math.max(scrollHeight, minHeight), maxHeight);
		textarea.style.height = `${newHeight}px`;
	}, []);

	useEffect(() => {
		adjustHeight();
	}, [value, adjustHeight]);

	const handleChange = useCallback(
		(newValue: string) => {
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(newValue),
			});
		},
		[onChange, variable],
	);

	if (variable.secret) {
		return (
			<div className="grid w-full items-center gap-1.5">
				<Input
					autoComplete="off"
					spellCheck="false"
					autoCorrect="off"
					autoCapitalize="off"
					disabled={disabled}
					value={value}
					onChange={(e) => handleChange(e.target.value)}
					type="password"
					id="default_value"
					placeholder={t('enterSecretValue', 'Enter secret value...')}
					className="font-mono"
				/>
			</div>
		);
	}

	return (
		<div className="grid w-full items-center gap-1.5">
			<div
				className={cn(
					"relative w-full rounded-md border bg-transparent transition-all duration-200",
					"border-input dark:bg-input/30",
					isFocused && "border-ring ring-ring/50 ring-[3px]",
					disabled && "opacity-50 cursor-not-allowed",
				)}
			>
				<textarea
					ref={textareaRef}
					disabled={disabled}
					value={value}
					onChange={(e) => handleChange(e.target.value)}
					onFocus={() => setIsFocused(true)}
					onBlur={() => setIsFocused(false)}
					placeholder={t('enterText', 'Enter text...')}
					autoComplete="off"
					spellCheck="false"
					autoCorrect="off"
					autoCapitalize="off"
					rows={MIN_ROWS}
					className={cn(
						"w-full resize-none bg-transparent px-3 py-2 text-sm outline-none",
						"font-mono leading-[22px]",
						"placeholder:text-muted-foreground",
						"selection:bg-primary selection:text-primary-foreground",
						"disabled:pointer-events-none",
						"scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted-foreground/30 hover:scrollbar-thumb-muted-foreground/50",
					)}
					style={{
						minHeight: `${MIN_ROWS * LINE_HEIGHT}px`,
						maxHeight: `${MAX_ROWS * LINE_HEIGHT}px`,
						caretColor: "hsl(var(--primary))",
					}}
				/>

				{/* Character count */}
				{(value?.length ?? 0) > 0 && (
					<div className="absolute bottom-1 right-2 text-[10px] text-muted-foreground/60 font-mono select-none pointer-events-none">{t('countCharacters', { defaultValue_one: '{{count}} character', defaultValue_other: '{{count}} characters', count: value.length })}{value.includes("\n") && <> · {t('countLines', { defaultValue_one: '{{count}} line', defaultValue_other: '{{count}} lines', count: value.split("\n").length })}</>}
					</div>
				)}
			</div>
		</div>
	);
}
