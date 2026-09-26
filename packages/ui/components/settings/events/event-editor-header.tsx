"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	ChevronLeftIcon,
	Loader2Icon,
	SaveIcon,
	TriangleAlertIcon,
	Undo2Icon,
} from "lucide-react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";

const SAVE_SHORTCUT =
	typeof navigator !== "undefined" && navigator.platform.includes("Mac")
		? "⌘S"
		: "Ctrl+S";

/**
 * The event editor's header: the way back plus the save controls.
 *
 * It is sticky at the top of the config scroll container on every breakpoint,
 * so Save stays in view however long a section is. The previous bar only
 * appeared below the content, where people scrolled past it or never reached
 * it, and left without saving.
 */
export function EventEditorHeader({
	eventName,
	canWrite,
	isDirty,
	isSaving,
	error,
	onBack,
	onSave,
	onDiscard,
}: Readonly<{
	eventName: string;
	canWrite: boolean;
	isDirty: boolean;
	isSaving: boolean;
	error?: string | null;
	onBack?: () => void;
	onSave: () => void;
	onDiscard: () => void;
}>) {
	const { t } = useTranslation("settings");
	const pending = canWrite && (isDirty || isSaving);

	return (
		<div
			className={cn(
				"z-30 -mx-3 px-3 py-3 md:mx-0 md:px-0",
				pending && "sticky top-0",
			)}
		>
			<div
				className={cn("rounded-xl", pending && "bg-background shadow-floating")}
			>
				<div
					className={cn(
						"flex min-h-12 flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border px-3 py-2 transition-colors",
						error
							? "border-destructive/50 bg-destructive/10"
							: pending
								? "border-primary/50 bg-primary/10"
								: "border-transparent",
					)}
				>
					<nav className="flex min-w-0 flex-1 basis-48 items-center gap-1.5 text-sm text-muted-foreground">
						<Button
							variant="ghost"
							size="sm"
							onClick={onBack}
							className="h-8 shrink-0 gap-1 px-2 font-normal hover:text-foreground"
						>
							<ChevronLeftIcon className="h-4 w-4" />
							{t("events", "Events")}
						</Button>
						<span className="shrink-0">/</span>
						<span className="truncate font-medium text-foreground">
							{eventName}
						</span>
					</nav>

					{canWrite && (
						<div className="ml-auto flex shrink-0 items-center gap-2">
							<SaveStatus isDirty={isDirty} isSaving={isSaving} error={error} />
							{pending && (
								<>
									<Button
										variant="outline"
										size="sm"
										onClick={onDiscard}
										disabled={isSaving}
										aria-label={t("discardChanges", "Discard changes")}
										className="h-9"
									>
										<Undo2Icon className="h-4 w-4" />
										<span className="hidden sm:inline">
											{t("discard", "Discard")}
										</span>
									</Button>
									<Button
										size="sm"
										onClick={onSave}
										disabled={!isDirty || isSaving}
										title={`${t("saveChanges", "Save Changes")} (${SAVE_SHORTCUT})`}
										className="h-9"
									>
										{isSaving ? (
											<Loader2Icon className="h-4 w-4 animate-spin" />
										) : (
											<SaveIcon className="h-4 w-4" />
										)}
										{isSaving ? (
											t("saving", "Saving…")
										) : (
											<>
												<span className="sm:hidden">{t("save", "Save")}</span>
												<span className="hidden sm:inline">
													{t("saveChanges", "Save Changes")}
												</span>
											</>
										)}
										<kbd className="ml-1 hidden rounded border border-primary-foreground/30 px-1 font-sans text-[10px] leading-4 opacity-80 lg:inline">
											{SAVE_SHORTCUT}
										</kbd>
									</Button>
								</>
							)}
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

function SaveStatus({
	isDirty,
	isSaving,
	error,
}: Readonly<{ isDirty: boolean; isSaving: boolean; error?: string | null }>) {
	const { t } = useTranslation("settings");
	if (error)
		return (
			<span className="flex min-w-0 items-center gap-1.5 text-sm font-medium text-destructive">
				<TriangleAlertIcon className="h-4 w-4 shrink-0" />
				<span className="truncate">{error}</span>
			</span>
		);
	if (isSaving) return null;
	if (isDirty)
		return (
			<span className="flex items-center gap-2 text-sm font-medium">
				<span className="h-2 w-2 shrink-0 rounded-full bg-primary" />
				<span>{t("unsavedChanges", "Unsaved changes")}</span>
				<span className="hidden text-xs font-normal text-muted-foreground xl:inline">
					{t(
						"yourEditsAreNotLiveUntilYouSave",
						"Your edits are not live until you save.",
					)}
				</span>
			</span>
		);
	return (
		<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
			<CheckIcon className="h-3.5 w-3.5" />
			{t("allChangesSaved", "All changes saved")}
		</span>
	);
}
