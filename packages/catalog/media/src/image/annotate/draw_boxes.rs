use flow_like_catalog_core::{BoundingBox, NodeImage};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    Error,
    ab_glyph::FontArc,
    async_trait,
    image::{DynamicImage, Rgba},
    imageproc::{
        drawing::{draw_hollow_rect_mut, draw_text_mut},
        rect::Rect,
    },
    json::json,
};

/// Pastelle Colors for Bounding Boxes
pub const COLORS: [Rgba<u8>; 10] = [
    Rgba([204, 102, 204, 255]), // Darker Magenta
    Rgba([204, 102, 136, 255]), // Darker Pink
    Rgba([204, 163, 102, 255]), // Darker Peach
    Rgba([204, 204, 102, 255]), // Darker Yellow
    Rgba([102, 204, 142, 255]), // Darker Mint Green
    Rgba([102, 163, 204, 255]), // Darker Blue
    Rgba([163, 102, 204, 255]), // Darker Lavender
    Rgba([204, 102, 153, 255]), // Darker Rose
    Rgba([163, 204, 102, 255]), // Darker Lime
    Rgba([102, 204, 204, 255]), // Darker Cyan
];

// manually determined scale factors to print annotations / draw boxes
const SCALE_THICKNESS: f32 = 15. / 3726.;
const SCALE_FONT: f32 = 100. / 3726.;

fn clipped_bbox_rect(
    bbox: &BoundingBox,
    image_width: u32,
    image_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    if image_width == 0
        || image_height == 0
        || !bbox.x1.is_finite()
        || !bbox.y1.is_finite()
        || !bbox.x2.is_finite()
        || !bbox.y2.is_finite()
    {
        return None;
    }

    let max_x = image_width as f32;
    let max_y = image_height as f32;
    let x1 = bbox.x1.min(bbox.x2).clamp(0.0, max_x).floor() as u32;
    let y1 = bbox.y1.min(bbox.y2).clamp(0.0, max_y).floor() as u32;
    let x2 = bbox.x1.max(bbox.x2).clamp(0.0, max_x).ceil() as u32;
    let y2 = bbox.y1.max(bbox.y2).clamp(0.0, max_y).ceil() as u32;
    let width = x2.saturating_sub(x1);
    let height = y2.saturating_sub(y1);

    if width == 0 || height == 0 {
        return None;
    }

    Some((x1, y1, width, height))
}

fn u32_to_i32(value: u32) -> i32 {
    value.min(i32::MAX as u32) as i32
}

/// # Draw Rectangles
/// Draws hollow rectangles onto input image using BoundingBox coordinates
/// Applies box thickness that is dynamically scaled by input image resolution
pub fn draw_bboxes(mut img: DynamicImage, bboxes: &[BoundingBox]) -> Result<DynamicImage, Error> {
    let img_d = img.width().min(img.height()) as f32;
    let thickness = SCALE_THICKNESS * img_d; // scale thickness by smaller image edge
    let thickness = (thickness as u32).max(1);

    let font_data = include_bytes!("./assets/DejaVuSans.ttf");
    let font = FontArc::try_from_slice(font_data as &[u8]).unwrap();
    let font_scale = SCALE_FONT * img_d;
    let font_offset = (font_scale * 1.1) as u32;

    for bbox in bboxes.iter() {
        let box_color = COLORS[bbox.class_idx.rem_euclid(COLORS.len() as i32) as usize];
        let Some((x1, y1, w, h)) = clipped_bbox_rect(bbox, img.width(), img.height()) else {
            continue;
        };
        let label = match &bbox.class_name {
            Some(class_name) => format!("{} ({:.2})", class_name, bbox.score),
            None => format!("class {} ({:.2})", bbox.class_idx, bbox.score),
        };
        draw_text_mut(
            &mut img,
            box_color,
            u32_to_i32(x1),
            u32_to_i32(y1.saturating_sub(font_offset)),
            font_scale,
            &font,
            &label,
        );
        for t in 0..thickness {
            let x = x1.saturating_sub(t);
            let y = y1.saturating_sub(t);
            let grow = t.saturating_mul(2);
            let w = w.saturating_add(grow);
            let h = h.saturating_add(grow);
            let rect = Rect::at(u32_to_i32(x), u32_to_i32(y)).of_size(w, h);
            draw_hollow_rect_mut(&mut img, rect, box_color);
        }
    }
    Ok(img)
}

#[crate::register_node]
#[derive(Default)]
pub struct DrawBoxesNode {}

impl DrawBoxesNode {
    pub fn new() -> Self {
        DrawBoxesNode {}
    }
}

#[async_trait]
impl NodeLogic for DrawBoxesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "draw_boxes",
            "Draw Boxes",
            "Draw Bounding Boxes",
            "Image/Annotate",
        );
        node.set_flowscript_name("image", "drawBoxes");
        node.set_receiver("image_in");
        node.set_version(1);
        node.add_icon("/flow/icons/image.svg");

        // inputs
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin("image_in", "Image", "Image object", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("bboxes", "Boxes", "Bounding Boxes", VariableType::Struct)
            .set_schema::<BoundingBox>()
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "use_ref",
            "Use Reference",
            "Use Reference of the image, transforming the original instead of a copy",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false))); // default false since we typically want to re-use the source image without painted boxes

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "image_out",
            "Image",
            "Image with Bounding Boxes",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        // fetch inputs
        let mut node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        let bboxes: Vec<BoundingBox> = context.evaluate_pin("bboxes").await?;
        if !use_ref {
            node_img = node_img.copy_image(context).await?;
        }
        let img = node_img.get_image(context).await?;

        // annotate image
        {
            let mut img_guard = img.lock().await;
            let img_annotated = draw_bboxes(img_guard.clone(), &bboxes)?;
            *img_guard = img_annotated;
        }

        // set outputs
        context.set_pin_value("image_out", json!(node_img)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
