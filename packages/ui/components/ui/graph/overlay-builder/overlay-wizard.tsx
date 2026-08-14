"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useState } from "react";
import type {
	CreateOverlayPayload,
	EdgeLabelMapping,
	NodeLabelMapping,
	PropertyColumn,
	ValidationResult,
} from "../../../../state/backend-state/graph-state";
import { Button } from "../../button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../dialog";
import { Input } from "../../input";
import { Label } from "../../label";
import { StepEdges } from "./step-edges";
import { StepNodes } from "./step-nodes";
import { StepReview } from "./step-review";
import { StepTables, type TableInfo } from "./step-tables";

const STEPS = ["Setup", "Nodes", "Edges", "Review"] as const;
type StepName = (typeof STEPS)[number];

export interface OverlayWizardProps {
	open: boolean;
	onClose: () => void;
	onSubmit: (payload: CreateOverlayPayload) => Promise<void>;
	tables: TableInfo[];
	tableColumns: Record<string, PropertyColumn[]>;
	validation?: ValidationResult | null;
	onValidate?: () => void;
	submitting?: boolean;
}

export function OverlayWizard({
	open,
	onClose,
	onSubmit,
	tables,
	tableColumns,
	validation,
	onValidate,
	submitting,
}: OverlayWizardProps) {
	const { t } = useTranslation("common");
	const [step, setStep] = useState<number>(0);
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [defaultLimit, setDefaultLimit] = useState(500);
	const [selectedTables, setSelectedTables] = useState<Set<string>>(new Set());
	const [nodes, setNodes] = useState<NodeLabelMapping[]>([]);
	const [edges, setEdges] = useState<EdgeLabelMapping[]>([]);

	const selectedTableNames = useMemo(
		() =>
			Array.from(selectedTables).map((k) =>
				k.startsWith("user:") ? k.slice(5) : k,
			),
		[selectedTables],
	);

	const filteredTableColumns = useMemo(() => {
		const result: Record<string, PropertyColumn[]> = {};
		for (const t of selectedTableNames) {
			if (tableColumns[t]) {
				result[t] = tableColumns[t];
			}
		}
		return result;
	}, [selectedTableNames, tableColumns]);

	// Reset on close
	useEffect(() => {
		if (!open) {
			setStep(0);
			setName("");
			setDescription("");
			setDefaultLimit(500);
			setSelectedTables(new Set());
			setNodes([]);
			setEdges([]);
		}
	}, [open]);

	// Trigger validation when reaching review step
	useEffect(() => {
		if (step === STEPS.length - 1 && onValidate) {
			onValidate();
		}
	}, [step, onValidate]);

	const toggleTable = useCallback((key: string) => {
		setSelectedTables((prev) => {
			const next = new Set(prev);
			if (next.has(key)) {
				next.delete(key);
			} else {
				next.add(key);
			}
			return next;
		});
	}, []);

	const canNext = useMemo(() => {
		const stepName = STEPS[step];
		switch (stepName) {
			case "Setup":
				return name.trim().length > 0 && selectedTables.size > 0;
			case "Nodes":
				return (
					nodes.length > 0 &&
					nodes.every((n) => n.label && n.table && n.id_column)
				);
			case "Edges":
				return true; // edges are optional
			case "Review":
				return true;
			default:
				return true;
		}
	}, [step, name, selectedTables, nodes]);

	const handleSubmit = useCallback(async () => {
		const payload: CreateOverlayPayload = {
			name: name.trim(),
			description: description.trim() || undefined,
			nodes,
			edges,
			default_limit: defaultLimit,
		};
		await onSubmit(payload);
	}, [name, description, nodes, edges, defaultLimit, onSubmit]);

	const currentStep = STEPS[step];

	return (
		<Dialog open={open} onOpenChange={(v) => !v && onClose()}>
			<DialogContent className="max-w-2xl max-h-[85vh] flex flex-col overflow-hidden">
				<DialogHeader>
					<DialogTitle>{t('createGraphOverlay', 'Create Graph Overlay')}</DialogTitle>
					<DialogDescription>
						{t('stepOf', 'Step {{step}} of {{total}}: {{currentStep}}', { step: step + 1, total: STEPS.length, currentStep })}</DialogDescription>
				</DialogHeader>

				{/* Step indicator */}
				<div className="flex gap-1 mb-2">
					{STEPS.map((s, i) => (
						<div
							key={s}
							className={`h-1 flex-1 rounded-full transition-colors ${
								i <= step ? "bg-primary" : "bg-muted"
							}`}
						/>
					))}
				</div>

				{/* Step content */}
				<div className="min-h-50 flex-1 overflow-y-auto pr-1">
					{currentStep === "Setup" && (
						<div className="space-y-6">
							<div className="space-y-4">
								<div>
									<h3 className="text-sm font-medium mb-1">{t('overlayDetails', 'Overlay Details')}</h3>
									<p className="text-xs text-muted-foreground">
										{t('nameTheOverlaySetItsDefaultQueryLimitAndPickTheTablesYouWantToMap', "Name the overlay, set its default query limit, and pick the tables you want to map.")}
									</p>
								</div>
								<div className="space-y-1.5">
									<Label className="text-sm">Name</Label>
									<Input
										value={name}
										onChange={(e) => setName(e.target.value)}
										placeholder={t('myGraphOverlay', 'My Graph Overlay')}
										autoFocus
									/>
								</div>
								<div className="space-y-1.5">
									<Label className="text-sm">{t('description', 'Description')}</Label>
									<Input
										value={description}
										onChange={(e) => setDescription(e.target.value)}
										placeholder={t('optionalDescription', 'Optional description...')}
									/>
								</div>
								<div className="space-y-1.5">
									<Label className="text-sm">{t('defaultQueryLimit', 'Default Query Limit')}</Label>
									<Input
										type="number"
										value={defaultLimit}
										onChange={(e) => setDefaultLimit(Number(e.target.value))}
										min={1}
										max={10000}
									/>
								</div>
							</div>

							<StepTables
								tables={tables}
								selected={selectedTables}
								onToggle={toggleTable}
							/>
						</div>
					)}
					{currentStep === "Nodes" && (
						<StepNodes
							nodes={nodes}
							tables={selectedTableNames}
							tableColumns={filteredTableColumns}
							onChange={setNodes}
						/>
					)}
					{currentStep === "Edges" && (
						<StepEdges
							edges={edges}
							nodes={nodes}
							tables={selectedTableNames}
							tableColumns={filteredTableColumns}
							onChange={setEdges}
						/>
					)}
					{currentStep === "Review" && (
						<StepReview
							name={name}
							description={description}
							nodes={nodes}
							edges={edges}
							defaultLimit={defaultLimit}
							validation={validation ?? null}
						/>
					)}
				</div>

				<DialogFooter className="flex-row justify-between sm:justify-between gap-2 shrink-0 pt-4 border-t">
					<div>
						{step > 0 && (
							<Button variant="outline" onClick={() => setStep(step - 1)}>
								{t('back', 'Back')}
							</Button>
						)}
					</div>
					<div className="flex gap-2">
						<Button variant="ghost" onClick={onClose}>
							{t('cancel', 'Cancel')}
						</Button>
						{step < STEPS.length - 1 ? (
							<Button onClick={() => setStep(step + 1)} disabled={!canNext}>
								{t('next', 'Next')}
							</Button>
						) : (
							<Button onClick={handleSubmit} disabled={submitting}>
								{submitting ? "Creating..." : t('createOverlay', 'Create Overlay')}
							</Button>
						)}
					</div>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
