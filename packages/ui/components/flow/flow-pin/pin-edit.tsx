"use client";

import { VariableIcon } from "lucide-react";
import {
	type FC,
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useState,
} from "react";
import { Button } from "../../../components/ui/button";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { IPin } from "../../../lib/schema/flow/pin";
import useFlowControlState from "../../../state/flow-control-state";
import type { FlowSelectorDataRef } from "../flow-selector-data";
import { resolvePinEditorKind } from "./pin-editor-kind";
import { BitVariable } from "./variable-types/bit-select";
import { BooleanVariable } from "./variable-types/boolean-variable";
import { VariableDescription } from "./variable-types/default-text";
import { ElementSelect } from "./variable-types/element-select";
import { EnumVariable } from "./variable-types/enum-variable";
import { FnVariable } from "./variable-types/fn-select";
import {
	OntologyActionSelect,
	OntologyObjectSelect,
	OntologySelect,
	RemoteOntologyActionSelect,
	RemoteOntologyObjectSelect,
	RemoteOntologySelect,
} from "./variable-types/ontology-pin-selects";
import { ProjectUserSelect } from "./variable-types/project-user-select";
import { RemoteDatabaseSelect } from "./variable-types/remote-database-select";
import { RemoteEventSelect } from "./variable-types/remote-event-select";
import { RemoteProjectSelect } from "./variable-types/remote-project-select";
import { TaxCodeSelect } from "./variable-types/tax-code-select";
import { VarVariable } from "./variable-types/var-select";
import { WidgetVariable } from "./variable-types/widget-select";

type PinDefaultValue = IPin["default_value"];

interface PinEditProps {
	readonly nodeId: string;
	readonly nodeName?: string;
	readonly pin: IPin;
	readonly defaultValue: PinDefaultValue;
	readonly appId: string;
	readonly boardId?: string;
	readonly boardRef?: RefObject<IBoard | undefined>;
	readonly changeDefaultValue: (value: PinDefaultValue) => void;
	readonly saveDefaultValue: (value: PinDefaultValue) => Promise<void>;
	readonly currentLayerId?: string;
	readonly selectorDataRef?: FlowSelectorDataRef;
	readonly selectorDataVersion?: number;
}

export const PinEdit: FC<PinEditProps> = memo(function PinEdit({
	nodeId,
	nodeName,
	pin,
	defaultValue,
	appId,
	boardId,
	boardRef,
	changeDefaultValue,
	saveDefaultValue,
	currentLayerId,
	selectorDataRef,
}: PinEditProps) {
	const [cachedDefaultValue, setCachedDefaultValue] = useState(defaultValue);

	// Sync cached value when prop changes (e.g., after board refetch)
	useEffect(() => {
		setCachedDefaultValue(defaultValue);
	}, [defaultValue]);

	const updateDefaultValue = useCallback(
		async (value: unknown) => {
			const nextValue = value as PinDefaultValue;
			setCachedDefaultValue(nextValue);
			changeDefaultValue(nextValue);
			await saveDefaultValue(nextValue);
		},
		[changeDefaultValue, saveDefaultValue],
	);

	const previewDefaultValue = useCallback(
		(value: PinDefaultValue) => {
			setCachedDefaultValue(value);
			changeDefaultValue(value);
		},
		[changeDefaultValue],
	);

	const ontologyProps = {
		pin,
		value: cachedDefaultValue,
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue: updateDefaultValue,
	} as const;

	switch (resolvePinEditorKind(pin, nodeName)) {
		case "label":
			return <VariableDescription pin={pin} />;
		case "projectUser":
			return (
				<ProjectUserSelect
					pin={pin}
					value={cachedDefaultValue}
					appId={appId}
					setValue={updateDefaultValue}
				/>
			);
		case "remoteProject":
			return (
				<RemoteProjectSelect
					pin={pin}
					value={cachedDefaultValue}
					appId={appId}
					boardId={boardId}
					nodeId={nodeId}
					boardRef={boardRef}
					setValue={updateDefaultValue}
					onPreviewValue={previewDefaultValue}
				/>
			);
		case "remoteDatabase":
			return (
				<RemoteDatabaseSelect
					pin={pin}
					value={cachedDefaultValue}
					appId={appId}
					nodeId={nodeId}
					boardRef={boardRef}
					setValue={updateDefaultValue}
				/>
			);
		case "remoteEvent":
			return (
				<RemoteEventSelect
					pin={pin}
					value={cachedDefaultValue}
					appId={appId}
					boardId={boardId}
					nodeId={nodeId}
					nodeName={nodeName}
					boardRef={boardRef}
					setValue={updateDefaultValue}
					onPreviewValue={previewDefaultValue}
				/>
			);
		case "ontology":
			return <OntologySelect {...ontologyProps} />;
		case "ontologyObject":
			return <OntologyObjectSelect {...ontologyProps} />;
		case "ontologyAction":
			return <OntologyActionSelect {...ontologyProps} />;
		case "remoteOntology":
			return <RemoteOntologySelect {...ontologyProps} />;
		case "remoteOntologyObject":
			return <RemoteOntologyObjectSelect {...ontologyProps} />;
		case "remoteOntologyAction":
			return <RemoteOntologyActionSelect {...ontologyProps} />;
		case "widget":
			return (
				<WidgetVariable
					pin={pin}
					value={cachedDefaultValue}
					appId={appId}
					setValue={updateDefaultValue}
				/>
			);
		case "taxCode":
			return (
				<TaxCodeSelect
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
				/>
			);
		case "boolean":
			return (
				<BooleanVariable
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
				/>
			);
		case "enum":
			return (
				<EnumVariable
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
				/>
			);
		case "bit":
			return (
				<BitVariable
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
					selectorDataRef={selectorDataRef}
				/>
			);
		case "fn":
			return (
				<FnVariable
					boardRef={boardRef}
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
				/>
			);
		case "var":
			return (
				<VarVariable
					boardRef={boardRef}
					pin={pin}
					value={cachedDefaultValue}
					currentLayerId={currentLayerId}
					setValue={updateDefaultValue}
				/>
			);
		case "element":
			return (
				<ElementSelect
					pin={pin}
					value={cachedDefaultValue}
					setValue={updateDefaultValue}
					selectorDataRef={selectorDataRef}
				/>
			);
		default:
			return (
				<WithMenu nodeId={nodeId} pin={pin} defaultValue={cachedDefaultValue} />
			);
	}
});

function WithMenuInner({
	nodeId,
	pin,
	defaultValue,
}: Readonly<{
	nodeId: string;
	pin: IPin;
	defaultValue: number[] | undefined | null;
}>) {
	const { editPin } = useFlowControlState();
	const isConnected = pin.connected_to && pin.connected_to.length > 0;
	const hasNoDefaultValue =
		typeof defaultValue === "undefined" || defaultValue === null;

	return (
		<>
			<VariableDescription pin={pin} />
			{!isConnected && (
				<Button
					size={"icon"}
					variant={"ghost"}
					className="w-fit h-fit text-foreground"
					onClick={() => {
						editPin(nodeId, pin);
					}}
				>
					<VariableIcon
						className={`size-[0.45rem] ${hasNoDefaultValue && "text-primary"}`}
					/>
				</Button>
			)}
		</>
	);
}

const WithMenu = memo(WithMenuInner) as typeof WithMenuInner;
