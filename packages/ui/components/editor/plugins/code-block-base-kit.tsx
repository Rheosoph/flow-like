"use client";
import {
	BaseCodeBlockPlugin,
	BaseCodeLinePlugin,
	BaseCodeSyntaxPlugin,
} from "@platejs/code-block";
import {
	CodeBlockElementStatic,
	CodeLineElementStatic,
	CodeSyntaxLeafStatic,
} from "../ui/code-block-node-static";
import { createEditorLowlight } from "./code-block-lowlight";

const lowlight = createEditorLowlight();

export const BaseCodeBlockKit = [
	BaseCodeBlockPlugin.configure({
		node: { component: CodeBlockElementStatic },
		options: { lowlight },
	}),
	BaseCodeLinePlugin.withComponent(CodeLineElementStatic),
	BaseCodeSyntaxPlugin.withComponent(CodeSyntaxLeafStatic),
];
