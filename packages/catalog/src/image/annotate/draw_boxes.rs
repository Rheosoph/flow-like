use crate::image::{BoundingBox, NodeImage};

use flow_like::{
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
    state::FlowLikeState,
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

/// # Draw Rectangles
/// Draws hollow rectangles onto input image using BoundingBox coordinates
/// Applies box thickness that is dynamically scaled by input image resolution
pub fn draw_bboxes(
    mut img: DynamicImage,
    bboxes: &Vec<BoundingBox>,
) -> Result<DynamicImage, Error> {
    let img_d = img.width().min(img.height()) as f32;
    let thickness = SCALE_THICKNESS * img_d; // scale thickness by smaller image edge
    let thickness = (thickness as u32).max(1);

    let font_data = include_bytes!("./assets/DejaVuSans.ttf");
    let font = FontArc::try_from_slice(font_data as &[u8]).unwrap();
    let font_scale = SCALE_FONT * img_d;
    let font_offset = (font_scale * 1.1) as u32;

    for bbox in bboxes.iter() {
        let box_color = COLORS[(bbox.class_idx as usize) % COLORS.len()];
        let (x1, y1, w, h) = bbox.x1y1wh();
        let label = format!("class {} ({:.2})", bbox.class_idx, bbox.score);
        tracing::trace!(%label, "Drawing bounding box");
        draw_text_mut(
            &mut img,
            box_color,
            x1 as i32,
            (y1 - font_offset) as i32,
            font_scale,
            &font,
            &label,
        );
        for t in 0..thickness {
            let x = if x1 > t { x1 - t } else { x1 };
            let y = if y1 > t { y1 - t } else { y1 };
            let w = w + 2 * t;
            let h = h + 2 * t;
            let rect = Rect::at(x as i32, y as i32).of_size(w, h);
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
