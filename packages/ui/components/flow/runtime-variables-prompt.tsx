import { useTranslation } from "@flow-like/locales";
import { AlertCircleIcon, CheckIcon, KeyIcon, SaveIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { IVariable } from "../../lib/schema/flow/board";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import type { RuntimeVariableValue } from "../../state/runtime-variables-context";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Label } from "../ui/label";
import {
	RuntimeVariableEditor,
	seedRuntimeVariable,
} from "./variables/runtime-variable-editor";

export interface RuntimeVariablesPromptProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	variables: IVariable[];
	existingValues: Map<string, RuntimeVariableValue>;
	onSave: (values: RuntimeVariableValue[]) => Promise<void>;
	onCancel: () => void;
	refs?: Record<string, string>;
}

/**
 * Whether a runtime variable currently holds a usable value. Booleans, numbers
 * and structured values count as configured as soon as they decode; strings
 * (including paths) must be non-empty.
 */
function isRuntimeVariableConfigured(variable: IVariable): boolean {
	const decoded = parseUint8ArrayToJson(variable.default_value);
	if (decoded === undefined || decoded === null) return false;
	if (typeof decoded === "string") return decoded.trim().length > 0;
	return true;
}

/**
 * A dialog that prompts the user to configure missing runtime variables
 * before executing a flow. Each variable is rendered with the editor that
 * matches its data type (path picker, date picker, number field, …).
 */
export function RuntimeVariablesPrompt({
	open,
	onOpenChange,
	variables,
	existingValues,
	onSave,
	onCancel,
	refs,
}: RuntimeVariablesPromptProps) {
	const { t } = useTranslation("flow");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<KeyIcon className="w-5 h-5" />
						{t("configureRuntimeVariables", "Configure Runtime Variables")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"thisFlowRequiresRuntimeVariablesToBeConfiguredBeforeExecutionTheseValuesAreStoredLocallyAndNeverUploaded",
							"This flow requires runtime variables to be configured before execution. These values are stored locally and never uploaded.",
						)}
					</DialogDescription>
				</DialogHeader>

				<RuntimeVariablesForm
					variables={variables}
					existingValues={existingValues}
					onSave={onSave}
					onCancel={onCancel}
					refs={refs}
				/>
			</DialogContent>
		</Dialog>
	);
}

function RuntimeVariablesForm({
	variables,
	existingValues,
	onSave,
	onCancel,
	refs,
}: Readonly<{
	variables: IVariable[];
	existingValues: Map<string, RuntimeVariableValue>;
	onSave: (values: RuntimeVariableValue[]) => Promise<void>;
	onCancel: () => void;
	refs?: Record<string, string>;
}>) {
	const { t } = useTranslation("flow");
	const [values, setValues] = useState<Map<string, IVariable>>(() => {
		const map = new Map<string, IVariable>();
		for (const variable of variables) {
			map.set(
				variable.id,
				seedRuntimeVariable(variable, existingValues.get(variable.id)?.value),
			);
		}
		return map;
	});
	const [isSaving, setIsSaving] = useState(false);

	const missingCount = useMemo(() => {
		return variables.filter((variable) => {
			const current = values.get(variable.id) ?? variable;
			return !isRuntimeVariableConfigured(current);
		}).length;
	}, [variables, values]);

	const canSave = missingCount === 0;

	const updateVariable = useCallback(async (next: IVariable) => {
		setValues((prev) => {
			const updated = new Map(prev);
			updated.set(next.id, next);
			return updated;
		});
	}, []);

	const handleSave = useCallback(async () => {
		if (!canSave) return;
		setIsSaving(true);
		try {
			const result: RuntimeVariableValue[] = [];
			for (const variable of variables) {
				const current = values.get(variable.id) ?? variable;
				const bytes = current.default_value;
				if (bytes && bytes.length > 0) {
					result.push({ variableId: variable.id, value: bytes });
				}
			}
			await onSave(result);
		} finally {
			setIsSaving(false);
		}
	}, [canSave, variables, values, onSave]);

	return (
		<>
			<div className="space-y-4 py-4">
				{variables.map((variable) => {
					const current = values.get(variable.id) ?? variable;
					const isConfigured = isRuntimeVariableConfigured(current);

					return (
						<Card key={variable.id} className="p-4">
							<div className="flex flex-col gap-2">
								<div className="flex items-center justify-between">
									<Label className="text-sm font-medium flex items-center gap-2">
										{variable.name}
										{variable.secret && (
											<Badge variant="secondary" className="text-xs gap-1">
												<KeyIcon className="w-3 h-3" />
												{t("secret", "Secret")}
											</Badge>
										)}
									</Label>
									{isConfigured && (
										<CheckIcon className="w-4 h-4 text-green-500" />
									)}
								</div>

								{variable.description && (
									<p className="text-xs text-muted-foreground">
										{variable.description}
									</p>
								)}

								<RuntimeVariableEditor
									variable={current}
									updateVariable={updateVariable}
									refs={refs}
								/>
							</div>
						</Card>
					);
				})}
			</div>

			{missingCount > 0 && (
				<div className="flex items-center gap-2 text-amber-500 text-sm">
					<AlertCircleIcon className="w-4 h-4" />
					{t("runtimeVariablesStillNeedToBeConfigured", {
						defaultValue_one: "{{count}} variable still needs to be configured",
						defaultValue_other:
							"{{count}} variables still need to be configured",
						count: missingCount,
					})}
				</div>
			)}

			<DialogFooter className="gap-2">
				<Button variant="outline" onClick={onCancel}>
					{t("cancel", "Cancel")}
				</Button>
				<Button
					onClick={handleSave}
					disabled={!canSave || isSaving}
					className="gap-2"
				>
					<SaveIcon className="w-4 h-4" />
					{t("saveContinue", "Save & Continue")}
				</Button>
			</DialogFooter>
		</>
	);
}
