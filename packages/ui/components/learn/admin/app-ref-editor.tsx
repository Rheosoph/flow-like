"use client";
import { useState } from "react";
import type { LessonAppRef, LessonAppRefKind } from "../../../lib/learn/types";
import { Button } from "../../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";

const kinds: ReadonlyArray<{ readonly value: LessonAppRefKind; readonly label: string }> = [
	{ value: "NAVIGATE", label: "Open subpage" },
	{ value: "FOCUS_NODE", label: "Focus node on board" },
	{ value: "ADD_NODE", label: "Add node to board" },
	{ value: "CREATE_EVENT", label: "Create event" },
	{ value: "OPEN_OR_CLONE_APP", label: "Open or clone app" },
];

export interface AppRefFormValue {
	readonly kind: LessonAppRefKind;
	readonly app_alias: string | null;
	readonly app_id: string | null;
	readonly label: string | null;
	readonly target: Record<string, unknown>;
}

interface AppRefEditorProps {
	readonly initial?: LessonAppRef | null;
	readonly aliasOptions: ReadonlyArray<string>;
	readonly onSubmit: (value: AppRefFormValue) => Promise<void> | void;
	readonly onDelete?: () => Promise<void> | void;
	readonly submitting?: boolean;
}

function fromInitial(r?: LessonAppRef | null): AppRefFormValue {
	return {
		kind: (r?.kind as LessonAppRefKind) ?? "NAVIGATE",
		app_alias: r?.app_alias ?? null,
		app_id: r?.app_id ?? null,
		label: r?.label ?? null,
		target: (r?.target as unknown as Record<string, unknown>) ?? {},
	};
}

export function AppRefEditor({
	initial,
	aliasOptions,
	onSubmit,
	onDelete,
	submitting,
}: AppRefEditorProps) {
	const [value, setValue] = useState<AppRefFormValue>(fromInitial(initial));
	function patch<K extends keyof AppRefFormValue>(
		key: K,
		v: AppRefFormValue[K],
	) {
		setValue((prev) => ({ ...prev, [key]: v }));
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center justify-between gap-2">
					<span>{initial ? "Edit app reference" : "New app reference"}</span>
					{onDelete && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() => void onDelete()}
						>
							Delete
						</Button>
					)}
				</CardTitle>
			</CardHeader>
			<CardContent>
				<form
					className="space-y-4"
					onSubmit={(e) => {
						e.preventDefault();
						void onSubmit(value);
					}}
				>
					<div className="grid grid-cols-1 md:grid-cols-3 gap-3">
						<div className="space-y-2">
							<Label>Kind</Label>
							<Select
								value={value.kind}
								onValueChange={(v) => patch("kind", v as LessonAppRefKind)}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{kinds.map((k) => (
										<SelectItem key={k.value} value={k.value}>
											{k.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label>Alias (course app link)</Label>
							<Select
								value={value.app_alias ?? ""}
								onValueChange={(v) => patch("app_alias", v || null)}
							>
								<SelectTrigger>
									<SelectValue placeholder="(none)" />
								</SelectTrigger>
								<SelectContent>
									{aliasOptions.map((a) => (
										<SelectItem key={a} value={a}>
											{a}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label>Button label (optional)</Label>
							<Input
								value={value.label ?? ""}
								onChange={(e) => patch("label", e.target.value || null)}
								placeholder="Open the playground"
							/>
						</div>
					</div>
					<TargetEditor
						kind={value.kind}
						target={value.target}
						onChange={(t) => patch("target", t)}
					/>
					<Button type="submit" disabled={submitting}>
						Save reference
					</Button>
				</form>
			</CardContent>
		</Card>
	);
}

interface TargetEditorProps {
	readonly kind: LessonAppRefKind;
	readonly target: Record<string, unknown>;
	readonly onChange: (next: Record<string, unknown>) => void;
}

function TargetEditor({ kind, target, onChange }: TargetEditorProps) {
	const setVal = (k: string, v: unknown) =>
		onChange({ ...target, [k]: v === "" ? undefined : v });

	if (kind === "NAVIGATE") {
		return (
			<div className="rounded-md border p-3 space-y-3 bg-muted/20">
				<div className="grid grid-cols-1 md:grid-cols-2 gap-3">
					<div className="space-y-2">
						<Label>Subpath</Label>
						<Select
							value={(target.subpath as string) ?? "config"}
							onValueChange={(v) => setVal("subpath", v)}
						>
							<SelectTrigger>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="config">App config</SelectItem>
								<SelectItem value="events">Events</SelectItem>
								<SelectItem value="pages">Pages</SelectItem>
								<SelectItem value="flow">Flow editor</SelectItem>
								<SelectItem value="use">Run app</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="space-y-2">
						<Label>Extra params (JSON)</Label>
						<Textarea
							value={JSON.stringify(target.params ?? {}, null, 2)}
							rows={3}
							className="font-mono text-xs"
							onChange={(e) => {
								try {
									setVal("params", JSON.parse(e.target.value || "{}"));
								} catch {
									/* mid-typing */
								}
							}}
						/>
					</div>
				</div>
			</div>
		);
	}
	if (kind === "FOCUS_NODE") {
		return (
			<div className="grid grid-cols-1 md:grid-cols-2 gap-3 rounded-md border p-3 bg-muted/20">
				<div className="space-y-2">
					<Label>Source board ID</Label>
					<Input
						value={(target.boardId as string) ?? ""}
						onChange={(e) => setVal("boardId", e.target.value)}
					/>
				</div>
				<div className="space-y-2">
					<Label>Source node ID</Label>
					<Input
						value={(target.nodeId as string) ?? ""}
						onChange={(e) => setVal("nodeId", e.target.value)}
					/>
				</div>
			</div>
		);
	}
	if (kind === "ADD_NODE") {
		return (
			<div className="grid grid-cols-1 md:grid-cols-3 gap-3 rounded-md border p-3 bg-muted/20">
				<div className="space-y-2">
					<Label>Source board ID</Label>
					<Input
						value={(target.boardId as string) ?? ""}
						onChange={(e) => setVal("boardId", e.target.value)}
					/>
				</div>
				<div className="space-y-2">
					<Label>Node type ID (catalog name)</Label>
					<Input
						value={(target.nodeTypeId as string) ?? ""}
						onChange={(e) => setVal("nodeTypeId", e.target.value)}
						placeholder="math.add"
					/>
				</div>
				<div className="space-y-2">
					<Label>Coords [x, y]</Label>
					<Input
						value={JSON.stringify(target.coords ?? [])}
						onChange={(e) => {
							try {
								setVal("coords", JSON.parse(e.target.value || "null"));
							} catch {
								/* mid-typing */
							}
						}}
						placeholder="[120, 80]"
					/>
				</div>
			</div>
		);
	}
	if (kind === "CREATE_EVENT") {
		return (
			<div className="rounded-md border p-3 space-y-2 bg-muted/20">
				<Label>Event template (JSON)</Label>
				<Textarea
					value={JSON.stringify(target.template ?? {}, null, 2)}
					rows={6}
					className="font-mono text-xs"
					onChange={(e) => {
						try {
							setVal("template", JSON.parse(e.target.value || "{}"));
						} catch {
							/* mid-typing */
						}
					}}
				/>
			</div>
		);
	}
	if (kind === "OPEN_OR_CLONE_APP") {
		return (
			<div className="rounded-md border p-3 space-y-2 bg-muted/20">
				<Label>Shared app ID (the source app for forking)</Label>
				<Input
					value={(target.sharedAppId as string) ?? ""}
					onChange={(e) => setVal("sharedAppId", e.target.value)}
				/>
			</div>
		);
	}
	return null;
}
