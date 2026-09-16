import { expect, test } from "bun:test";
import { mkdirSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { extractContract } from "../src/extract";
import { tmpDir } from "./helpers";
test("geometry annotations survive package exports and consumer tsconfig resolution", () => {
	const dir = tmpDir("flwb-package-geometry");
	mkdirSync(join(dir, "node_modules/@flow-like"), { recursive: true });
	symlinkSync(
		join(import.meta.dir, "../../widget-sdk"),
		join(dir, "node_modules/@flow-like/widget-sdk"),
		"dir",
	);
	writeFileSync(
		join(dir, "tsconfig.json"),
		JSON.stringify({
			compilerOptions: {
				module: "ESNext",
				moduleResolution: "Bundler",
				target: "ES2022",
				strict: true,
				skipLibCheck: true,
			},
		}),
	);
	mkdirSync(join(dir, "src"));
	const config = join(dir, "src/widget.config.ts");
	writeFileSync(
		config,
		`import {defineWidget,type GeoGeometry} from "@flow-like/widget-sdk";
 type Geometry = GeoGeometry;
 interface Inputs { geometry:Geometry; features:{geometry:Geometry}[]; }
 interface Events { selected:Geometry; }
 interface Queries { locate:{args:Geometry;returns:Geometry}; }
 export default defineWidget<Inputs,Events,Queries>({id:"package-geometry",name:"Geometry"});`,
	);
	const { contract } = extractContract(config);
	const expected = { type: "object", "x-flow-like-type": "geometry" };
	expect(contract.inputs.geometry?.schema).toEqual(expected);
	expect(
		(contract.inputs.features?.schema as any).items.properties.geometry,
	).toEqual(expected);
	expect(contract.events.selected?.payloadSchema).toEqual(expected);
	expect(contract.queries.locate?.resultSchema).toEqual(expected);
}, 30000);
