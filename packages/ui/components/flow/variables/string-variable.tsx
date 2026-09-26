import { useTranslation } from "@flow-like/locales";
import { EyeIcon, EyeOffIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../../../components/ui/button";
import { Input } from "../../../components/ui/input";
import { Textarea } from "../../../components/ui/textarea";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { cn } from "../../../lib/utils";

const MIN_ROWS = 1;
const MAX_ROWS = 15;
const LINE_HEIGHT = 22; // px

function SecretStringInput({
	value,
	disabled,
	onChange,
}: Readonly<{
	value: string;
	disabled?: boolean;
	onChange: (value: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const [revealed, setRevealed] = useState(false);
	const multiline = /[\r\n]/.test(value);
	const showLabel = revealed
		? t("hideSecretValue", "Hide secret value")
		: t("showSecretValue", "Show secret value");

	return (
		<div className="grid w-full items-center gap-1.5">
			<div className="relative">
				{revealed ? (
					<Textarea
						autoComplete="off"
						spellCheck={false}
						autoCorrect="off"
						autoCapitalize="off"
						disabled={disabled}
						value={value}
						onChange={(event) => onChange(event.target.value)}
						aria-label={t("secretValue", "Secret value")}
						placeholder={t("enterSecretValue", "Enter secret value...")}
						rows={multiline ? 5 : 1}
						className="min-h-9 max-h-[330px] pr-20 font-mono"
					/>
				) : (
					<Input
						disabled={disabled}
						type="password"
						aria-label={t("secretValue", "Secret value")}
						value={multiline ? "••••••••" : value}
						readOnly={multiline}
						onChange={(event) => onChange(event.target.value)}
						onPaste={(event) => {
							const pasted = event.clipboardData.getData("text/plain");
							if (disabled || (!multiline && !/[\r\n]/.test(pasted))) return;
							// Password inputs strip line breaks before onChange fires.
							event.preventDefault();
							const input = event.currentTarget;
							onChange(
								multiline
									? pasted
									: value.slice(0, input.selectionStart ?? value.length) +
											pasted +
											value.slice(input.selectionEnd ?? value.length),
							);
						}}
						placeholder={t("enterSecretValue", "Enter secret value...")}
						className="pr-20 font-mono"
					/>
				)}
				<div className="absolute right-1 top-1 flex gap-1">
					{value.length > 0 && (
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							disabled={disabled}
							aria-label={t("clearValue", "Clear value")}
							onClick={() => onChange("")}
						>
							<XIcon className="h-4 w-4" />
						</Button>
					)}
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						disabled={disabled}
						aria-label={showLabel}
						aria-pressed={revealed}
						onClick={() => setRevealed((current) => !current)}
					>
						{revealed ? (
							<EyeOffIcon className="h-4 w-4" />
						) : (
							<EyeIcon className="h-4 w-4" />
						)}
					</Button>
				</div>
			</div>
			{multiline && !revealed && (
				<p className="text-xs text-muted-foreground">
					{t(
						"multilineSecretHint",
						"Multiline secret. Show to edit or paste to replace.",
					)}
				</p>
			)}
		</div>
	);
}

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

	// biome-ignore lint/correctness/useExhaustiveDependencies: Remeasure the textarea when its content changes.
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
			<SecretStringInput
				disabled={disabled}
				value={typeof value === "string" ? value : ""}
				onChange={handleChange}
			/>
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
					placeholder={t("enterText", "Enter text...")}
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
					<div className="absolute bottom-1 right-2 text-[10px] text-muted-foreground/60 font-mono select-none pointer-events-none">
						{t("countCharacters", {
							defaultValue_one: "{{count}} character",
							defaultValue_other: "{{count}} characters",
							count: value.length,
						})}
						{value.includes("\n") && (
							<>
								{" "}
								·{" "}
								{t("countLines", {
									defaultValue_one: "{{count}} line",
									defaultValue_other: "{{count}} lines",
									count: value.split("\n").length,
								})}
							</>
						)}
					</div>
				)}
			</div>
		</div>
	);
}
