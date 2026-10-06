import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	mkdirSync,
	readFileSync,
	readdirSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { join, parse, relative } from "node:path";
import { generateSystemOneTypes } from "./systemone-schema";

console.log("Running cargo command: cargo run --bin schema-gen");
execSync("cargo run --bin schema-gen", { stdio: "inherit" });

const schemaDir = "packages/schema";
const outputDir = "packages/ui/lib/schema";

function getAllFiles(dir: string): string[] {
	let files: string[] = [];

	readdirSync(dir).forEach((file) => {
		const fullPath = join(dir, file);
		if (statSync(fullPath).isDirectory()) {
			files = files.concat(getAllFiles(fullPath));
		} else {
			files.push(fullPath);
		}
	});

	return files;
}

function normalizeType(typeStr: string): string {
	return typeStr.trim().replace(/\s+/g, " ");
}

function hashType(typeStr: string): string {
	return createHash("sha256").update(normalizeType(typeStr)).digest("hex");
}

function extractTypes(content: string): Map<string, string> {
	const types = new Map<string, string>();
	const interfaceRegex = /export interface (\w+) \{[\s\S]*?\n\}/g;
	const enumRegex = /export enum (\w+) \{[\s\S]*?\n\}/g;

	for (const match of content.matchAll(interfaceRegex)) {
		types.set(match[1], match[0]);
	}

	for (const match of content.matchAll(enumRegex)) {
		types.set(match[1], match[0]);
	}

	return types;
}

mkdirSync(outputDir, { recursive: true });

const schemaFiles = getAllFiles(schemaDir).filter((file) =>
	file.endsWith(".json"),
);

const typeHashMap = new Map<
	string,
	{ name: string; definition: string; sourceFile: string }
>();
const fileTypeMap = new Map<string, Set<string>>();

console.log("\n=== Phase 1: Generate and collect types ===");

schemaFiles.forEach((schemaFile) => {
	const relativePath = relative(schemaDir, schemaFile);
	const parsedPath = parse(relativePath);
	const outputFilePath = join(
		outputDir,
		parsedPath.dir,
		`${parsedPath.name}.ts`,
	);

	mkdirSync(parse(outputFilePath).dir, { recursive: true });

	const systemOneTypes = generateSystemOneTypes(
		JSON.parse(readFileSync(schemaFile, "utf-8")),
	);
	if (systemOneTypes !== undefined) {
		writeFileSync(outputFilePath, systemOneTypes);
		return;
	}

	const quicktypeCommand = `bunx quicktype@23.0.0 --just-types -o ${outputFilePath} -s schema ${schemaFile}`;

	console.log(`Processing: ${schemaFile}`);
	execSync(quicktypeCommand, { stdio: "ignore" });

	const prefixes = new Set<string>();
	let content = readFileSync(outputFilePath, "utf-8");

	for (const match of content.matchAll(/interface (\w+) \{/g)) {
		prefixes.add(match[1]);
	}

	for (const match of content.matchAll(/enum (\w+) \{/g)) {
		prefixes.add(match[1]);
	}

	for (const prefix of prefixes) {
		content = content.replaceAll(
			new RegExp(`\\b${prefix}(?=[ ;\\[])`, "g"),
			`I${prefix}`,
		);
	}

	writeFileSync(outputFilePath, content);

	const types = extractTypes(content);
	fileTypeMap.set(outputFilePath, new Set());

	for (const [typeName, typeDefinition] of types) {
		const hash = hashType(typeDefinition);

		if (!typeHashMap.has(hash)) {
			typeHashMap.set(hash, {
				name: typeName,
				definition: typeDefinition,
				sourceFile: outputFilePath,
			});
		}

		fileTypeMap.get(outputFilePath)?.add(hash);
	}
});

console.log("\n=== Phase 2: Create shared types file ===");

const sharedTypes = new Map<string, { name: string; definition: string }>();
const hashToCanonicalName = new Map<string, string>();

for (const [hash, typeInfo] of typeHashMap) {
	const filesUsingType = Array.from(fileTypeMap.entries())
		.filter(([_, hashes]) => hashes.has(hash))
		.map(([file]) => file);

	if (filesUsingType.length > 1) {
		sharedTypes.set(hash, typeInfo);
		hashToCanonicalName.set(hash, typeInfo.name);
		console.log(
			`Shared type found: ${typeInfo.name} (used in ${filesUsingType.length} files)`,
		);
	}
}

if (sharedTypes.size > 0) {
	const sharedTypesContent = Array.from(sharedTypes.values())
		.map((t) => t.definition)
		.join("\n\n");

	const sharedTypesPath = join(outputDir, "shared-types.ts");
	writeFileSync(sharedTypesPath, sharedTypesContent);
	console.log(`Created shared types file with ${sharedTypes.size} types`);
}

console.log("\n=== Phase 3: Deduplicate files ===");

for (const [filePath, hashes] of fileTypeMap) {
	let content = readFileSync(filePath, "utf-8");
	const types = extractTypes(content);
	const imports = new Set<string>();
	let modified = false;

	for (const [typeName, typeDefinition] of types) {
		const hash = hashType(typeDefinition);

		if (sharedTypes.has(hash)) {
			const canonicalName = hashToCanonicalName.get(hash);
			if (canonicalName) {
				content = content.replace(typeDefinition, "");

				if (typeName !== canonicalName) {
					content = content.replaceAll(
						new RegExp(`\\b${typeName}\\b`, "g"),
						canonicalName,
					);
				}

				imports.add(canonicalName);
				modified = true;
			}
		}
	}

	if (modified && imports.size > 0) {
		const relativePathToShared = relative(
			parse(filePath).dir,
			join(outputDir, "shared-types"),
		).replace(/\\/g, "/");

		const importPath = relativePathToShared.startsWith(".")
			? relativePathToShared
			: `./${relativePathToShared}`;

		const importStatement = `import type { ${Array.from(imports).sort().join(", ")} } from "${importPath}";\n\n`;
		content = importStatement + content.replace(/^\s*\n/gm, "");

		writeFileSync(filePath, content);
		console.log(
			`Deduplicated: ${relative(outputDir, filePath)} (${imports.size} shared types)`,
		);
	}
}

console.log("\n=== Schema generation completed ===");
