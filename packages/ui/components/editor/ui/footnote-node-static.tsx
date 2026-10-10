"use client";

import { useContext } from "react";
import { SlateElement, type SlateElementProps } from "platejs/static";
import {
	type FootnoteElement,
	footnoteDomId,
	footnoteReferenceOccurrence,
} from "../../../lib/plate-footnotes";
import { FootnoteScopeContext } from "../footnote-context";

export function FootnoteReferenceStatic(
	props: SlateElementProps<FootnoteElement>,
) {
	const scope = useContext(FootnoteScopeContext);
	const { identifier } = props.element;
	const occurrence = footnoteReferenceOccurrence(
		props.editor.children,
		props.element,
	);
	const id =
		footnoteDomId("ref", scope, identifier) +
		(occurrence ? `-${occurrence}` : "");
	return (
		<SlateElement {...props} as="span">
			<sup>
				<a
					id={id}
					href={`#${footnoteDomId("note", scope, identifier)}`}
					aria-label={`Footnote ${identifier}`}
					className="text-primary underline"
				>
					[{identifier}]
				</a>
			</sup>
			{props.children}
		</SlateElement>
	);
}

export function FootnoteDefinitionStatic(
	props: SlateElementProps<FootnoteElement>,
) {
	const scope = useContext(FootnoteScopeContext);
	const { identifier } = props.element;
	return (
		<SlateElement
			{...props}
			attributes={{
				...props.attributes,
				id: footnoteDomId("note", scope, identifier),
			}}
			className="my-2 border-l-2 pl-3 text-sm"
		>
			<a
				href={`#${footnoteDomId("ref", scope, identifier)}`}
				aria-label={`Back to reference ${identifier}`}
				className="text-primary underline"
			>
				[{identifier}] ↩
			</a>
			{props.children}
		</SlateElement>
	);
}
