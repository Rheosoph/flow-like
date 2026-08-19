"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, Play, Workflow } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import type {
	GraphOverlay,
	OntologyActionDefinition,
	OntologyActionRun,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../dialog";
import { Input } from "../input";
import { Label } from "../label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../select";
import { Switch } from "../switch";
import { Textarea } from "../textarea";

export interface OntologyActionTarget {
	action: OntologyActionDefinition;
	node: SubgraphNode;
}

export type InvokeOntologyAction = (
	action: OntologyActionDefinition,
	node: SubgraphNode,
	parameters: Record<string, unknown>,
	onStatus?: (run: OntologyActionRun) => void,
) => Promise<OntologyActionRun>;

interface ActionSchemaProperty {
	type?: string | string[];
	title?: string;
	description?: string;
	default?: unknown;
	enum?: unknown[];
}

interface ActionParameterSchema {
	properties?: Record<string, ActionSchemaProperty>;
	required?: string[];
}

const SUCCESSFUL_ACTION_STATUSES = new Set([
	"complete",
	"completed",
	"success",
	"succeeded",
	"applied",
]);

export function extractGraphErrorMessage(err: unknown): string {
	if (err instanceof Error) return err.message;
	if (typeof err === "string") return err;
	if (err && typeof err === "object") {
		const obj = err as Record<string, unknown>;
		if (typeof obj.error === "string") return obj.error;
		if (typeof obj.message === "string") return obj.message;
		try {
			return JSON.stringify(err);
		} catch {
			return String(err);
		}
	}
	return String(err);
}

function actionSucceeded(status: string): boolean {
	return SUCCESSFUL_ACTION_STATUSES.has(status.trim().toLowerCase());
}

function humanizeParameter(value: string): string {
	return value
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/[_-]+/g, " ")
		.replace(/\b\w/g, (character) => character.toUpperCase());
}

function toActionParameterSchema(
	schema?: Record<string, unknown>,
): ActionParameterSchema | undefined {
	if (!schema || typeof schema !== "object") return undefined;
	return schema as ActionParameterSchema;
}

function parameterType(property: ActionSchemaProperty): string {
	if (Array.isArray(property.type)) {
		return property.type.find((type) => type !== "null") ?? "string";
	}
	return property.type ?? "string";
}

function initialActionParameters(
	schema?: Record<string, unknown>,
): Record<string, unknown> {
	const definition = toActionParameterSchema(schema);
	const properties = definition?.properties ?? {};
	const required = new Set(definition?.required ?? []);
	return Object.fromEntries(
		Object.entries(properties).flatMap(([name, property]) => {
			if (property.default !== undefined) return [[name, property.default]];
			if (required.has(name) && parameterType(property) === "boolean") {
				return [[name, false]];
			}
			return [];
		}),
	);
}

export interface OntologyActionDialogProps {
	target: OntologyActionTarget | null;
	overlay: GraphOverlay | null;
	onClose: () => void;
	onInvoke: InvokeOntologyAction;
}

export const OntologyActionDialog: React.FC<OntologyActionDialogProps> = ({
	target,
	overlay,
	onClose,
	onInvoke,
}) => {
	const { t } = useTranslation("common");
	const action = target?.action ?? null;
	const node = target?.node ?? null;
	const [parameters, setParameters] = useState<Record<string, unknown>>({});
	const [submitting, setSubmitting] = useState(false);
	const [run, setRun] = useState<OntologyActionRun | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setParameters(initialActionParameters(action?.parameter_schema));
		setRun(null);
		setError(null);
		setSubmitting(false);
	}, [action]);

	const definition = toActionParameterSchema(action?.parameter_schema);
	const properties = definition?.properties ?? {};
	const required = new Set(definition?.required ?? []);
	const missingRequired = [...required].some((name) => {
		const value = parameters[name];
		return value === undefined || value === "" || value === null;
	});

	const mapping = overlay?.nodes.find(
		(candidate) => candidate.label === node?.label,
	);
	const titleProperty =
		overlay?.object_views?.find((view) =>
			mapping
				? view.object_type === mapping.id ||
					view.object_type === mapping.api_name ||
					view.object_type === mapping.label
				: false,
		)?.title_property ??
		mapping?.display_column ??
		mapping?.id_column;
	const targetTitle =
		(titleProperty && node?.props?.[titleProperty]) ??
		node?.caption ??
		node?.id;

	const succeeded = Boolean(run && actionSucceeded(run.status));

	const handleUpdate = useCallback((name: string, value: unknown) => {
		setParameters((current) => ({ ...current, [name]: value }));
	}, []);

	const handleInvoke = useCallback(async () => {
		if (!action || !node) return;
		setSubmitting(true);
		setRun(null);
		setError(null);
		try {
			const result = await onInvoke(action, node, parameters, (nextRun) =>
				setRun(nextRun),
			);
			setRun(result);
			if (!actionSucceeded(result.status)) {
				setError(
					result.error_message ??
						t('theActionEndedWithStatusVal', 'The action ended with status {{val}}.', { val: result.status.toLowerCase() }),
				);
			}
		} catch (invokeError) {
			setError(extractGraphErrorMessage(invokeError));
		} finally {
			setSubmitting(false);
		}
	}, [action, node, onInvoke, parameters]);

	return (
		<Dialog
			open={Boolean(target)}
			onOpenChange={(open) => {
				if (!open && !submitting) onClose();
			}}
		>
			<DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Workflow className="h-4 w-4 text-primary" />
						{action?.name ?? t('applyAction', 'Apply action')}
					</DialogTitle>
					<DialogDescription>
						{action?.description ??
							t('runThisGovernedOperationThroughItsSavedWorkflowBinding', 'Run this governed operation through its saved workflow binding.')}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-1" aria-busy={submitting}>
					<div className="rounded-lg border bg-muted/30 p-3">
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
							{t('target', 'Target')} {node?.label ?? "object"}
						</p>
						<p className="mt-1 font-medium">
							{String(targetTitle ?? "Object")}
						</p>
						<p className="mt-0.5 font-mono text-[10px] text-muted-foreground break-all">
							{node?.id}
						</p>
					</div>
					{Object.keys(properties).length > 0 && (
						<div className="space-y-3 rounded-lg border p-3">
							<Label>{t('parameters', 'Parameters')}</Label>
							{Object.entries(properties).map(([name, property]) => {
								const type = parameterType(property);
								const fieldId = `graph-action-${name}`;
								const fieldLabel = property.title ?? humanizeParameter(name);
								const isRequired = required.has(name);

								if (property.enum?.length) {
									return (
										<div key={name} className="grid gap-1.5">
											<Label htmlFor={fieldId}>
												{fieldLabel}
												{isRequired ? " *" : ""}
											</Label>
											<Select
												disabled={submitting}
												value={
													parameters[name] === undefined
														? undefined
														: String(parameters[name])
												}
												onValueChange={(value) =>
													handleUpdate(
														name,
														property.enum?.find(
															(option) => String(option) === value,
														) ?? value,
													)
												}
											>
												<SelectTrigger id={fieldId}>
													<SelectValue
														placeholder={t('chooseVal', 'Choose {{val}}', { val: fieldLabel.toLowerCase() })}
													/>
												</SelectTrigger>
												<SelectContent>
													{property.enum.map((option) => (
														<SelectItem
															key={String(option)}
															value={String(option)}
														>
															{String(option)}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
									);
								}

								if (type === "boolean") {
									return (
										<div
											key={name}
											className="flex items-center justify-between gap-4 rounded-md bg-muted/30 p-2.5"
										>
											<Label htmlFor={fieldId}>{fieldLabel}</Label>
											<Switch
												id={fieldId}
												disabled={submitting}
												checked={Boolean(parameters[name])}
												onCheckedChange={(checked) =>
													handleUpdate(name, checked)
												}
											/>
										</div>
									);
								}

								if (type === "array" || type === "object") {
									return (
										<div key={name} className="grid gap-1.5">
											<Label htmlFor={fieldId}>
												{fieldLabel}
												{isRequired ? " *" : ""}
											</Label>
											<Textarea
												id={fieldId}
												disabled={submitting}
												className="min-h-24 font-mono text-xs"
												defaultValue={JSON.stringify(
													parameters[name] ?? (type === "array" ? [] : {}),
													null,
													2,
												)}
												onChange={(event) => {
													try {
														handleUpdate(name, JSON.parse(event.target.value));
													} catch {
														// Keep the last valid value until the JSON parses.
													}
												}}
											/>
										</div>
									);
								}

								return (
									<div key={name} className="grid gap-1.5">
										<Label htmlFor={fieldId}>
											{fieldLabel}
											{isRequired ? " *" : ""}
										</Label>
										<Input
											id={fieldId}
											disabled={submitting}
											type={
												type === "integer" || type === "number"
													? "number"
													: "text"
											}
											value={String(parameters[name] ?? "")}
											onChange={(event) => {
												const value = event.target.value;
												handleUpdate(
													name,
													type === "integer"
														? value === ""
															? ""
															: Number.parseInt(value, 10)
														: type === "number"
															? value === ""
																? ""
																: Number.parseFloat(value)
															: value,
												);
											}}
											placeholder={property.description}
										/>
									</div>
								);
							})}
						</div>
					)}
					<div aria-live="polite">
						{run && succeeded && (
							<div className="rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-3 text-sm">
								{t('actionApplied', 'Action applied')}
								{run.run_id && (
									<span className="ml-1 font-mono text-[10px] text-muted-foreground">{t('runRun_id', 'Run {{run_id}}', { run_id: run.run_id })}</span>
								)}
							</div>
						)}
						{error && (
							<div
								role="alert"
								className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
							>
								{error}
							</div>
						)}
					</div>
				</div>
				<DialogFooter>
					<Button variant="ghost" onClick={onClose} disabled={submitting}>
						{run && succeeded ? "Done" : "Cancel"}
					</Button>
					{!succeeded && (
						<Button
							onClick={handleInvoke}
							disabled={submitting || missingRequired || !action || !node}
						>
							{submitting ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<Play className="h-4 w-4" />
							)}
							{t('confirm', 'Confirm')} {action?.name ?? "action"}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};
