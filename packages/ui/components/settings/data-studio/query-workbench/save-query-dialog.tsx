"use client";

import { useTranslation } from "@flow-like/locales";
import { Lightbulb, Loader2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../../lib/utils";
import type { SavedQueryKind } from "../../../../state/backend-state/query-state";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import { recommendQueryKind } from "../../../ui/sql-editor";
import { Textarea } from "../../../ui/textarea";

export interface SaveQueryDefaults {
	name: string;
	description?: string;
	kind: SavedQueryKind;
}

export function SaveQueryDialog({
	open,
	onOpenChange,
	sql,
	params,
	defaults,
	busy,
	error,
	onConfirm,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	sql: string;
	params: string[];
	defaults?: SaveQueryDefaults;
	busy: boolean;
	error: string | null;
	onConfirm: (payload: {
		name: string;
		description?: string;
		kind: SavedQueryKind;
	}) => void;
}>) {
	const { t } = useTranslation("settings");
	const recommendation = useMemo(
		() => recommendQueryKind(sql, params),
		[sql, params],
	);
	const paramsPresent = params.length > 0;
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [kind, setKind] = useState<SavedQueryKind>("query");

	// Read defaults through a ref so the reset fires only on the open transition.
	// Depending on the `defaults` object identity would re-run on every parent
	// re-render (e.g. the Save click) and wipe the user's in-progress edits.
	const defaultsRef = useRef(defaults);
	defaultsRef.current = defaults;
	useEffect(() => {
		if (!open) return;
		const current = defaultsRef.current;
		setName(current?.name ?? "");
		setDescription(current?.description ?? "");
		setKind(current?.kind ?? recommendation.kind);
	}, [open, recommendation.kind]);

	const effectiveKind: SavedQueryKind = paramsPresent ? "query" : kind;
	const canSave = name.trim().length > 0 && !busy;

	const submit = () => {
		if (!canSave) return;
		onConfirm({
			name: name.trim(),
			description: description.trim() || undefined,
			kind: effectiveKind,
		});
	};

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!busy) onOpenChange(next);
			}}
		>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t('saveQuery', 'Save query')}</DialogTitle>
					<DialogDescription>
						{t('storeThisQueryToRerunItOrSaveItAsAViewOtherQueriesCanReadFrom', "Store this query to rerun it, or save it as a view other queries can read from.")}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-1">
					<div className="grid gap-1.5">
						<Label htmlFor="saved-query-name">Name</Label>
						<Input
							id="saved-query-name"
							value={name}
							disabled={busy}
							autoFocus
							onChange={(event) => setName(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									submit();
								}
							}}
							placeholder={t('activeCustomers', 'Active customers')}
						/>
					</div>
					<div className="grid gap-1.5">
						<Label htmlFor="saved-query-description">{t('description', 'Description')}</Label>
						<Textarea
							id="saved-query-description"
							value={description}
							disabled={busy}
							className="min-h-16"
							onChange={(event) => setDescription(event.target.value)}
							placeholder={t('optionalNotesAboutWhatThisQueryReturns', 'Optional notes about what this query returns')}
						/>
					</div>

					<div className="grid gap-1.5">
						<Label id="save-as-label">{t('saveAs', 'Save as')}</Label>
						<div className="grid grid-cols-2 gap-2">
							<button
								type="button"
								aria-pressed={effectiveKind === "query"}
								disabled={busy}
								onClick={() => setKind("query")}
								className={cn(
									"rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
									effectiveKind === "query"
										? "border-primary bg-primary/10"
										: "hover:bg-muted/50",
								)}
							>
								<p className="text-sm font-medium">{t('storedQuery', 'Stored query')}</p>
								<p className="mt-0.5 text-xs text-muted-foreground">
									{t('rerunItAnyTimeWithParameters', 'Rerun it any time, with parameters.')}
								</p>
							</button>
							<button
								type="button"
								aria-pressed={effectiveKind === "view"}
								aria-describedby={paramsPresent ? "save-as-hint" : undefined}
								disabled={busy || paramsPresent}
								onClick={() => setKind("view")}
								className={cn(
									"rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
									effectiveKind === "view"
										? "border-primary bg-primary/10"
										: "hover:bg-muted/50",
								)}
							>
								<p className="text-sm font-medium">{t('view', 'View')}</p>
								<p className="mt-0.5 text-xs text-muted-foreground">
									{t('aNamedTableOtherQueriesCanFrom', 'A named table other queries can FROM.')}
								</p>
							</button>
						</div>
						<p
							id="save-as-hint"
							className="flex items-center gap-1.5 text-xs text-muted-foreground"
						>
							<Lightbulb className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />
							{paramsPresent
								? t('queriesWithParametersCantBeViewsSavedAsAStoredQuery', 'Queries with parameters can\'t be views — saved as a stored query.')
								: recommendation.reason}
						</p>
					</div>

					{paramsPresent && (
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{t('parameters', 'Parameters')}
							</Label>
							<div className="flex flex-wrap gap-1.5">
								{params.map((param) => (
									<Badge
										key={param}
										variant="secondary"
										className="font-mono text-[11px]"
									>{`$${param}`}</Badge>
								))}
							</div>
						</div>
					)}

					<div className="grid gap-1.5">
						<Label className="text-xs text-muted-foreground">SQL</Label>
						<pre className="max-h-24 overflow-auto rounded-md bg-muted/40 p-2 font-mono text-xs text-muted-foreground">
							{sql.trim()}
						</pre>
					</div>

					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
				</div>
				<DialogFooter>
					<Button
						variant="ghost"
						disabled={busy}
						onClick={() => onOpenChange(false)}
					>
						{t('cancel', 'Cancel')}
					</Button>
					<Button disabled={!canSave} onClick={submit}>
						{busy && <Loader2 className="h-4 w-4 animate-spin" />}
						{t('save', 'Save')}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
