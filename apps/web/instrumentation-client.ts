import { loader } from "@monaco-editor/react";
import { version } from "monaco-editor/package.json";

// Configure the shared loader before hydration can mount an editor.
loader.config({ paths: { vs: `/monaco/${version}/vs` } });
