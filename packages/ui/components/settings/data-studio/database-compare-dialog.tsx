"use client";

import { ArrowLeftRight, Loader2 } from "lucide-react";
import { useState } from "react";
import { getErrorMessage } from "../../../lib/error-message";
import { asArray } from "../../../lib/response-shape";
import { useBackend } from "../../../state/backend-state";
import type {
	IDatabaseDiff,
	IDatabaseReference,
	IDatabaseSelector,
} from "../../../state/backend-state/db-state";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";

export function DatabaseCompareDialog({
	appId,
	table,
	userScoped,
	reference,
}: {
	appId: string;
	table: string;
	userScoped?: boolean;
	reference: IDatabaseReference;
}) {
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [branch, setBranch] = useState("main");
	const [version, setVersion] = useState("");
	const [tag, setTag] = useState("");
	const [key, setKey] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [result, setResult] = useState<IDatabaseDiff | null>(null);
	const compare = async () => {
		if (busy || !key.trim()) return;
		setError(null);
		setResult(null);
		const target: IDatabaseSelector = tag.trim()
			? { tag: tag.trim() }
			: { branch: branch.trim() || "main" };
		if (!target.tag && version.trim()) {
			if (
				!/^\d+$/.test(version.trim()) ||
				!Number.isSafeInteger(Number(version)) ||
				Number(version) < 1
			) {
				setError("Enter a valid version number or leave it empty for latest.");
				return;
			}
			target.version = Number(version);
		}
		setBusy(true);
		try {
			setResult(
				await backend.dbState.databaseCompare(
					appId,
					table,
					target,
					key.trim(),
					100,
					userScoped,
					{ branch: reference.branch, version: reference.version },
				),
			);
		} catch (err) {
			setError(getErrorMessage(err));
		} finally {
			setBusy(false);
		}
	};
	return (
		<>
			<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
				<ArrowLeftRight className="h-4 w-4" />
				Compare
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
					<DialogHeader>
						<DialogTitle>Compare {table} snapshots</DialogTitle>
						<DialogDescription>
							Use {reference.branch}, version {reference.version}, as the
							baseline. Added and removed rows are reported relative to this
							baseline. The key must contain unique, non-null string or integer
							values.
						</DialogDescription>
					</DialogHeader>
					<div className="grid gap-3 sm:grid-cols-2">
						<div className="space-y-1">
							<Label htmlFor="compare-branch">Target branch</Label>
							<Input
								id="compare-branch"
								value={branch}
								disabled={busy || Boolean(tag)}
								onChange={(event) => {
									setBranch(event.target.value);
									setResult(null);
								}}
							/>
						</div>
						<div className="space-y-1">
							<Label htmlFor="compare-version">
								Target version (empty for latest)
							</Label>
							<Input
								id="compare-version"
								value={version}
								disabled={busy || Boolean(tag)}
								onChange={(event) => {
									setVersion(event.target.value);
									setResult(null);
								}}
							/>
						</div>
						<div className="space-y-1">
							<Label htmlFor="compare-tag">Or target tag</Label>
							<Input
								id="compare-tag"
								value={tag}
								disabled={busy}
								onChange={(event) => {
									setTag(event.target.value);
									setResult(null);
								}}
							/>
						</div>
						<div className="space-y-1">
							<Label htmlFor="compare-key">Unique key column</Label>
							<Input
								id="compare-key"
								value={key}
								disabled={busy}
								onChange={(event) => {
									setKey(event.target.value);
									setResult(null);
								}}
								placeholder="record_id"
							/>
						</div>
					</div>
					<Button disabled={busy || !key.trim()} onClick={() => void compare()}>
						{busy && <Loader2 className="h-4 w-4 animate-spin" />}Compare
						snapshots
					</Button>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					{result && (
						<div className="space-y-3">
							<p className="text-sm">
								{result.source?.branch} v{result.source?.version} →{" "}
								{result.target?.branch} v{result.target?.version}
							</p>
							<div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
								{[
									["Added", result.added],
									["Removed", result.removed],
									["Changed", result.changed],
									["Unchanged", result.unchanged],
								].map(([label, count]) => (
									<div key={label} className="rounded-md border p-3">
										<p className="text-xs text-muted-foreground">{label}</p>
										<p className="text-lg font-semibold">{count}</p>
									</div>
								))}
							</div>
							{asArray(result.schema_changes).length > 0 && (
								<div className="space-y-1">
									<h3 className="text-sm font-medium">Schema changes</h3>
									<ul className="list-inside list-disc text-sm">
										{asArray(result.schema_changes).map((change) => (
											<li key={change}>{change}</li>
										))}
									</ul>
								</div>
							)}
							{result.truncated && (
								<p className="text-xs text-muted-foreground">
									Showing the first 100 changed rows.
								</p>
							)}
							{asArray(result.rows).map((row, index) => (
								<details
									key={`${row.kind}:${index}`}
									className="rounded-md border p-2 text-sm"
								>
									<summary className="cursor-pointer">
										{row.kind}: {String(row.key)}
									</summary>
									<pre className="mt-2 max-h-64 overflow-auto bg-muted p-2 text-xs">
										{JSON.stringify(
											{ before: row.before, after: row.after },
											null,
											2,
										)}
									</pre>
								</details>
							))}
						</div>
					)}
				</DialogContent>
			</Dialog>
		</>
	);
}
