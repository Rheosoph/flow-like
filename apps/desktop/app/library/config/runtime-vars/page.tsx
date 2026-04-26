"use client";

import {
	Badge,
	Button,
	type IBoard,
	type IVariable,
	Input,
	Progress,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "@tm9657/flow-like-ui/lib/uint8";
import { useLiveQuery } from "dexie-react-hooks";
import {
	CheckCircle2Icon,
	ChevronDownIcon,
	CircleDotIcon,
	EyeIcon,
	EyeOffIcon,
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
						<h2 className="text-2xl font-semibold">All Set!</h2>
						<p className="text-muted-foreground">
							This app doesn&apos;t require any runtime variables or secrets.
						</p>
					</div>
					<div className="p-4 rounded-lg bg-muted/50 text-sm text-muted-foreground">
						<strong>Tip:</strong> Mark variables as &quot;Runtime
						Configured&quot; or &quot;Secret&quot; in the Flow Editor to manage
						them here.
					</div>
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
								Runtime Variables
							</h1>
							<p className="text-sm text-muted-foreground">
								Set secrets and user-specific values that your flows need at
								runtime
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
						<p className="text-xs text-muted-foreground">
							{configuredCount} of {totalVariables} configured
						</p>
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
					<p className="text-sm font-medium">Security Notice</p>
					<p className="text-xs text-muted-foreground">
						Runtime variables are stored locally on your device and are never
						uploaded to the server. For remote execution, only non-secret
						runtime variables will be sent.
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
	if (isComplete) {
		return (
			<Badge className="gap-1.5 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20 hover:bg-emerald-500/20">
				<CheckCircle2Icon className="w-3.5 h-3.5" />
				All Configured
			</Badge>
		);
	}
	return (
		<Badge variant="secondary" className="gap-1.5">
			<CircleDotIcon className="w-3.5 h-3.5" />
			{configured}/{total}
		</Badge>
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
						<p className="text-sm text-muted-foreground">
							{configuredCount} of {variables.length} configured
						</p>
					</div>
				</div>
				<div className="flex items-center gap-3">
					{isComplete && (
						<Badge className="bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20">
							Complete
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
						/>
					))}
				</div>
			)}
		</div>
	);
}

function decodeValue(data?: number[] | null): string {
	if (!data) return "";
	try {
		const decoded = parseUint8ArrayToJson(data);
		return typeof decoded === "string" ? decoded : JSON.stringify(decoded);
	} catch {
		return "";
	}
}

function VariableRow({
	appId,
	boardId,
	variable,
	savedValue,
}: Readonly<{
	appId: string;
	boardId: string;
	variable: IVariable;
	savedValue?: IRuntimeVariableValue;
}>) {
	const [value, setValue] = useState<string>(
		() => decodeValue(savedValue?.value) || decodeValue(variable.default_value),
	);
	const [showPassword, setShowPassword] = useState(false);
	const [isSaving, setIsSaving] = useState(false);
	const [hasChanges, setHasChanges] = useState(false);

	// Sync value when savedValue changes from Dexie
	useEffect(() => {
		if (!hasChanges) {
			const decoded =
				decodeValue(savedValue?.value) || decodeValue(variable.default_value);
			setValue(decoded);
		}
	}, [savedValue?.value, variable.default_value, hasChanges]);

	const handleSave = useCallback(async () => {
		setIsSaving(true);
		try {
			const encodedValue = convertJsonToUint8Array(value);
			if (!encodedValue) return;
			await setRuntimeVar(
				appId,
				boardId,
				variable.id,
				variable.name,
				encodedValue,
				variable.secret,
			);
			setHasChanges(false);
		} finally {
			setIsSaving(false);
		}
	}, [appId, boardId, variable, value]);

	const handleDelete = useCallback(async () => {
		await deleteRuntimeVar(appId, variable.id);
		setValue("");
		setHasChanges(false);
	}, [appId, variable.id]);

	const handleChange = useCallback((newValue: string) => {
		setValue(newValue);
		setHasChanges(true);
	}, []);

	const isSecret = variable.secret;
	const isConfigured = !!savedValue;

	return (
		<div className="flex items-center gap-4 p-4 hover:bg-muted/30 transition-colors">
			{/* Status Indicator */}
			<div
				className={cn(
					"w-2 h-2 rounded-full shrink-0",
					isConfigured ? "bg-emerald-500" : "bg-muted-foreground/30",
				)}
			/>

			{/* Variable Info */}
			<div className="flex-1 min-w-0 space-y-1">
				<div className="flex items-center gap-2">
					<span className="font-medium truncate">{variable.name}</span>
					{isSecret ? (
						<Tooltip>
							<TooltipTrigger>
								<Badge variant="secondary" className="gap-1 text-xs shrink-0">
									<KeyRoundIcon className="w-3 h-3" />
									Secret
								</Badge>
							</TooltipTrigger>
							<TooltipContent>
								This value is encrypted and never sent to remote servers
							</TooltipContent>
						</Tooltip>
					) : (
						<Badge variant="outline" className="text-xs shrink-0">
							Runtime
						</Badge>
					)}
				</div>
				{variable.description && (
					<p className="text-xs text-muted-foreground truncate">
						{variable.description}
					</p>
				)}
			</div>

			{/* Input */}
			<div className="flex items-center gap-2 shrink-0">
				<div className="relative w-64">
					<Input
						type={isSecret && !showPassword ? "password" : "text"}
						value={value}
						onChange={(e) => handleChange(e.target.value)}
						placeholder={isSecret ? "••••••••" : "Enter value..."}
						className={cn(
							"h-9 pr-9 text-sm",
							isConfigured && !hasChanges && "border-emerald-500/50",
						)}
					/>
					{isSecret && (
						<Button
							variant="ghost"
							size="icon"
							className="absolute right-0 top-0 h-9 w-9 hover:bg-transparent"
							onClick={() => setShowPassword(!showPassword)}
						>
							{showPassword ? (
								<EyeOffIcon className="w-4 h-4 text-muted-foreground" />
							) : (
								<EyeIcon className="w-4 h-4 text-muted-foreground" />
							)}
						</Button>
					)}
				</div>

				{/* Actions */}
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant={hasChanges ? "default" : "ghost"}
							size="icon"
							className="h-9 w-9"
							onClick={handleSave}
							disabled={isSaving || !value}
						>
							<SaveIcon className="w-4 h-4" />
						</Button>
					</TooltipTrigger>
					<TooltipContent>Save</TooltipContent>
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
						<TooltipContent>Delete</TooltipContent>
					</Tooltip>
				)}
			</div>
		</div>
	);
}
