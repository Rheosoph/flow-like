"use client";

import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import type {
	FormSessionActions,
	FormSessionState,
	Preset,
} from "../contracts";
import {
	type SaveBlock,
	type SaveRow,
	type SaveStart,
	blockOf,
	clashOf,
	draftOf,
	saveRowsOf,
} from "./preset-save-model";
import { secretCheck } from "./rail-model";

interface FormState {
	readonly name: string;
	readonly openDefault: boolean;
	readonly ticked: ReadonlySet<string>;
	readonly showDefaults: boolean;
	/** A press found something to fix: the message shows. */
	readonly attempted: boolean;
}

export interface SaveModelInput {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
	readonly start: SaveStart;
	readonly fromRunId: string | null;
	/** The proposed name, worded ("Preset 3"). */
	readonly initialName: string;
}

export interface SaveModel {
	readonly form: FormState;
	readonly rows: readonly SaveRow[];
	readonly clash: Preset | null;
	readonly block: SaveBlock | null;
	/** The block is worth saying now: after a press, or at once for a form with all its presets. */
	readonly showBlock: boolean;
	readonly nameRef: RefObject<HTMLInputElement | null>;
	readonly setName: (name: string) => void;
	readonly setOpenDefault: (on: boolean) => void;
	readonly toggleDefaults: () => void;
	readonly toggle: (name: string) => void;
	readonly submit: () => void;
}

/** The Save dialog's state and checks; the view only draws it. */
export function useSaveModel(input: SaveModelInput): SaveModel {
	const { state, actions, start, fromRunId, initialName } = input;
	const nameRef = useRef<HTMLInputElement | null>(null);
	const [form, setForm] = useState<FormState>(() => ({
		name: initialName,
		openDefault: start.openDefault,
		ticked: new Set(start.ticks),
		showDefaults: false,
		attempted: false,
	}));
	const { fields } = state.form;
	const { noSave } = state.memory.prefs;
	const rows = useMemo(
		() => saveRowsOf(fields, start.source, secretCheck(noSave)),
		[fields, noSave, start.source],
	);
	const updatingId = start.updating?.id ?? null;
	const clash = clashOf(state.memory.presets, form.name, updatingId);
	const replaceId = clash?.id ?? updatingId;
	const block = blockOf({
		name: form.name,
		ticked: form.ticked.size,
		presets: state.memory.presets.length,
		replaceId,
	});

	useEffect(() => {
		nameRef.current?.focus();
		nameRef.current?.select();
	}, []);

	const patch = (next: Partial<FormState>) =>
		setForm((current) => ({ ...current, ...next }));
	const toggle = (name: string) => {
		const ticked = new Set(form.ticked);
		if (!ticked.delete(name)) ticked.add(name);
		patch({ ticked });
	};
	const submit = () => {
		if (block !== null) {
			patch({ attempted: true });
			if (block === "name") nameRef.current?.focus();
			return;
		}
		actions.savePreset(draftOf({ ...form, rows, fromRunId, replaceId }));
		actions.closeOverlay();
	};
	return {
		form,
		rows,
		clash,
		block,
		showBlock: block !== null && (form.attempted || block === "limit"),
		nameRef,
		setName: (name) => patch({ name }),
		setOpenDefault: (openDefault) => patch({ openDefault }),
		toggleDefaults: () => patch({ showDefaults: !form.showDefaults }),
		toggle,
		submit,
	};
}
