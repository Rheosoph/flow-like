import { createId } from "@paralleldrive/cuid2";
import type {
	IMoveToLayer,
	INode,
	IRemoveLayer,
	IUpsertLayer,
} from "../schema";
import type { IAddNode } from "../schema/flow/board/commands/add-node";
import type { IConnectPins } from "../schema/flow/board/commands/connect-pins";
import type { ICopyPaste } from "../schema/flow/board/commands/copy-paste";
import type { IDisconnectPins } from "../schema/flow/board/commands/disconnect-pins";
import {
	ICommandType,
	type IGenericCommand,
} from "../schema/flow/board/commands/generic-command";
import type { IMoveNode } from "../schema/flow/board/commands/move-node";
import type { IRemoveComment } from "../schema/flow/board/commands/remove-comment";
import type { IRemoveNode } from "../schema/flow/board/commands/remove-node";
import type { IRemoveVariable } from "../schema/flow/board/commands/remove-variable";
import type { IUpdateNode } from "../schema/flow/board/commands/update-node";
import type { IUpsertComment } from "../schema/flow/board/commands/upsert-comment";
import type { IUpsertPin } from "../schema/flow/board/commands/upsert-pin";
import type { IUpsertVariable } from "../schema/flow/board/commands/upsert-variable";

export function addNodeCommand(command: IAddNode): {
	command: IGenericCommand;
	node: INode;
} {
	command.node.id = createId();

	const pin_ids = Object.keys(command.node.pins);
	for (const pin of pin_ids) {
		const newId = createId();
		command.node.pins[newId] = command.node.pins[pin];
		command.node.pins[newId].id = newId;
		delete command.node.pins[pin];
	}

	const generic_command = {
		...command,
		command_type: ICommandType.AddNode,
	};

	return {
		command: generic_command as any,
		node: command.node,
	};
}

export function removeCommentCommand(command: IRemoveComment): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.RemoveComment,
	};

	return generic_command as any;
}

export function upsertCommentCommand(command: IUpsertComment): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.UpsertComment,
	};

	return generic_command as any;
}

export function copyPasteCommand(command: ICopyPaste): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.CopyPaste,
	};

	return generic_command as any;
}

export function moveNodeCommand(command: IMoveNode): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.MoveNode,
	};

	return generic_command as any;
}

export function removeNodeCommand(command: IRemoveNode): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.RemoveNode,
	};

	return generic_command as any;
}

export function updateNodeCommand(command: IUpdateNode): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.UpdateNode,
	};

	return generic_command as any;
}

export function connectPinsCommand(command: IConnectPins): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.ConnectPin,
	};

	return generic_command as any;
}

export function disconnectPinsCommand(
	command: IDisconnectPins,
): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.DisconnectPin,
	};

	return generic_command as any;
}

export function upsertPinCommand(command: IUpsertPin): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.UpsertPin,
	};

	return generic_command as any;
}

export function upsertVariableCommand(
	command: IUpsertVariable,
): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.UpsertVariable,
	};

	return generic_command as any;
}

export function removeVariableCommand(
	command: IRemoveVariable,
): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.RemoveVariable,
	};

	return generic_command as any;
}

export function removeLayerCommand(command: IRemoveLayer): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.RemoveLayer,
	};

	return generic_command as any;
}

export function upsertLayerCommand(command: IUpsertLayer): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.UpsertLayer,
	};

	return generic_command as any;
}

export function moveToLayerCommand(command: IMoveToLayer): IGenericCommand {
	const generic_command = {
		...command,
		command_type: ICommandType.MoveToLayer,
	};

	return generic_command as any;
}
