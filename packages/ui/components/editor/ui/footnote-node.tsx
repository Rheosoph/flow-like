"use client";

import { useContext } from "react";
import { PlateElement, type PlateElementProps } from "platejs/react";
import {
	type FootnoteElement,
	footnoteDomId,
	footnoteReferenceOccurrence,
} from "../../../lib/plate-footnotes";
import { FootnoteScopeContext } from "../footnote-context";

export function FootnoteReference(props: PlateElementProps<FootnoteElement>) {
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
		<PlateElement {...props} as="span">
			<sup contentEditable={false}>
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
		</PlateElement>
	);
}

export function FootnoteDefinition(props: PlateElementProps<FootnoteElement>) {
	const scope = useContext(FootnoteScopeContext);
	const { identifier } = props.element;
	return (
		<PlateElement
			{...props}
			attributes={{
				...props.attributes,
				id: footnoteDomId("note", scope, identifier),
			}}
			className="my-2 border-l-2 pl-3 text-sm"
		>
			<a
				contentEditable={false}
				href={`#${footnoteDomId("ref", scope, identifier)}`}
				aria-label={`Back to reference ${identifier}`}
				className="text-primary underline"
			>
				[{identifier}] ↩
			</a>
			{props.children}
		</PlateElement>
	);
}
