"use client";

import { i18n as i18next } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	CircleDotIcon,
	EllipsisVerticalIcon,
	GripIcon,
	ListIcon,
} from "lucide-react";
import { memo, useCallback, useState } from "react";
import { useBoardFormat } from "../../../hooks/use-board-format";
import { GEOMETRY_BOARD_FORMAT_VERSION } from "../../../lib/board-format";
import { encodedTypeDefault } from "../../../lib/flow-defaults";
import type { IVariable } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
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
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { typeToColor } from "../utils";
import { GeometrySubtypeSelect } from "./geometry-variable";

interface NewVariableDialogProps {
	appId?: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onCreateVariable: (variable: IVariable) => Promise<void>;
}

const TypePreview = memo(({ type }: { type: IVariableType }) => (
	<div className="flex items-center gap-2">
		<div
			className="size-2 rounded-full"
			style={{ backgroundColor: typeToColor(type) }}
		/>
		<span>{type}</span>
	</div>
));

TypePreview.displayName = "TypePreview";

const ValueTypePreview = memo(
	({
		valueType,
		dataType,
	}: { valueType: IValueType; dataType: IVariableType }) => {
		const color = typeToColor(dataType);
		const iconClass = "w-3.5 h-3.5";

		const getIcon = () => {
			switch (valueType) {
				case IValueType.Normal:
					return <CircleDotIcon className={iconClass} style={{ color }} />;
				case IValueType.Array:
					return <GripIcon className={iconClass} style={{ color }} />;
				case IValueType.HashSet:
					return (
						<EllipsisVerticalIcon className={iconClass} style={{ color }} />
					);
				case IValueType.HashMap:
					return <ListIcon className={iconClass} style={{ color }} />;
				default:
					return null;
			}
		};

		const getLabel = () => {
			switch (valueType) {
				case IValueType.Normal:
					return "Single";
				case IValueType.Array:
					return "Array";
				case IValueType.HashSet:
					return "Set";
				case IValueType.HashMap:
					return "Map";
				default:
					return valueType;
			}
		};

		return (
			<div className="flex items-center gap-2">
				{getIcon()}
				<span>{getLabel()}</span>
			</div>
		);
	},
);

ValueTypePreview.displayName = "ValueTypePreview";

const NewVariableDialog = memo(
	({ open, onOpenChange, onCreateVariable, appId }: NewVariableDialogProps) => {
		const geometryEnabled =
			useBoardFormat(appId) >= GEOMETRY_BOARD_FORMAT_VERSION;
		const [name, setName] = useState("New Variable");
		const [dataType, setDataType] = useState<IVariableType>(
			IVariableType.String,
		);
		const [valueType, setValueType] = useState<IValueType>(IValueType.Normal);
		const [geometrySchema, setGeometrySchema] = useState<string | null>(null);
		const [category, setCategory] = useState("");
		const [isCreating, setIsCreating] = useState(false);

		const handleCreate = useCallback(async () => {
			if (
				!name.trim() ||
				(dataType === IVariableType.Geometry && !geometryEnabled)
			)
				return;

			setIsCreating(true);
			try {
				const variable: IVariable = {
					id: createId(),
					name: name.trim(),
					data_type: dataType,
					value_type: valueType,
					exposed: false,
					secret: false,
					editable: true,
					category: category.trim() || undefined,
					default_value: encodedTypeDefault(valueType, dataType),
					description: "",
					schema: dataType === IVariableType.Geometry ? geometrySchema : null,
				};

				await onCreateVariable(variable);
				onOpenChange(false);

				// Reset form
				setName("New Variable");
				setDataType(IVariableType.String);
				setValueType(IValueType.Normal);
				setCategory("");
				setGeometrySchema(null);
			} finally {
				setIsCreating(false);
			}
		}, [
			name,
			dataType,
			valueType,
			category,
			geometrySchema,
			geometryEnabled,
			onCreateVariable,
			onOpenChange,
		]);

		return (
			<Dialog open={open} onOpenChange={onOpenChange}>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<DialogTitle>
							{i18next.t("createNewVariable", "Create New Variable")}
						</DialogTitle>
						<DialogDescription>
							{`Define a new variable for your flow.`}
						</DialogDescription>
					</DialogHeader>

					<div className="grid gap-4 py-4">
						<div className="grid gap-2">
							<Label htmlFor="var-name">Name</Label>
							<Input
								id="var-name"
								value={name}
								onChange={(e) => setName(e.target.value)}
								placeholder={i18next.t("variableName2", "Variable name...")}
								autoFocus
							/>
						</div>

						<div className="grid gap-2">
							<Label htmlFor="var-category">
								{i18next.t("categoryOptional", "Category (optional)")}
							</Label>
							<Input
								id="var-category"
								value={category}
								onChange={(e) => setCategory(e.target.value)}
								placeholder={i18next.t(
									"egConfigsettings",
									"e.g. Config/Settings",
								)}
							/>
							<p className="text-xs text-muted-foreground">
								{i18next.t(
									"useToCreateNestedFolders",
									'Use "/" to create nested folders',
								)}
							</p>
						</div>

						<div className="grid grid-cols-2 gap-4">
							<div className="grid gap-2">
								<Label htmlFor="var-data-type">
									{i18next.t("dataType", "Data Type")}
								</Label>
								<Select
									value={dataType}
									onValueChange={(value) => setDataType(value as IVariableType)}
								>
									<SelectTrigger id="var-data-type">
										<SelectValue
											placeholder={i18next.t("selectType", "Select type")}
										/>
									</SelectTrigger>
									<SelectContent>
										<SelectGroup>
											<SelectLabel>
												{i18next.t("dataType", "Data Type")}
											</SelectLabel>
											<SelectItem value="Boolean">
												<TypePreview type={IVariableType.Boolean} />
											</SelectItem>
											<SelectItem value="Date">
												<TypePreview type={IVariableType.Date} />
											</SelectItem>
											<SelectItem value="Float">
												<TypePreview type={IVariableType.Float} />
											</SelectItem>
											<SelectItem value="Integer">
												<TypePreview type={IVariableType.Integer} />
											</SelectItem>
											<SelectItem value="String">
												<TypePreview type={IVariableType.String} />
											</SelectItem>
											<SelectItem value="PathBuf">
												<TypePreview type={IVariableType.PathBuf} />
											</SelectItem>
											<SelectItem value="Geometry" disabled={!geometryEnabled}>
												<TypePreview type={IVariableType.Geometry} />
											</SelectItem>
											<SelectItem value="Struct">
												<TypePreview type={IVariableType.Struct} />
											</SelectItem>
										</SelectGroup>
									</SelectContent>
								</Select>
							</div>

							<div className="grid gap-2">
								<Label htmlFor="var-value-type">
									{i18next.t("valueType", "Value Type")}
								</Label>
								<Select
									value={valueType}
									onValueChange={(value) => setValueType(value as IValueType)}
								>
									<SelectTrigger id="var-value-type">
										<SelectValue
											placeholder={i18next.t("selectType", "Select type")}
										/>
									</SelectTrigger>
									<SelectContent>
										<SelectGroup>
											<SelectLabel>
												{i18next.t("valueType", "Value Type")}
											</SelectLabel>
											<SelectItem value="Normal">
												<ValueTypePreview
													valueType={IValueType.Normal}
													dataType={dataType}
												/>
											</SelectItem>
											<SelectItem value="Array">
												<ValueTypePreview
													valueType={IValueType.Array}
													dataType={dataType}
												/>
											</SelectItem>
											<SelectItem value="HashSet">
												<ValueTypePreview
													valueType={IValueType.HashSet}
													dataType={dataType}
												/>
											</SelectItem>
											<SelectItem value="HashMap">
												<ValueTypePreview
													valueType={IValueType.HashMap}
													dataType={dataType}
												/>
											</SelectItem>
										</SelectGroup>
									</SelectContent>
								</Select>
							</div>
						</div>
					</div>

					{dataType === IVariableType.Geometry && (
						<GeometrySubtypeSelect
							schema={geometrySchema}
							onChange={setGeometrySchema}
							disabled={isCreating}
						/>
					)}

					{!geometryEnabled && (
						<p className="text-xs text-muted-foreground">
							{i18next.t(
								"flow:geometryBackendUpgrade",
								"Update the backend to a version that supports Geometry, then reopen this editor.",
							)}
						</p>
					)}
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => onOpenChange(false)}
							disabled={isCreating}
						>
							{i18next.t("cancel", "Cancel")}
						</Button>
						<Button
							onClick={handleCreate}
							disabled={!name.trim() || isCreating}
						>
							{isCreating
								? "Creating..."
								: i18next.t("createVariable", "Create Variable")}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		);
	},
);

NewVariableDialog.displayName = "NewVariableDialog";

export { NewVariableDialog };
