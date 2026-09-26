"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRightIcon,
	CheckIcon,
	KeyRoundIcon,
	LockIcon,
	SaveIcon,
	Trash2Icon,
	UndoIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../../hooks";
import { upsertVariableCommand } from "../../../lib/command/generic-command";
import {
	isRuntimeVariableConfigured,
	seedRuntimeVariable,
} from "../../../lib/runtime-vars-utils";
import type { IVariable } from "../../../lib/schema/flow/variable";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { useRuntimeVariables } from "../../../state/runtime-variables-context";
import { VariableTypeIndicator } from "../../flow/variables/components";
import { RuntimeVariableEditor } from "../../flow/variables/runtime-variable-editor";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import type { DeviceRequirement } from "./use-app-setup";

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

/**
 * Hold an editable copy of a variable and reconcile it with the value that
 * comes back from its store.
 *
 * While a local edit is pending the persisted value has not necessarily
 * propagated yet, so the local bytes are held and the dirty flag is only
 * cleared once the store catches up. Without that, a just-typed value is
 * clobbered by a stale prop; without the remount key, an editor that seeds
 * itself once never shows a value that changed underneath it.
 */
function usePendingValue(resolved: IVariable) {
	const [state, setState] = useState<IVariable>(resolved);
	const [dirty, setDirty] = useState(false);
	const [editorKey, setEditorKey] = useState(0);

	useEffect(() => {
		if (dirty) {
			if (bytesEqual(state.default_value, resolved.default_value))
				setDirty(false);
			return;
		}
		if (bytesEqual(state.default_value, resolved.default_value)) return;
		setState(resolved);
		setEditorKey((key) => key + 1);
	}, [dirty, resolved, state.default_value]);

	const update = useCallback(async (next: IVariable) => {
		setState(next);
		setDirty(true);
	}, []);

	return { state, dirty, editorKey, update, setDirty };
}

function RowShell({
	tone,
	status,
	first,
	children,
}: Readonly<{
	tone: "device" | "shared";
	status: "need" | "set" | "optional";
	first?: boolean;
	children: React.ReactNode;
}>) {
	return (
		<div
			data-setup-row
			{...(first ? { "data-first-requirement": "" } : {})}
			className={cn(
				"flex flex-col gap-3 p-4 transition-colors hover:bg-muted/30",
				tone === "device"
					? "border-l-[3px] border-l-violet-500/70"
					: "border-l-[3px] border-l-sky-500/70",
				status === "set" && "bg-emerald-500/[0.035]",
			)}
		>
			{children}
		</div>
	);
}

function StatusDot({
	status,
}: Readonly<{ status: "need" | "set" | "optional" }>) {
	return (
		<span
			aria-hidden
			className={cn(
				"mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full",
				status === "need" && "bg-amber-500 ring-4 ring-amber-500/15",
				status === "set" && "bg-emerald-500",
				status === "optional" && "bg-muted-foreground/30",
			)}
		/>
	);
}

/**
 * One thing the app is waiting on from this person.
 *
 * Saves go to the device store and nowhere else. There is deliberately no
 * shared `onSave` between this row and {@link SharedParameterRow}: a single
 * handler that branched on a flag is the one shape of this screen that could
 * publish somebody's credential to the whole team.
 */
export function RequirementRow({
	appId,
	requirement,
	first,
	onWaive,
	onJumpToShared,
}: Readonly<{
	appId: string;
	requirement: DeviceRequirement;
	first?: boolean;
	onWaive: (variableId: string, waived: boolean) => void;
	onJumpToShared: () => void;
}>) {
	const { t } = useTranslation("common");
	const runtimeVars = useRuntimeVariables();
	const { variable, boardId, boardName, refs, seeded, stored, waived } =
		requirement;
	const { state, dirty, editorKey, update } = usePendingValue(seeded);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();

	const usable = useMemo(
		() => isRuntimeVariableConfigured(state, refs),
		[state, refs],
	);
	const status = waived ? "optional" : requirement.satisfied ? "set" : "need";

	const save = useCallback(async () => {
		if (!usable) return;
		if (!runtimeVars) {
			setError(
				t(
					"deviceStorageUnavailableKeepThisPageOpenAndTryAgain",
					"Device storage is unavailable. Keep this page open and try again.",
				),
			);
			return;
		}
		setError(undefined);
		setBusy(true);
		try {
			await runtimeVars.saveValues(appId, boardId, [
				{
					variableId: variable.id,
					variableName: variable.name,
					value: state.default_value ?? [],
					isSecret: variable.secret,
				},
			]);
			toast.success(t("savedOnThisDevice", "Saved on this device"));
		} catch {
			setError(
				t(
					"couldNotSaveOnThisDeviceDraftKept",
					"Could not save on this device. Your draft is still here. Try again.",
				),
			);
		} finally {
			setBusy(false);
		}
	}, [runtimeVars, usable, appId, boardId, variable, state.default_value, t]);

	const clear = useCallback(async () => {
		if (!runtimeVars) {
			setError(
				t(
					"deviceStorageUnavailableKeepThisPageOpenAndTryAgain",
					"Device storage is unavailable. Keep this page open and try again.",
				),
			);
			return;
		}
		setError(undefined);
		setBusy(true);
		try {
			await runtimeVars.deleteValue(appId, variable.id);
		} catch {
			setError(
				t(
					"couldNotClearDeviceValueTryAgain",
					"Could not clear this value. Try again.",
				),
			);
		} finally {
			setBusy(false);
		}
	}, [runtimeVars, appId, variable.id, t]);

	return (
		<RowShell tone="device" status={status} first={first}>
			<div className="flex items-start justify-between gap-4">
				<div className="flex min-w-0 items-start gap-3">
					<StatusDot status={status} />
					<div className="min-w-0 space-y-1.5">
						<div className="flex flex-wrap items-center gap-2">
							<VariableTypeIndicator
								type={variable.value_type}
								valueType={variable.data_type}
							/>
							<span className="truncate font-mono text-sm font-semibold">
								{variable.name}
							</span>
							{variable.secret && (
								<Tooltip>
									<TooltipTrigger asChild>
										<Badge variant="secondary" className="gap-1 text-xs">
											<KeyRoundIcon className="h-3 w-3" />
											{t("secret", "Secret")}
										</Badge>
									</TooltipTrigger>
									<TooltipContent className="max-w-xs">
										{t(
											"secretsAreStoredOnlyOnThisInstallationAndAreNeverSentToTheServerForARemoteRun",
											"Stored only on this installation and never sent to the server for a remote run.",
										)}
									</TooltipContent>
								</Tooltip>
							)}
							{requirement.alsoShared && (
								<Badge
									variant="outline"
									className="gap-1 border-sky-500/30 text-xs text-sky-600 dark:text-sky-400"
								>
									<UsersIcon className="h-3 w-3" />
									{t("alsoAnAppDefault", "also an app default")}
								</Badge>
							)}
							<span className="text-xs text-muted-foreground">{boardName}</span>
						</div>
						{variable.description && (
							<p className="text-xs text-muted-foreground">
								{variable.description}
							</p>
						)}
					</div>
				</div>

				<div className="flex shrink-0 items-center gap-1.5">
					{variable.secret && !requirement.satisfied && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-9 w-9"
									disabled={busy}
									onClick={() => onWaive(variable.id, !waived)}
								>
									{waived ? (
										<UndoIcon className="h-4 w-4" />
									) : (
										<XIcon className="h-4 w-4" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent className="max-w-xs">
								{waived
									? t("countThisAgain", "Count this again")
									: t(
											"theAppAlreadyProvidesThisWhetherAStoredSecretExistsCannotBeCheckedFromHere",
											"The app already provides this. Whether a stored secret exists cannot be checked from here, so this stops it blocking.",
										)}
							</TooltipContent>
						</Tooltip>
					)}
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={dirty && usable ? "default" : "ghost"}
								size="icon"
								className="h-9 w-9"
								onClick={save}
								aria-label={t("saveOnThisDevice", "Save on this device")}
								disabled={busy || !usable}
							>
								<SaveIcon className="h-4 w-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{t("saveOnThisDevice", "Save on this device")}
						</TooltipContent>
					</Tooltip>
					{stored && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-9 w-9 text-destructive hover:bg-destructive/10 hover:text-destructive"
									onClick={clear}
									disabled={busy}
								>
									<Trash2Icon className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("clearValue", "Clear value")}</TooltipContent>
						</Tooltip>
					)}
				</div>
			</div>

			<div className="pl-5">
				<RuntimeVariableEditor
					key={editorKey}
					variable={state}
					updateVariable={update}
					refs={refs}
					disabled={busy}
				/>
			</div>
			{error && (
				<p role="alert" className="pl-5 text-sm text-destructive">
					{error}
				</p>
			)}

			{requirement.alsoShared && (
				<button
					type="button"
					onClick={onJumpToShared}
					className="flex items-center gap-1.5 pl-5 text-left text-xs text-muted-foreground hover:text-foreground"
				>
					{t(
						"yourValueIsUsedInsteadOfTheAppDefaultForYourOwnRuns",
						"Your value is used instead of the app default for your own runs",
					)}
					<ArrowRightIcon className="h-3 w-3" />
				</button>
			)}

			{stored && (
				<p className="pl-5 text-xs text-muted-foreground">
					{t("setUpdatedat", "Set {{updatedAt}}", {
						updatedAt: new Date(stored.updatedAt).toLocaleDateString(),
					})}
				</p>
			)}
		</RowShell>
	);
}

/**
 * One app-wide parameter.
 *
 * Saves issue a board command, so they are shared, versioned and refused
 * without flow write access. The write is explicit and awaited rather than
 * fired per keystroke, so a rejection is something the user sees instead of an
 * unhandled promise.
 */
export function SharedParameterRow({
	appId,
	boardId,
	variable,
	refs,
	locked,
	disabled,
	overridden,
	onJumpToSetup,
}: Readonly<{
	appId: string;
	boardId: string;
	variable: IVariable;
	refs?: Record<string, string>;
	locked?: boolean;
	disabled?: boolean;
	overridden?: boolean;
	onJumpToSetup: () => void;
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const resolved = useMemo(
		() => seedRuntimeVariable(variable, variable.default_value),
		[variable],
	);
	const { state, dirty, editorKey, update } = usePendingValue(resolved);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();

	const readOnly = locked || disabled;

	const commit = useCallback(
		async (next: IVariable) => {
			setError(undefined);
			setBusy(true);
			try {
				await backend.boardState.executeCommand(
					appId,
					boardId,
					upsertVariableCommand({ variable: next }),
				);
				await invalidate(backend.boardState.getBoardVariables, [appId]);
				await invalidate(backend.boardState.getBoard, [appId, boardId]);
				toast.success(t("savedToTheApp", "Saved to the app"));
			} catch {
				setError(
					t(
						"couldNotSaveToTheAppDraftKept",
						"Could not save to the app. Your draft is still here. Try again.",
					),
				);
			} finally {
				setBusy(false);
			}
		},
		[backend, appId, boardId, invalidate, t],
	);

	const save = useCallback(() => commit(state), [commit, state]);

	// A secret's stored value never reaches the browser, so `default_value: null`
	// on the wire means "unchanged". Clearing one has to be an explicit empty
	// value, which is what this sends — and why it is a separate action from
	// Save rather than what an empty field does on its own.
	const clearSecret = useCallback(
		() => commit({ ...state, default_value: [] }),
		[commit, state],
	);

	const canSave = variable.secret ? dirty : true;

	return (
		<RowShell tone="shared" status="set">
			<div className="flex items-start justify-between gap-4">
				<div className="flex min-w-0 items-start gap-3">
					<StatusDot status="set" />
					<div className="min-w-0 space-y-1.5">
						<div className="flex flex-wrap items-center gap-2">
							<VariableTypeIndicator
								type={variable.value_type}
								valueType={variable.data_type}
							/>
							<span className="truncate font-mono text-sm font-semibold">
								{variable.name}
							</span>
							{variable.secret && (
								<Badge variant="secondary" className="gap-1 text-xs">
									<KeyRoundIcon className="h-3 w-3" />
									{t("secret", "Secret")}
								</Badge>
							)}
							{locked && (
								<Badge variant="outline" className="gap-1 text-xs">
									<LockIcon className="h-3 w-3" />
									{t("lockedByTheApp", "Locked by the app")}
								</Badge>
							)}
							{overridden && (
								<Badge
									variant="outline"
									className="gap-1 border-violet-500/30 text-xs text-violet-600 dark:text-violet-400"
								>
									{t("youOverrideThis", "you override this")}
								</Badge>
							)}
						</div>
						{variable.description && (
							<p className="text-xs text-muted-foreground">
								{variable.description}
							</p>
						)}
						{locked && (
							<p className="text-xs text-muted-foreground">
								{t(
									"theFlowAuthorMarkedThisReadonlyItIsShownSoYouKnowItExistsAndWhatItIsSetTo",
									"The flow author marked this read-only. It is shown so you know it exists and what it is set to.",
								)}
							</p>
						)}
					</div>
				</div>

				<div className="flex shrink-0 items-center gap-1.5">
					{variable.secret && !readOnly && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-9 w-9 text-destructive hover:bg-destructive/10 hover:text-destructive"
									onClick={clearSecret}
									disabled={busy}
								>
									<Trash2Icon className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{t("clearStoredSecret", "Clear the stored secret")}
							</TooltipContent>
						</Tooltip>
					)}
					<Button
						variant={dirty ? "default" : "outline"}
						size="sm"
						onClick={save}
						disabled={busy || readOnly || !canSave}
					>
						{dirty ? (
							<SaveIcon className="mr-1.5 h-3.5 w-3.5" />
						) : (
							<CheckIcon className="mr-1.5 h-3.5 w-3.5" />
						)}
						{t("save", "Save")}
					</Button>
				</div>
			</div>

			<div className="pl-5">
				<RuntimeVariableEditor
					key={editorKey}
					variable={state}
					updateVariable={update}
					refs={refs}
					disabled={busy || readOnly}
				/>
				{variable.secret && (
					<p className="mt-1.5 text-xs text-muted-foreground">
						{t(
							"aStoredSecretIsNeverSentToTheBrowserLeaveThisBlankToKeepTheCurrentValue",
							"A stored secret is never sent to the browser. Leave this blank to keep the current value.",
						)}
					</p>
				)}
			</div>

			{error && (
				<p role="alert" className="pl-5 text-sm text-destructive">
					{error}
				</p>
			)}

			{overridden && (
				<button
					type="button"
					onClick={onJumpToSetup}
					className="flex items-center gap-1.5 pl-5 text-left text-xs text-muted-foreground hover:text-foreground"
				>
					{t(
						"youHaveADeviceValueThatIsUsedInsteadOfThisForYourOwnRuns",
						"You have a device value that is used instead of this for your own runs",
					)}
					<ArrowRightIcon className="h-3 w-3" />
				</button>
			)}
		</RowShell>
	);
}
