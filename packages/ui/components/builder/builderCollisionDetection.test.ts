import { describe, expect, test } from "bun:test";
import type { CollisionDetection } from "@dnd-kit/core";
import { Window } from "happy-dom";
import type { SurfaceComponent } from "../a2ui/types";
import {
	createBuilderCollisionDetection,
	measureBuilderDroppable,
} from "./builderCollisionDetection";

function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	const root = window.document.createElement("div");
	const nested = window.document.createElement("div");
	root.appendChild(nested);
	window.document.body.appendChild(root);
	const components = new Map([
		[
			"root",
			{
				id: "root",
				component: {
					type: "column",
					children: { explicitList: ["nested", "moving"] },
				},
			},
		],
		[
			"nested",
			{
				id: "nested",
				component: { type: "column", children: { explicitList: [] } },
			},
		],
		["moving", { id: "moving", component: { type: "text" } }],
	] as [string, SurfaceComponent][]);
	const rect = (left: number, top: number, width: number, height: number) =>
		new window.DOMRect(left, top, width, height);
	const containers = [
		{
			id: "outer",
			key: "outer",
			disabled: false,
			data: { current: { type: "drop-zone", parentId: "root", index: 0 } },
			node: { current: root },
			rect: { current: rect(0, 0, 400, 400) },
		},
		{
			id: "inner",
			key: "inner",
			disabled: false,
			data: { current: { type: "container", parentId: "nested" } },
			node: { current: nested },
			rect: { current: rect(100, 100, 150, 150) },
		},
	];
	const args = {
		active: {
			id: "drag",
			data: { current: { type: "a2ui-component", componentType: "text" } },
			rect: {
				current: {
					initial: rect(0, 0, 40, 40),
					translated: rect(0, 0, 40, 40),
				},
			},
		},
		pointerCoordinates: { x: 140, y: 140 },
		collisionRect: rect(0, 0, 400, 400),
		droppableContainers: containers,
		droppableRects: new Map(
			containers.map((target) => [target.id, target.rect.current]),
		),
	} as unknown as Parameters<CollisionDetection>[0];
	return {
		detect: createBuilderCollisionDetection(components),
		args,
		window,
		root,
		nested,
		rect,
	};
}

describe("builder drop targets", () => {
	test("nested containers beat an ancestor's full-size drop zone", () => {
		const { detect, args } = setup();
		expect(detect(args)[0]?.id).toBe("inner");
	});

	test("releasing outside targets has no rectangle or previous-target fallback", () => {
		const { detect, args } = setup();
		expect(detect(args)).toHaveLength(1);
		expect(detect({ ...args, pointerCoordinates: { x: 450, y: 450 } })).toEqual(
			[],
		);
	});

	test("explicit hierarchy insertion zones beat their containing row", () => {
		const { detect, args } = setup();
		args.droppableContainers[1].data.current = {
			type: "drop-zone",
			parentId: "root",
			index: 1,
		};
		expect(detect(args)[0]?.data?.dropData).toMatchObject({
			parentId: "root",
			index: 1,
		});
	});

	test("resolves fresh geometry while staying over the same container", () => {
		const { detect, args, root, nested, window, rect } = setup();
		args.droppableContainers = args.droppableContainers.slice(0, 1);
		args.droppableContainers[0].data.current = {
			type: "container",
			parentId: "root",
		};
		root.dataset.builderComponent = "root";
		root.style.display = "flex";
		root.style.flexDirection = "column";
		root.getBoundingClientRect = () => rect(0, 0, 400, 400);
		nested.dataset.builderComponent = "nested";
		nested.getBoundingClientRect = () => rect(50, 50, 300, 100);
		const moving = window.document.createElement("div");
		moving.dataset.builderComponent = "moving";
		moving.getBoundingClientRect = () => rect(50, 250, 300, 100);
		root.appendChild(moving);
		expect(
			detect({ ...args, pointerCoordinates: { x: 100, y: 60 } })[0]?.data
				?.dropData.index,
		).toBe(0);
		expect(
			detect({ ...args, pointerCoordinates: { x: 100, y: 340 } })[0]?.data
				?.dropData,
		).toMatchObject({ index: 2, indicator: { top: 349 } });
	});

	test("does not target the moving component itself", () => {
		const { detect, args } = setup();
		args.active.data.current = {
			type: "a2ui-component-move",
			componentId: "nested",
			currentParentId: "root",
		};
		expect(detect(args).some((collision) => collision.id === "inner")).toBe(
			false,
		);
	});

	test("container edges insert beside it while its center accepts children", () => {
		const { detect, args, root, nested, rect } = setup();
		args.droppableContainers[0].data.current = {
			type: "container",
			parentId: "root",
		};
		root.dataset.builderComponent = "root";
		root.style.display = "flex";
		root.style.flexDirection = "column";
		root.getBoundingClientRect = () => rect(0, 0, 400, 400);
		nested.dataset.builderComponent = "nested";
		nested.getBoundingClientRect = () => rect(100, 100, 150, 150);
		expect(
			detect({ ...args, pointerCoordinates: { x: 140, y: 103 } })[0]?.data
				?.dropData,
		).toMatchObject({ parentId: "root", index: 0 });
		expect(
			detect({ ...args, pointerCoordinates: { x: 140, y: 247 } })[0]?.data
				?.dropData,
		).toMatchObject({ parentId: "root", index: 1 });
		expect(detect(args)[0]?.data?.dropData.parentId).toBe("nested");
		root.style.flexDirection = "row";
		expect(
			detect({ ...args, pointerCoordinates: { x: 103, y: 140 } })[0]?.data
				?.dropData,
		).toMatchObject({ parentId: "root", index: 0 });
	});

	test("ignores portions of canvas targets clipped by a scroll viewport", () => {
		const { detect, args, root, window, rect } = setup();
		const viewport = window.document.createElement("div");
		viewport.style.overflowY = "auto";
		viewport.getBoundingClientRect = () => rect(0, 0, 400, 100);
		root.parentElement?.appendChild(viewport);
		viewport.appendChild(root);
		expect(detect(args)).toEqual([]);
	});

	test("measures display contents using its rendered children", () => {
		const { root, nested, rect } = setup();
		root.style.display = "contents";
		root.getBoundingClientRect = () => rect(0, 0, 0, 0);
		nested.getBoundingClientRect = () => rect(100, 150, 200, 80);
		expect(measureBuilderDroppable(root as unknown as HTMLElement)).toEqual({
			left: 100,
			top: 150,
			width: 200,
			height: 80,
			right: 300,
			bottom: 230,
		});
	});

	test("gives empty zero-height containers a hit area without altering the element", () => {
		const { nested, rect } = setup();
		nested.dataset.builderEmpty = "true";
		nested.getBoundingClientRect = () => rect(100, 150, 200, 0);
		expect(measureBuilderDroppable(nested as unknown as HTMLElement)).toEqual({
			left: 100,
			top: 150,
			width: 200,
			height: 32,
			right: 300,
			bottom: 182,
		});
		expect(nested.style.height).toBe("");
		nested.style.display = "none";
		expect(
			measureBuilderDroppable(nested as unknown as HTMLElement).height,
		).toBe(0);
	});
});
