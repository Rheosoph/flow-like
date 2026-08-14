"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Badge,
	Button,
	type IBoard,
	type IVariable,
	Progress,
	RuntimeVariableEditor,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
	seedRuntimeVariable,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useLiveQuery } from "dexie-react-hooks";
import {
	CheckCircle2Icon,
	ChevronDownIcon,
	CircleDotIcon,
	KeyRoundIcon,
	SaveIcon,
	ShieldCheckIcon,
	Trash2Icon,
	VariableIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	type IRuntimeVariableValue,
	deleteRuntimeVar,
	runtimeVarsDB,
	setRuntimeVar,
} from "../../../../lib/runtime-vars-db";

export default function RuntimeVariablesPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");

	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[id ?? ""],
		typeof id === "string",
	);

	const runtimeVars = useLiveQuery(
		() =>
			runtimeVarsDB.values
				.where("appId")
				.equals(id ?? "")
				.toArray(),
		[id ?? ""],
		[],
	);

	const runtimeConfiguredBoards = useMemo(() => {
		return (boards.data ?? [])
			.map((board) => ({
				board,
				variables: Object.values(board.variables)
					.filter((variable) => variable.runtime_configured || variable.secret)
					.sort((a, b) => a.name.localeCompare(b.name)),
			}))
			.filter(({ variables }) => variables.length > 0)
			.sort((a, b) => a.board.name.localeCompare(b.board.name));
	}, [boards.data]);

	const runtimeVarsMap = useMemo(() => {
		const map = new Map<string, IRuntimeVariableValue>();
		for (const rv of runtimeVars ?? []) {
			map.set(rv.variableId, rv);
		}
		return map;
	}, [runtimeVars]);

	const totalVariables = runtimeConfiguredBoards.reduce(
		(sum, { variables }) => sum + variables.length,
		0,
	);
	const configuredCount = runtimeVars?.length ?? 0;
	const progressPercent =
		totalVariables > 0 ? (configuredCount / totalVariables) * 100 : 100;
	const isComplete = configuredCount === totalVariables && totalVariables > 0;

	if (runtimeConfiguredBoards.length === 0) {
		return (
			<main className="flex flex-col items-center justify-center w-full flex-1 p-8">
				<div className="flex flex-col items-center gap-6 max-w-md text-center">
					<div className="w-20 h-20 rounded-2xl bg-linear-to-br from-emerald-500/20 to-green-500/20 flex items-center justify-center">
						<ShieldCheckIcon className="w-10 h-10 text-emerald-500" />
					</div>
					<div className="space-y-2">
						<h2 className="text-2xl font-semibold">{t('allSet', 'All Set!')}</h2>
						<p className="text-muted-foreground">
							{t('thisAppDoesnapostRequireAnyRuntimeVariablesOrSecrets', "This app doesn't require any runtime variables or secrets.")}
						</p>
					</div>
					<div className="p-4 rounded-lg bg-muted/50 text-sm text-muted-foreground"><Trans i18nKey="strongtipstrongMarkVariablesAsQuotruntimeConfiguredquotOrQuotsecretquotInTheFlowEditorToManageThemHere"><strong>Tip:</strong> Mark variables as &quot;Runtime
						Configured&quot; or &quot;Secret&quot; in the Flow Editor to manage
						them here.</Trans></div>
				</div>
			</main>
		);
	}

	return (
		<main className="flex flex-col w-full flex-1 max-h-full overflow-y-auto md:overflow-visible gap-8 pb-8">
			{/* Header */}
			<header className="sticky top-0 z-10 bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/60 border-b">
				<div className="py-6 space-y-4">
					<div className="flex items-start justify-between gap-4">
						<div className="space-y-1">
							<h1 className="text-2xl font-semibold tracking-tight">
								{t('runtimeVariables', 'Runtime Variables')}
							</h1>
							<p className="text-sm text-muted-foreground">
								{t('setSecretsAndUserspecificValuesThatYourFlowsNeedAtRuntime', "Set secrets and user-specific values that your flows need at runtime")}
							</p>
						</div>
						<StatusBadge
							configured={configuredCount}
							total={totalVariables}
							isComplete={isComplete}
						/>
					</div>
					<div className="space-y-2">
						<Progress value={progressPercent} className="h-2" />
						<p className="text-xs text-muted-foreground">{t('configuredcountOfTotalvariablesConfigured', '{{configuredCount}} of {{totalVariables}} configured', { configuredCount, totalVariables })}</p>
					</div>
				</div>
			</header>

			{/* Board List */}
			<div className="space-y-4">
				{id &&
					runtimeConfiguredBoards.map(({ board, variables }) => (
						<BoardSection
							key={board.id}
							appId={id}
							board={board}
							variables={variables}
							runtimeVarsMap={runtimeVarsMap}
						/>
					))}
			</div>

			{/* Security Notice */}
			<footer className="mt-auto p-4 rounded-xl border bg-muted/30 flex items-start gap-3">
				<ShieldCheckIcon className="w-5 h-5 text-muted-foreground shrink-0 mt-0.5" />
				<div className="space-y-1">
					<p className="text-sm font-medium">{t('securityNotice', 'Security Notice')}</p>
					<p className="text-xs text-muted-foreground">
						{t('runtimeVariablesAreStoredLocallyOnYourDeviceAndAreNeverUploadedToTheServerForRemoteExecutionOnlyNonsecretRuntimeVariablesWillBeSent', "Runtime variables are stored locally on your device and are never uploaded to the server. For remote execution, only non-secret runtime variables will be sent.")}
					</p>
				</div>
			</footer>
		</main>
	);
}

function StatusBadge({
	configured,
	total,
	isComplete,
}: { configured: number; total: number; isComplete: boolean }) {
	const { t } = useTranslation("common");
	if (isComplete) {
		return (
			<Badge className="gap-1.5 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20 hover:bg-emerald-500/20">
				<CheckCircle2Icon className="w-3.5 h-3.5" />
				{t('allConfigured', 'All Configured')}
			</Badge>
		);
	}
	return (
		<Badge variant="secondary" className="gap-1.5">
			<CircleDotIcon className="w-3.5 h-3.5" />{`${configured}/${total}`}</Badge>
	);
}

function BoardSection({
	appId,
	board,
	variables,
	runtimeVarsMap,
}: Readonly<{
	appId: string;
	board: IBoard;
	variables: IVariable[];
	runtimeVarsMap: Map<string, IRuntimeVariableValue>;
}>) {
	const { t } = useTranslation("common");
	const [isOpen, setIsOpen] = useState(true);
	const configuredCount = variables.filter((v) =>
		runtimeVarsMap.has(v.id),
	).length;
	const isComplete = configuredCount === variables.length;

	return (
		<div className="rounded-xl border bg-card overflow-hidden">
			{/* Board Header */}
			<button
				type="button"
				onClick={() => setIsOpen(!isOpen)}
				className="w-full flex items-center justify-between p-4 hover:bg-muted/50 transition-colors"
			>
				<div className="flex items-center gap-3">
					<div
						className={cn(
							"w-10 h-10 rounded-lg flex items-center justify-center",
							isComplete ? "bg-emerald-500/10" : "bg-primary/10",
						)}
					>
						{isComplete ? (
							<CheckCircle2Icon className="w-5 h-5 text-emerald-500" />
						) : (
							<VariableIcon className="w-5 h-5 text-primary" />
						)}
					</div>
					<div className="text-left">
						<h3 className="font-medium">{board.name}</h3>
						<p className="text-sm text-muted-foreground">{t('configuredcountOfLengthConfigured', '{{configuredCount}} of {{length}} configured', { configuredCount, length: variables.length })}</p>
					</div>
				</div>
				<div className="flex items-center gap-3">
					{isComplete && (
						<Badge className="bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20">
							{t('complete', 'Complete')}
						</Badge>
					)}
					<ChevronDownIcon
						className={cn(
							"w-5 h-5 text-muted-foreground transition-transform",
							!isOpen && "-rotate-90",
						)}
					/>
				</div>
			</button>

			{/* Variables List */}
			{isOpen && (
				<div className="border-t divide-y">
					{variables.map((variable) => (
						<VariableRow
							key={variable.id}
							appId={appId}
							boardId={board.id}
							variable={variable}
							savedValue={runtimeVarsMap.get(variable.id)}
							refs={board.refs}
						/>
					))}
				</div>
			)}
		</div>
	);
}

function bytesEqual(a?: number[] | null, b?: number[] | null): boolean {
	if (a === b) return true;
	const aLen = a?.length ?? 0;
	const bLen = b?.length ?? 0;
	if (aLen !== bLen) return false;
	for (let i = 0; i < aLen; i++) {
		if (a?.[i] !== b?.[i]) return false;
	}
	return true;
}

function VariableRow({
	appId,
	boardId,
	variable,
	savedValue,
	refs,
}: Readonly<{
	appId: string;
	boardId: string;
	variable: IVariable;
	savedValue?: IRuntimeVariableValue;
	refs?: Record<string, string>;
}>) {
	const { t } = useTranslation("common");
	const resolvedBytes = useMemo(
		() => seedRuntimeVariable(variable, savedValue?.value).default_value,
		[variable, savedValue?.value],
	);
	const [state, setState] = useState<IVariable>(() =>
		seedRuntimeVariable(variable, savedValue?.value),
	);
	const [isSaving, setIsSaving] = useState(false);
	const [hasChanges, setHasChanges] = useState(false);
	// Bumped to re-seed the editor when the value changes from outside (Dexie).
	const [editorKey, setEditorKey] = useState(0);

	// Reconcile with Dexie. While a local edit is pending (hasChanges), the write
	// has not necessarily propagated through useLiveQuery yet, so we hold the
	// local value and only clear the dirty flag once the persisted value catches
	// up — this avoids clobbering the just-saved value with a stale prop or
	// double-remounting the editor. With no pending edit, reseed from Dexie.
	useEffect(() => {
		if (hasChanges) {
			if (bytesEqual(state.default_value, resolvedBytes)) setHasChanges(false);
			return;
		}
		if (bytesEqual(state.default_value, resolvedBytes)) return;
		setState((prev) => ({ ...prev, default_value: resolvedBytes }));
		setEditorKey((key) => key + 1);
	}, [hasChanges, resolvedBytes, state.default_value]);

	const handleUpdate = useCallback(async (next: IVariable) => {
		setState(next);
		setHasChanges(true);
	}, []);

	const handleSave = useCallback(async () => {
		const bytes = state.default_value;
		if (!bytes || bytes.length === 0) return;
		setIsSaving(true);
		try {
			await setRuntimeVar(
				appId,
				boardId,
				variable.id,
				variable.name,
				bytes,
				variable.secret,
			);
			// Keep hasChanges set; the reconcile effect clears it once the saved
			// value propagates back through useLiveQuery.
		} finally {
			setIsSaving(false);
		}
	}, [appId, boardId, variable, state.default_value]);

	const handleDelete = useCallback(async () => {
		await deleteRuntimeVar(appId, variable.id);
		// Let the reconcile effect reseed to the board default once the deletion
		// propagates, rather than eagerly mutating against a stale savedValue.
	}, [appId, variable.id]);

	const isSecret = variable.secret;
	const isConfigured = !!savedValue;
	const hasValue = !!state.default_value && state.default_value.length > 0;

	return (
		<div className="flex flex-col gap-3 p-4 hover:bg-muted/30 transition-colors">
			<div className="flex items-start justify-between gap-4">
				{/* Variable Info */}
				<div className="flex items-start gap-3 min-w-0">
					<div
						className={cn(
							"w-2 h-2 mt-1.5 rounded-full shrink-0",
							isConfigured ? "bg-emerald-500" : "bg-muted-foreground/30",
						)}
					/>
					<div className="min-w-0 space-y-1">
						<div className="flex items-center gap-2">
							<span className="font-medium truncate">{variable.name}</span>
							{isSecret ? (
								<Tooltip>
									<TooltipTrigger>
										<Badge
											variant="secondary"
											className="gap-1 text-xs shrink-0"
										>
											<KeyRoundIcon className="w-3 h-3" />
											{t('secret', 'Secret')}
										</Badge>
									</TooltipTrigger>
									<TooltipContent>
										{t('thisValueIsEncryptedAndNeverSentToRemoteServers', 'This value is encrypted and never sent to remote servers')}
									</TooltipContent>
								</Tooltip>
							) : (
								<Badge variant="outline" className="text-xs shrink-0">
									{t('runtime', 'Runtime')}
								</Badge>
							)}
						</div>
						{variable.description && (
							<p className="text-xs text-muted-foreground">
								{variable.description}
							</p>
						)}
					</div>
				</div>

				{/* Actions */}
				<div className="flex items-center gap-2 shrink-0">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={hasChanges ? "default" : "ghost"}
								size="icon"
								className="h-9 w-9"
								onClick={handleSave}
								disabled={isSaving || !hasValue}
							>
								<SaveIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t('save', 'Save')}</TooltipContent>
					</Tooltip>

					{isConfigured && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-9 w-9 text-destructive hover:text-destructive hover:bg-destructive/10"
									onClick={handleDelete}
								>
									<Trash2Icon className="w-4 h-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t('delete', 'Delete')}</TooltipContent>
						</Tooltip>
					)}
				</div>
			</div>

			{/* Type-correct editor (path picker, date picker, number field, …) */}
			<div className="pl-5">
				<RuntimeVariableEditor
					key={editorKey}
					variable={state}
					updateVariable={handleUpdate}
					refs={refs}
				/>
			</div>
		</div>
	);
}
