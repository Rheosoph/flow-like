"use client";

import { Slot, Slottable } from "@radix-ui/react-slot";
import { LoaderCircle, type LucideIcon } from "lucide-react";
import type { ComponentProps, MouseEvent, ReactNode } from "react";
import { buttonVariants } from "../../../ui/button";
import { cx } from "./tone";

export type DvButtonVariant =
	| "primary"
	| "default"
	| "ghost"
	| "danger"
	| "danger-ghost"
	| "link";
export type DvButtonSize = "md" | "sm" | "xs";

/*
 * Spec variants mapped onto the shadcn variants (M-UI §1.2). The overrides
 * remove the coral hovers (R2), the shadows (R13) and the 36 px heights; busy
 * state is explicit because Button's auto-busy wrapper never runs with onClick.
 */
const VARIANT: Record<
	DvButtonVariant,
	{
		base: "default" | "outline" | "ghost" | "destructive" | "link";
		cls: string;
	}
> = {
	primary: {
		base: "default",
		cls: "border border-primary font-semibold hover:bg-primary/90",
	},
	default: {
		base: "outline",
		cls: "border-border bg-card text-foreground hover:border-border-strong hover:bg-row-hover hover:text-foreground dark:border-border dark:bg-card dark:hover:bg-row-hover",
	},
	ghost: {
		base: "ghost",
		cls: "border border-transparent bg-transparent hover:bg-row-hover hover:text-foreground dark:hover:bg-row-hover",
	},
	danger: {
		base: "destructive",
		cls: "border border-danger bg-danger font-semibold text-danger-foreground hover:bg-danger-hover dark:bg-danger dark:hover:bg-danger-hover",
	},
	"danger-ghost": {
		base: "outline",
		cls: "border-critical-line bg-transparent text-critical hover:bg-critical-bg hover:text-critical dark:border-critical-line dark:bg-transparent dark:hover:bg-critical-bg",
	},
	link: {
		base: "link",
		cls: "h-auto border-0 px-0 py-0 font-normal text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-current has-[>svg]:px-0",
	},
};

const SIZE: Record<
	DvButtonSize,
	{ cls: string; icon: string; square: string }
> = {
	md: {
		cls: "h-8 gap-1.5 px-3 py-0 has-[>svg]:px-3",
		icon: "size-4",
		square: "w-8 px-0 has-[>svg]:px-0",
	},
	sm: {
		cls: "h-7 gap-1.5 px-2.5 py-0 has-[>svg]:px-2.5",
		icon: "size-4",
		square: "w-7 px-0 has-[>svg]:px-0",
	},
	xs: {
		cls: "h-6 gap-1 px-2 py-0 text-xs has-[>svg]:px-2",
		icon: "size-3.5",
		square: "w-6 px-0 has-[>svg]:px-0",
	},
};

export interface DvButtonProps
	extends Omit<ComponentProps<"button">, "children"> {
	variant?: DvButtonVariant;
	size?: DvButtonSize;
	icon?: LucideIcon;
	/** Swaps the icon for a spinner, sets `aria-busy` and ignores clicks; the label stays. */
	busy?: boolean;
	/** Square button; pass `aria-label`. */
	iconOnly?: boolean;
	/** Render the single child element (e.g. a link) with button styling. */
	asChild?: boolean;
	children?: ReactNode;
}

export function DvButton({
	variant = "default",
	size = "md",
	icon: Icon,
	busy = false,
	iconOnly = false,
	asChild = false,
	className,
	children,
	onClick,
	type,
	...props
}: DvButtonProps) {
	const v = VARIANT[variant];
	const s = SIZE[size];
	const Comp = asChild ? Slot : "button";
	const glyph = busy ? (
		<LoaderCircle aria-hidden className={cx(s.icon, "animate-spin")} />
	) : Icon ? (
		<Icon aria-hidden className={s.icon} />
	) : null;

	const handleClick = (event: MouseEvent<HTMLButtonElement>) => {
		if (
			busy ||
			props["aria-disabled"] === true ||
			props["aria-disabled"] === "true"
		) {
			event.preventDefault();
			return;
		}
		onClick?.(event);
	};

	return (
		<Comp
			data-slot="dv-button"
			data-variant={variant}
			data-dv-primary={variant === "primary" ? "" : undefined}
			aria-busy={busy || undefined}
			type={asChild ? undefined : (type ?? "button")}
			className={cx(
				buttonVariants({ variant: v.base, size: "default" }),
				"rounded-lg text-ui font-medium shadow-none focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring focus-visible:outline-solid aria-disabled:cursor-not-allowed aria-disabled:opacity-50 aria-busy:cursor-progress",
				s.cls,
				iconOnly && s.square,
				v.cls,
				className,
			)}
			onClick={handleClick}
			{...props}
		>
			{glyph}
			<Slottable>{children}</Slottable>
		</Comp>
	);
}
