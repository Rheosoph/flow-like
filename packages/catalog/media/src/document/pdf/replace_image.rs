#[cfg(feature = "execute")]
use lopdf::{Document, Object, Stream};
#[cfg(feature = "execute")]
use std::io::Cursor;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

use crate::document::ImageScaleMode;

#[crate::register_node]
#[derive(Default)]
pub struct PdfReplaceImageNode;

impl PdfReplaceImageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfReplaceImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_replace_image",
            "Replace Image in PDF",
            "Replaces an image XObject in a PDF by name. Any image format is accepted and automatically converted to JPEG.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/image.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(6)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(6)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "template",
            "Template",
            "PDF file containing the image to replace",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "image_name",
            "Image Name",
            "XObject image name (e.g. \"Im0\", \"Image1\")",
            VariableType::String,
        );

        node.add_input_pin(
            "image",
            "Image",
            "Replacement image file (any format — auto-converted to JPEG)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "scale_mode",
            "Scale Mode",
            "How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)",
            VariableType::String,
        )
        .set_schema::<ImageScaleMode>()
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "None".to_string(),
                    "KeepWidth".to_string(),
                    "KeepHeight".to_string(),
                    "Stretch".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("KeepWidth")));

        node.add_input_pin(
            "output",
            "Output Path",
            "Path to save the modified PDF",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues after image replacement",
            VariableType::Execution,
        );

        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let image_name: String = context.evaluate_pin("image_name").await?;
        let image_path: FlowPath = context.evaluate_pin("image").await?;
        let scale_mode: ImageScaleMode = context.evaluate_pin("scale_mode").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let pdf_bytes = template.get(context, false).await?;
        let img_bytes = image_path.get(context, false).await?;

        let jpeg_bytes = to_jpeg(&img_bytes)?;

        let mut doc = Document::load_mem(&pdf_bytes)?;

        replace_xobject_image(&mut doc, &image_name, &jpeg_bytes, &scale_mode)?;

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;

        output.put(context, buf, false).await?;

        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PDF processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn to_jpeg(img_bytes: &[u8]) -> flow_like_types::Result<Vec<u8>> {
    use flow_like_types::image::{DynamicImage, ImageFormat, ImageReader};

    let reader = ImageReader::new(Cursor::new(img_bytes)).with_guessed_format()?;
    let format = reader.format();

    if format == Some(ImageFormat::Jpeg) {
        return Ok(img_bytes.to_vec());
    }

    let img: DynamicImage = reader.decode()?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

#[cfg(feature = "execute")]
fn replace_xobject_image(
    doc: &mut Document,
    image_name: &str,
    jpeg_bytes: &[u8],
    scale_mode: &ImageScaleMode,
) -> flow_like_types::Result<()> {
    let target_name = image_name.as_bytes();
    let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();

    let new_dimensions = get_jpeg_dimensions(jpeg_bytes);

    for page_id in page_ids {
        let xobject_refs = collect_xobject_refs(doc, page_id, target_name)?;

        for xobj_id in xobject_refs {
            let (orig_width, orig_height) = extract_image_dimensions(doc, xobj_id)?;
            let (final_w, final_h) =
                compute_scaled_dimensions(orig_width, orig_height, new_dimensions, scale_mode);

            let obj = doc.get_object_mut(xobj_id)?;
            let stream = obj
                .as_stream_mut()
                .map_err(|_| flow_like_types::anyhow!("XObject is not a stream"))?;

            build_jpeg_stream(stream, jpeg_bytes, final_w, final_h);
        }
    }

    Ok(())
}

#[cfg(feature = "execute")]
fn get_jpeg_dimensions(jpeg_bytes: &[u8]) -> Option<(u32, u32)> {
    use flow_like_types::image::ImageReader;
    ImageReader::new(Cursor::new(jpeg_bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

#[cfg(feature = "execute")]
fn compute_scaled_dimensions(
    orig_width: Option<i64>,
    orig_height: Option<i64>,
    new_dims: Option<(u32, u32)>,
    mode: &ImageScaleMode,
) -> (Option<i64>, Option<i64>) {
    match mode {
        ImageScaleMode::None => match new_dims {
            Some((w, h)) => (Some(w as i64), Some(h as i64)),
            None => (None, None),
        },
        ImageScaleMode::KeepWidth => {
            let w = orig_width;
            let h = match (orig_width, new_dims) {
                (Some(ow), Some((nw, nh))) if nw > 0 => {
                    Some((ow as f64 * nh as f64 / nw as f64).round() as i64)
                }
                _ => orig_height,
            };
            (w, h)
        }
        ImageScaleMode::KeepHeight => {
            let h = orig_height;
            let w = match (orig_height, new_dims) {
                (Some(oh), Some((nw, nh))) if nh > 0 => {
                    Some((oh as f64 * nw as f64 / nh as f64).round() as i64)
                }
                _ => orig_width,
            };
            (w, h)
        }
        ImageScaleMode::Stretch => (orig_width, orig_height),
    }
}

#[cfg(feature = "execute")]
fn collect_xobject_refs(
    doc: &Document,
    page_id: lopdf::ObjectId,
    target_name: &[u8],
) -> flow_like_types::Result<Vec<lopdf::ObjectId>> {
    let mut refs = Vec::new();

    let page = doc.get_object(page_id)?;
    let page_dict = page
        .as_dict()
        .map_err(|_| flow_like_types::anyhow!("Page is not a dictionary"))?;

    let resources = match page_dict.get(b"Resources") {
        Ok(r) => resolve_object(doc, r)?,
        Err(_) => return Ok(refs),
    };

    let res_dict = match resources.as_dict() {
        Ok(d) => d,
        Err(_) => return Ok(refs),
    };

    let xobjects = match res_dict.get(b"XObject") {
        Ok(x) => resolve_object(doc, x)?,
        Err(_) => return Ok(refs),
    };

    let xobj_dict = match xobjects.as_dict() {
        Ok(d) => d,
        Err(_) => return Ok(refs),
    };

    if let Ok(entry) = xobj_dict.get(target_name)
        && let Ok(id) = entry.as_reference()
    {
        refs.push(id);
    }

    Ok(refs)
}

#[cfg(feature = "execute")]
fn resolve_object<'a>(doc: &'a Document, obj: &'a Object) -> flow_like_types::Result<&'a Object> {
    match obj.as_reference() {
        Ok(id) => Ok(doc.get_object(id)?),
        Err(_) => Ok(obj),
    }
}

#[cfg(feature = "execute")]
fn extract_image_dimensions(
    doc: &Document,
    obj_id: lopdf::ObjectId,
) -> flow_like_types::Result<(Option<i64>, Option<i64>)> {
    let obj = doc.get_object(obj_id)?;
    let stream = obj
        .as_stream()
        .map_err(|_| flow_like_types::anyhow!("XObject is not a stream"))?;

    let width = stream.dict.get(b"Width").ok().and_then(|w| w.as_i64().ok());
    let height = stream
        .dict
        .get(b"Height")
        .ok()
        .and_then(|h| h.as_i64().ok());

    Ok((width, height))
}

#[cfg(feature = "execute")]
fn build_jpeg_stream(
    stream: &mut Stream,
    jpeg_bytes: &[u8],
    width: Option<i64>,
    height: Option<i64>,
) {
    stream.set_content(jpeg_bytes.to_vec());
    stream
        .dict
        .set("Filter", Object::Name(b"DCTDecode".to_vec()));
    stream.dict.set("Subtype", Object::Name(b"Image".to_vec()));
    stream
        .dict
        .set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    stream.dict.set("BitsPerComponent", Object::Integer(8));

    if let Some(w) = width {
        stream.dict.set("Width", Object::Integer(w));
    }
    if let Some(h) = height {
        stream.dict.set("Height", Object::Integer(h));
    }

    stream.dict.remove(b"DecodeParms");
    stream.dict.remove(b"SMask");
}
