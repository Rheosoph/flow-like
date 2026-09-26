import { ChevronDown } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { SelectTrigger } from "../../../components/ui/select";
import { cn } from "../../../lib/utils";

export const PIN_TEXT_CLASS = "text-[0.6rem] leading-3 font-normal";
export const PIN_CHEVRON_CLASS =
	"size-2 min-w-2 min-h-2 shrink-0 text-card-foreground";
export const PIN_TRIGGER_CLASS = cn(
	"w-fit! max-w-full! min-w-0 p-0 border-0 bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden",
	PIN_TEXT_CLASS,
);

const stopEvent = (event: { stopPropagation: () => void }) =>
	event.stopPropagation();

export function PinLabel({
	text,
	align,
	className,
}: Readonly<{ text: string; align?: "start" | "end"; className?: string }>) {
	return (
		<span
			title={text}
			className={cn(
				"block min-w-0 max-w-full truncate",
				PIN_TEXT_CLASS,
				align === "start" && "text-start ml-1",
				align === "end" && "text-end mr-1",
				className,
			)}
		>
			{text}
		</span>
	);
}

export function PinEditorRow({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<div
			className={cn(
				"flex flex-row items-center justify-start min-w-0 max-w-full ml-1 overflow-hidden",
				className,
			)}
			onMouseDown={stopEvent}
			onPointerDown={stopEvent}
		>
			{children}
		</div>
	);
}

export function PinSelectTrigger({
	label,
	className,
	...props
}: Readonly<
	{ label: string } & Omit<
		ComponentProps<typeof SelectTrigger>,
		"children" | "size" | "noChevron"
	>
>) {
	return (
		<SelectTrigger
			noChevron
			size="sm"
			title={label}
			className={cn(PIN_TRIGGER_CLASS, className)}
			{...props}
		>
			<PinLabel text={label} />
			<ChevronDown className={PIN_CHEVRON_CLASS} />
		</SelectTrigger>
	);
}
