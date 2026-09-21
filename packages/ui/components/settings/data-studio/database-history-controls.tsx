"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	GitBranch,
	History,
	Loader2,
	LockKeyhole,
	Plus,
	RefreshCw,
	Tag,
	Trash2,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { getErrorMessage } from "../../../lib/error-message";
import { useBackend } from "../../../state/backend-state";
import type {
	IDatabaseAction,
	IDatabaseReference,
	IDatabaseSelector,
} from "../../../state/backend-state/db-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { DatabaseCompareDialog } from "./database-compare-dialog";
import { isDatabaseSnapshot } from "./database-reference";

interface PendingAction {
	action: IDatabaseAction;
	title: string;
	description: string;
	destructive?: boolean;
	needsName?: boolean;
	reference?: IDatabaseReference;
}

export function DatabaseHistoryControls({
	appId,
	table,
	userScoped,
	selector,
	canWrite,
	onSelect,
}: {
	appId: string;
	table: string;
	userScoped?: boolean;
	selector: IDatabaseSelector;
	canWrite: boolean;
	onSelect: (selector: IDatabaseSelector) => void;
}) {
	const backend = useBackend();
	const cache = useQueryClient();
	const history = useInvoke(backend.dbState.databaseHistory, backend.dbState, [
		appId,
		table,
		userScoped,
		selector,
	]);
	const [open, setOpen] = useState(false);
	const [pending, setPending] = useState<PendingAction | null>(null);
	const [name, setName] = useState("");
	const [days, setDays] = useState("30");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const reference = history.error ? undefined : history.data?.reference;
	const branch = reference?.branch ?? selector.branch ?? "main";
	const readonly = isDatabaseSnapshot(selector) || reference?.read_only;
	const canManage = canWrite && !selector.read_only && Boolean(reference);
	const revision = selector.tag
		? `tag:${selector.tag}`
		: selector.version !== undefined
			? `version:${selector.version}`
			: "latest";

	const refresh = async () => {
		await cache.invalidateQueries({
			predicate: (query) =>
				query.queryKey[1] === appId && query.queryKey[2] === table,
		});
	};
	const request = (action: PendingAction) => {
		setPending({ ...action, reference });
		setName("");
		setError(null);
	};
	const snapshotSelector = (ref: IDatabaseReference): IDatabaseSelector => ({
		branch: ref.branch,
		version: ref.version,
	});
	const run = async () => {
		if (!pending || busy || !canManage || !pending.reference) return;
		const selectedReference = pending.reference;
		let action = pending.action;
		if (pending.needsName)
			action = { ...action, name: name.trim() } as IDatabaseAction;
		if (action.action === "cleanup")
			action = { ...action, older_than_days: Number(days) };
		setBusy(true);
		setError(null);
		try {
			// Bind operations to the exact version displayed when the dialog opens.
			// Restore and cleanup enforce their own write rules in the backend.
			const target =
				action.action === "delete_branch"
					? { branch: "main" }
					: action.action === "cleanup"
						? { branch: selectedReference.branch }
						: snapshotSelector(selectedReference);
			const result = await backend.dbState.databaseAction(
				appId,
				table,
				action,
				userScoped,
				target,
			);
			if (action.action === "create_branch" || action.action === "restore")
				onSelect({ branch: result.reference.branch });
			if (action.action === "snapshot")
				onSelect(snapshotSelector(result.reference));
			if (action.action === "delete_branch" && action.name === branch)
				onSelect({ branch: "main" });
			if (
				(action.action === "delete_tag" || action.action === "update_tag") &&
				action.name === selector.tag
			)
				onSelect(snapshotSelector(selectedReference));
			await refresh();
			if (action.action === "clone") {
				await cache.invalidateQueries({
					predicate: (query) => query.queryKey[1] === appId,
				});
			}
			toast.success(
				action.action === "cleanup"
					? "Version cleanup completed"
					: action.action === "clone"
						? `Created table ${result.reference.table}`
						: "Database reference updated",
				{
					description: result.cleanup
						? `${result.cleanup.old_versions} versions removed; ${result.cleanup.bytes_removed.toLocaleString()} bytes reclaimed.`
						: undefined,
				},
			);
			setPending(null);
		} catch (err) {
			setError(getErrorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="shrink-0 space-y-2 border-b px-4 py-3">
			<div className="flex flex-wrap items-center gap-2">
				<GitBranch className="h-4 w-4 text-muted-foreground" />
				<Select
					value={branch}
					onValueChange={(value) => onSelect({ branch: value })}
					disabled={busy}
				>
					<SelectTrigger aria-label="Database branch" className="h-8 w-44">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{Array.from(
							new Set([
								"main",
								branch,
								...(history.data?.branches.map((item) => item.name) ?? []),
							]),
						).map((value) => (
							<SelectItem key={value} value={value}>
								{value}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Select
					value={revision}
					onValueChange={(value) => {
						if (value === "latest") onSelect({ branch });
						else if (value.startsWith("tag:"))
							onSelect({ tag: value.slice(4) });
						else onSelect({ branch, version: Number(value.slice(8)) });
					}}
					disabled={busy}
				>
					<SelectTrigger aria-label="Database revision" className="h-8 w-48">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="latest">Latest version</SelectItem>
						{selector.tag &&
							!history.data?.tags.some((tag) => tag.name === selector.tag) && (
								<SelectItem value={`tag:${selector.tag}`}>
									Tag: {selector.tag}
								</SelectItem>
							)}
						{selector.version !== undefined &&
							!history.data?.versions.some(
								(version) => version.version === selector.version,
							) && (
								<SelectItem value={`version:${selector.version}`}>
									Version {selector.version}
								</SelectItem>
							)}
						{history.data?.versions.map((version) => (
							<SelectItem
								key={version.version}
								value={`version:${version.version}`}
							>
								Version {version.version}
							</SelectItem>
						))}
						{history.data?.tags.map((tag) => (
							<SelectItem key={tag.name} value={`tag:${tag.name}`}>
								Tag: {tag.name}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				{readonly && (
					<Badge variant="secondary" className="gap-1">
						<LockKeyhole className="h-3 w-3" />
						Read-only snapshot
					</Badge>
				)}
				{reference && (
					<span className="text-xs text-muted-foreground">
						Version {reference.version}
					</span>
				)}
				<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
					<History className="h-4 w-4" />
					History & branches
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8"
					aria-label="Refresh database reference"
					disabled={history.isFetching || busy}
					onClick={() => void refresh()}
				>
					<RefreshCw
						className={`h-4 w-4 ${history.isFetching ? "animate-spin" : ""}`}
					/>
				</Button>
			</div>
			{history.error && (
				<p role="alert" className="text-xs text-destructive">
					{getErrorMessage(history.error)}
				</p>
			)}
			{readonly && (
				<p className="text-xs text-muted-foreground">
					Rows, schema, and indexes are pinned to this snapshot. Select the
					latest version to edit its branch, or create a branch from this
					snapshot.
				</p>
			)}
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
					<DialogHeader>
						<DialogTitle>{table}: history and references</DialogTitle>
						<DialogDescription>
							Branches have separate writable histories. Tags name retained
							snapshots; moving a tag changes what its name resolves to.
						</DialogDescription>
					</DialogHeader>
					{reference && (
						<DatabaseCompareDialog
							appId={appId}
							table={table}
							userScoped={userScoped}
							reference={reference}
						/>
					)}
					{canManage && reference && (
						<div className="flex flex-wrap gap-2">
							<Button
								size="sm"
								variant="outline"
								onClick={() =>
									request({
										action: { action: "create_branch", name: "" },
										title: "Create branch",
										description: `Start a writable branch from ${branch}, version ${reference.version}.`,
										needsName: true,
									})
								}
							>
								<Plus className="h-4 w-4" />
								Create branch
							</Button>
							<Button
								size="sm"
								variant="outline"
								onClick={() =>
									request({
										action: { action: "snapshot" },
										title: "Save tagged snapshot",
										description: `Retain ${branch}, version ${reference.version}, for reproducible training or evaluation.`,
										needsName: true,
									})
								}
							>
								<Tag className="h-4 w-4" />
								Save snapshot
							</Button>
							<Button
								size="sm"
								variant="outline"
								onClick={() =>
									request({
										action: { action: "clone", name: "" },
										title: "Clone into another table",
										description: `Create a shallow clone of ${branch}, version ${reference.version}. The new table shares data files with its source; keep the source storage available.`,
										needsName: true,
									})
								}
							>
								Clone table
							</Button>
							{!readonly && (
								<Button
									size="sm"
									variant="outline"
									onClick={() =>
										request({
											action: { action: "cleanup", older_than_days: 30 },
											title: "Clean up old versions",
											description:
												"Remove unprotected versions older than the retention period. Removed versions can no longer be opened. Tags and branch references remain protected.",
											destructive: true,
										})
									}
								>
									Clean up versions
								</Button>
							)}
						</div>
					)}
					<section className="space-y-2">
						<h3 className="text-sm font-semibold">Branches</h3>
						{history.data?.branches.map((item) => (
							<div
								key={item.name}
								className="flex items-center gap-2 rounded-md border p-2 text-sm"
							>
								<GitBranch className="h-4 w-4 shrink-0" />
								<div className="min-w-0 flex-1">
									<p className="truncate">{item.name}</p>
									{item.parent_branch && (
										<p className="text-xs text-muted-foreground">
											From {item.parent_branch}, version {item.parent_version}
										</p>
									)}
								</div>
								<Button
									size="sm"
									variant="ghost"
									onClick={() => {
										onSelect({ branch: item.name });
										setOpen(false);
									}}
								>
									Open
								</Button>
								{canManage && item.name !== "main" && (
									<Button
										size="icon"
										variant="ghost"
										aria-label={`Delete branch ${item.name}`}
										onClick={() =>
											request({
												action: { action: "delete_branch", name: item.name },
												title: `Delete branch ${item.name}?`,
												description: `Remove the ${item.name} branch from ${table}. This does not delete the table or other branches.`,
												destructive: true,
											})
										}
									>
										<Trash2 className="h-4 w-4 text-destructive" />
									</Button>
								)}
							</div>
						))}
					</section>
					<section className="space-y-2">
						<h3 className="text-sm font-semibold">Tags</h3>
						{history.data?.tags.length === 0 && (
							<p className="text-sm text-muted-foreground">
								No tagged snapshots yet.
							</p>
						)}
						{history.data?.tags.map((tag) => (
							<div
								key={tag.name}
								className="flex flex-wrap items-center gap-2 rounded-md border p-2 text-sm"
							>
								<Tag className="h-4 w-4 shrink-0" />
								<div className="min-w-0 flex-1">
									<p className="truncate">{tag.name}</p>
									<p className="text-xs text-muted-foreground">
										{tag.branch}, version {tag.version}
									</p>
								</div>
								<Button
									size="sm"
									variant="ghost"
									onClick={() => {
										onSelect({ tag: tag.name });
										setOpen(false);
									}}
								>
									Open
								</Button>
								{canManage && (
									<>
										<Button
											size="sm"
											variant="ghost"
											onClick={() =>
												request({
													action: { action: "update_tag", name: tag.name },
													title: `Move tag ${tag.name}?`,
													description: `Move this tag to ${branch}, version ${reference?.version}. Workflows using the tag name will read the new snapshot.`,
													destructive: true,
												})
											}
										>
											Move here
										</Button>
										<Button
											size="icon"
											variant="ghost"
											aria-label={`Delete tag ${tag.name}`}
											onClick={() =>
												request({
													action: { action: "delete_tag", name: tag.name },
													title: `Delete tag ${tag.name}?`,
													description:
														"Remove this name and its retention protection. The version stays available until it is cleaned up.",
													destructive: true,
												})
											}
										>
											<Trash2 className="h-4 w-4 text-destructive" />
										</Button>
									</>
								)}
							</div>
						))}
					</section>
					<section className="space-y-2">
						<h3 className="text-sm font-semibold">Versions on {branch}</h3>
						{history.data?.versions.map((version) => (
							<div
								key={version.version}
								className="flex items-center gap-2 rounded-md border p-2 text-sm"
							>
								<div className="min-w-0 flex-1">
									<p>Version {version.version}</p>
									<p className="text-xs text-muted-foreground">
										{new Date(version.timestamp).toLocaleString()}
									</p>
									{Object.keys(version.metadata).length > 0 && (
										<p className="truncate text-xs text-muted-foreground">
											{JSON.stringify(version.metadata)}
										</p>
									)}
								</div>
								<Button
									size="sm"
									variant="ghost"
									onClick={() => {
										onSelect({ branch, version: version.version });
										setOpen(false);
									}}
								>
									Inspect
								</Button>
							</div>
						))}
						{canManage && readonly && reference && (
							<Button
								variant="outline"
								onClick={() =>
									request({
										action: { action: "restore" },
										title: `Restore version ${reference.version}?`,
										description: `Make this snapshot the latest state of ${table} on ${branch}. This creates a new version and changes the data read by workflows using that branch.`,
										destructive: true,
									})
								}
							>
								Restore selected version
							</Button>
						)}
					</section>
				</DialogContent>
			</Dialog>
			<Dialog
				open={pending !== null}
				onOpenChange={(value) => {
					if (!value && !busy) setPending(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{pending?.title}</DialogTitle>
						<DialogDescription>{pending?.description}</DialogDescription>
					</DialogHeader>
					{pending?.needsName && (
						<div className="space-y-2">
							<Label htmlFor="database-reference-name">Name</Label>
							<Input
								id="database-reference-name"
								value={name}
								onChange={(event) => setName(event.target.value)}
								disabled={busy}
								autoComplete="off"
							/>
						</div>
					)}
					{pending?.action.action === "cleanup" && (
						<div className="space-y-2">
							<Label htmlFor="database-retention-days">
								Keep at least this many days of history
							</Label>
							<Input
								id="database-retention-days"
								type="number"
								min="1"
								step="1"
								value={days}
								onChange={(event) => setDays(event.target.value)}
								disabled={busy}
							/>
						</div>
					)}
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<DialogFooter>
						<Button
							variant="outline"
							disabled={busy}
							onClick={() => setPending(null)}
						>
							Cancel
						</Button>
						<Button
							variant={pending?.destructive ? "destructive" : "default"}
							disabled={
								busy ||
								!reference ||
								(pending?.needsName && !name.trim()) ||
								(pending?.action.action === "cleanup" &&
									(!Number.isSafeInteger(Number(days)) || Number(days) < 1))
							}
							onClick={() => void run()}
						>
							{busy && <Loader2 className="h-4 w-4 animate-spin" />}Confirm
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
