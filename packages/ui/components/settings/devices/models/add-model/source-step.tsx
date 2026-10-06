"use client";

import { useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import { Boxes, FolderHeart } from "lucide-react";
import { type ReactNode, useState } from "react";
import { bitModelName } from "../../../../../lib/bit/model-display-name";
import type { IBit } from "../../../../../lib/schema";
import { bytesText } from "../../observe/observe-data";
import type { DevicesT } from "../../primitives/area-context";
import {
	ChoiceCards,
	type ChoiceOption,
	DvInput,
	Field,
} from "../../primitives/form-fields";
import { InlineResult } from "../../primitives/inline-result";
import { type SegmentOption, Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { engineLabel, kindLabel } from "../models-copy";
import { type ModelSource, engineOfBit, kindOfBit } from "./model-options";
import type { SourcePick } from "./use-add-model";

/** Rows shown before the search narrows them. */
const SHOWN = 25;

function sourceOptions(t: DevicesT): SegmentOption<ModelSource>[] {
	return [
		{
			value: "hub",
			icon: Boxes,
			label: t("devices:models.install.source.hub", "Flow-Like hub"),
		},
		{
			value: "huggingface",
			label: t("devices:models.install.source.huggingFace", "Hugging Face"),
		},
		{
			value: "bits",
			icon: FolderHeart,
			label: t("devices:models.install.source.bits", "My Bits"),
		},
	];
}

/** "Chat · llama.cpp · 4.7 GiB". */
function bitHint(t: DevicesT, bit: IBit) {
	const kind = kindOfBit(bit);
	const engine = engineOfBit(bit);
	const parts = [
		kind ? kindLabel(t, kind) : "",
		engine ? engineLabel(t, engine) : "",
		bit.size ? bytesText(bit.size) : "",
	];
	return parts.filter(Boolean).join(" · ");
}

const bitName = (bit: IBit) => bitModelName(bit) ?? bit.id;

function matches(bit: IBit, query: string) {
	const words = query.toLowerCase().split(/\s+/u).filter(Boolean);
	const text =
		`${bitName(bit)} ${bit.meta?.en?.description ?? ""}`.toLowerCase();
	return words.every((word) => text.includes(word));
}

interface BitListProps {
	id: string;
	legend: string;
	query: UseQueryResult<IBit[]>;
	selected?: string;
	onSelect(bit: IBit): void;
	/** The Bits that match the search. */
	rows: readonly IBit[];
	/** There is no Bit to offer. */
	empty: string;
	/** There are Bits, but none matches; `empty` when unset. */
	noMatch?: string;
}

function BitList(props: Readonly<BitListProps>) {
	const { t } = useTranslation("devices");
	const { query, rows } = props;
	if (query.isLoading)
		return (
			<StateView
				kind="loading"
				rows={4}
				title={t("devices:models.install.source.loading", "Loading models…")}
			/>
		);
	if (query.error)
		return (
			<InlineResult tone="critical">
				{t(
					"devices:models.install.source.listFailed",
					"The list couldn't be loaded: {{reason}}",
					{ reason: query.error.message },
				)}
			</InlineResult>
		);
	if (!rows.length) {
		const none = !query.data?.length;
		return (
			<StateView
				kind="empty"
				title={none ? props.empty : (props.noMatch ?? props.empty)}
			/>
		);
	}
	const options: ChoiceOption<string>[] = [];
	for (const bit of rows.slice(0, SHOWN))
		options.push({ value: bit.id, title: bitName(bit), hint: bitHint(t, bit) });
	return (
		<ChoiceCards
			id={props.id}
			legend={props.legend}
			value={props.selected}
			onValueChange={(id) => {
				const bit = rows.find((row) => row.id === id);
				if (bit) props.onSelect(bit);
			}}
			options={options}
		/>
	);
}

interface ListsProps {
	hub: UseQueryResult<IBit[]>;
	bits: UseQueryResult<IBit[]>;
}

export interface SourceStepProps extends ListsProps {
	pick: SourcePick;
	onPick(pick: SourcePick): void;
	/** Why the last lookup failed, in the source's own words. */
	error?: string;
	/** Enter in the Hugging Face field looks the repository up. */
	onSubmit(): void;
}

type SearchedBitsProps = Omit<BitListProps, "rows"> & { searchLabel: string };

/** A search over the Bits: the first `SHOWN` matches are offered, and the field says when more match. */
function SearchedBits({ searchLabel, ...list }: Readonly<SearchedBitsProps>) {
	const { t } = useTranslation("devices");
	const [filter, setFilter] = useState("");
	const rows = (list.query.data ?? []).filter((bit) => matches(bit, filter));
	const hint =
		rows.length > SHOWN
			? t(
					"devices:models.install.source.shown",
					"{{shown, number}} of {{total, number}} shown. Search to find the others.",
					{ shown: SHOWN, total: rows.length },
				)
			: undefined;
	return (
		<div className="flex flex-col gap-3">
			<Field id={`${list.id}-search`} label={searchLabel} hint={hint}>
				<DvInput
					type="search"
					value={filter}
					placeholder={t(
						"devices:models.install.source.searchPlaceholder",
						"Qwen, Gemma, embeddings…",
					)}
					onChange={(event) => setFilter(event.target.value)}
				/>
			</Field>
			<BitList {...list} rows={rows} />
		</div>
	);
}

function HubSource({ pick, onPick, hub }: Readonly<SourceStepProps>) {
	const { t } = useTranslation("devices");
	return (
		<SearchedBits
			id="add-model-hub"
			searchLabel={t(
				"devices:models.install.source.searchLabel",
				"Search the hub",
			)}
			legend={t("devices:models.install.source.hubLegend", "Models on the hub")}
			query={hub}
			selected={pick.bit?.id}
			onSelect={(bit) => onPick({ ...pick, bit })}
			empty={t(
				"devices:models.install.source.hubEmpty",
				"No hub model a device can host matches.",
			)}
		/>
	);
}

function HuggingFaceSource({
	pick,
	onPick,
	onSubmit,
}: Readonly<SourceStepProps>) {
	const { t } = useTranslation("devices");
	return (
		<Field
			id="add-model-reference"
			label={t("devices:models.install.source.reference", "Repository")}
			hint={t(
				"devices:models.install.source.referenceHint",
				"Like Qwen/Qwen3-8B-GGUF, or its huggingface.co address. GGUF and MLX repositories work; the device downloads the files itself.",
			)}
		>
			<DvInput
				mono
				value={pick.reference}
				spellCheck={false}
				autoComplete="off"
				placeholder="owner/model"
				onChange={(event) => onPick({ ...pick, reference: event.target.value })}
				onKeyDown={(event) => {
					if (event.key === "Enter") onSubmit();
				}}
			/>
		</Field>
	);
}

function BitsSource({ pick, onPick, bits }: Readonly<SourceStepProps>) {
	const { t } = useTranslation("devices");
	return (
		<SearchedBits
			id="add-model-bits"
			searchLabel={t(
				"devices:models.install.source.bitsSearchLabel",
				"Search your models",
			)}
			legend={t("devices:models.install.source.bitsLegend", "Your models")}
			query={bits}
			selected={pick.bit?.id}
			onSelect={(bit) => onPick({ ...pick, bit })}
			empty={t(
				"devices:models.install.source.bitsEmpty",
				"None of your Bits is a model a device can host: a GGUF or MLX model from Hugging Face.",
			)}
			noMatch={t(
				"devices:models.install.source.bitsNoMatch",
				"None of your models matches.",
			)}
		/>
	);
}

const SOURCES: Record<
	ModelSource,
	(props: Readonly<SourceStepProps>) => ReactNode
> = {
	hub: HubSource,
	huggingface: HuggingFaceSource,
	bits: BitsSource,
};

/** Step 1: where the model comes from, and which one. */
export function SourceStep(props: Readonly<SourceStepProps>) {
	const { t } = useTranslation("devices");
	const { pick, onPick } = props;
	const Source = SOURCES[pick.source];
	return (
		<div data-step="source" className="flex flex-col gap-3.5">
			<Segmented
				label={t(
					"devices:models.install.source.label",
					"Where the model comes from",
				)}
				options={sourceOptions(t)}
				value={pick.source}
				onChange={(source) => onPick({ ...pick, source, bit: undefined })}
				wrap
			/>
			<Source {...props} />
			{props.error ? (
				<InlineResult tone="critical">
					{t(
						"devices:models.install.source.lookupFailed",
						"The model couldn't be read: {{reason}}",
						{ reason: props.error },
					)}
				</InlineResult>
			) : null}
		</div>
	);
}
