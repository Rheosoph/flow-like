"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeft, type LucideIcon, X } from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	useCallback,
	useMemo,
	useState,
} from "react";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../../../ui/dialog";
import { DvButton } from "./dv-button";
import { cx } from "./tone";

export interface DvSheetProps {
	open: boolean;
	onOpenChange(open: boolean): void;
	title: ReactNode;
	sub?: ReactNode;
	icon?: LucideIcon;
	/** Small label above the title ("Before this runs"). */
	eyebrow?: ReactNode;
	/** 780 px instead of 620 px. */
	wide?: boolean;
	/** Foot buttons, right-aligned (Cancel first, the one primary last). */
	foot?: ReactNode;
	/** Foot text on the left ("Step 1 of 2 · Review"). */
	footNote?: ReactNode;
	/** Shows Back in the head: sheets never stack, they replace their body (SPEC §4.21). */
	onBack?: () => void;
	/** `alertdialog` for confirms. */
	role?: "dialog" | "alertdialog";
	/** Outside clicks close the sheet (Esc always does). Off for confirms. */
	closeOnOutside?: boolean;
	/** Takes the focus when the sheet opens (the field to type in); the first control otherwise. */
	initialFocus?: RefObject<HTMLElement | null>;
	className?: string;
	bodyClassName?: string;
	children?: ReactNode;
}

/* As the phone bottom sheet, the last row keeps clear of the home indicator. */
const PHONE_SAFE_FOOT =
	"max-[720px]:pb-[calc(--spacing(3)+var(--fl-safe-bottom,0px))]";
const PHONE_SAFE_BODY =
	"max-[720px]:pb-[calc(--spacing(4)+var(--fl-safe-bottom,0px))]";

/**
 * SPEC §4.21: popover surface, `border-strong`, scrim, no blur or shadow
 * (R13); 620/780 px; bottom sheet below 720 px of viewport (it is portalled,
 * so the viewport is its container); sticky sunken foot.
 */
export function DvSheet({
	open,
	onOpenChange,
	title,
	sub,
	icon: Icon,
	eyebrow,
	wide = false,
	foot,
	footNote,
	onBack,
	role = "dialog",
	closeOnOutside = true,
	initialFocus,
	className,
	bodyClassName,
	children,
}: Readonly<DvSheetProps>) {
	const { t } = useTranslation("devices");
	const hasFoot = Boolean(foot || footNote);
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				role={role}
				showCloseButton={false}
				overlayClassName="bg-scrim backdrop-blur-none"
				onInteractOutside={
					closeOnOutside ? undefined : (event) => event.preventDefault()
				}
				onOpenAutoFocus={
					initialFocus
						? (event) => {
								const target = initialFocus.current;
								if (!target) return;
								event.preventDefault();
								target.focus();
							}
						: undefined
				}
				{...(sub ? {} : { "aria-describedby": undefined })}
				className={cx(
					"gap-0 overflow-hidden rounded-lg border border-border-strong bg-popover p-0 shadow-none backdrop-blur-none [&>[data-slot=dialog-accent]]:hidden",
					"max-h-[calc(100dvh-48px-var(--fl-safe-top,0px)-var(--fl-safe-bottom,0px))] w-[min(620px,calc(100vw-32px))] max-w-[calc(100vw-32px)] sm:max-w-[620px]",
					wide && "w-[min(780px,calc(100vw-32px))] sm:max-w-[780px]",
					"max-[720px]:top-auto max-[720px]:bottom-0 max-[720px]:max-h-[calc(100dvh-24px-var(--fl-safe-top,0px))] max-[720px]:w-full max-[720px]:max-w-full max-[720px]:translate-y-0 max-[720px]:rounded-b-none max-[720px]:sm:max-w-full",
					className,
				)}
			>
				<div className="flex items-start gap-2.5 border-b border-hairline pt-4 pr-4 pb-3 pl-5">
					{onBack ? (
						<DvButton
							variant="ghost"
							size="sm"
							iconOnly
							icon={ArrowLeft}
							aria-label={t("common.sheet.back", "Back")}
							onClick={onBack}
						/>
					) : Icon ? (
						<Icon aria-hidden className="mt-0.5 size-4 shrink-0" />
					) : null}
					<div className="min-w-0 flex-1">
						{eyebrow ? (
							<p className="mb-0.5 text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
								{eyebrow}
							</p>
						) : null}
						<DialogTitle className="text-[15px]/5 font-semibold">
							{title}
						</DialogTitle>
						{sub ? (
							<DialogDescription className="mt-0.5 text-xs">
								{sub}
							</DialogDescription>
						) : null}
					</div>
					<DvButton
						variant="ghost"
						size="sm"
						iconOnly
						icon={X}
						aria-label={t("common.sheet.close", "Close")}
						onClick={() => onOpenChange(false)}
					/>
				</div>
				<div
					className={cx(
						"flex min-h-0 flex-1 flex-col gap-3.5 overflow-auto px-5 py-4 text-sm",
						!hasFoot && PHONE_SAFE_BODY,
						bodyClassName,
					)}
				>
					{children}
				</div>
				{hasFoot ? (
					<div
						className={cx(
							"sticky bottom-0 flex flex-wrap items-center justify-end gap-2 border-t border-hairline bg-surface-sunken px-5 py-3 text-xs text-muted-foreground",
							PHONE_SAFE_FOOT,
						)}
					>
						<span className="min-w-[12ch] flex-1 max-[560px]:basis-full max-[560px]:empty:hidden">
							{footNote}
						</span>
						{foot}
					</div>
				) : null}
			</DialogContent>
		</Dialog>
	);
}

/** One sheet at a time: `push` replaces the body and enables Back; `open` starts a new stack. */
export function useSheetStack<T>() {
	const [stack, setStack] = useState<readonly T[]>([]);
	const open = useCallback((entry: T) => setStack([entry]), []);
	const push = useCallback(
		(entry: T) => setStack((current) => [...current, entry]),
		[],
	);
	const back = useCallback(
		() => setStack((current) => current.slice(0, -1)),
		[],
	);
	const close = useCallback(() => setStack([]), []);
	return useMemo(
		() => ({
			current: stack.at(-1) ?? null,
			depth: stack.length,
			canGoBack: stack.length > 1,
			open,
			push,
			back,
			close,
		}),
		[stack, open, push, back, close],
	);
}
