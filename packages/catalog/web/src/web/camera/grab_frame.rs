use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{Bytes, async_trait, json::json};
use futures::StreamExt;

use crate::web::api::{GuardedHttpClient, HttpRequest};

#[crate::register_node]
#[derive(Default)]
pub struct GrabFrameNode {}

impl GrabFrameNode {
    pub fn new() -> Self {
        GrabFrameNode {}
    }
}

#[async_trait]
impl NodeLogic for GrabFrameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "web_camera_grab_frame",
            "Grab IP-Camera Frame",
            "Captures a frame from an IP camera",
            "Web/Camera",
        );

        node.add_icon("/flow/icons/cctv.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the HTTP request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "request",
            "Request",
            "The HTTP request to perform",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_success",
            "Success",
            "Execution if the request succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "image",
            "Image",
            "The captured image frame",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node.add_output_pin(
            "exec_error",
            "Error",
            "Execution if the request fails",
            VariableType::Execution,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;
        context.activate_exec_pin("exec_error").await?;

        let request: HttpRequest = context.evaluate_pin("request").await?;
        let client = GuardedHttpClient::new(context.execution_environment())?;

        let response = request.raw_request(&client).await?;
        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();

        let mut frame = Vec::new();

        while let Some(chunk_res) = stream.next().await {
            let chunk: Bytes = chunk_res.unwrap();
            buf.extend_from_slice(&chunk);

            if let Some(start) = buf.windows(2).position(|w| w == [0xFF, 0xD8]) {
                // Look for JPEG End Of Image marker: 0xFF 0xD9
                if let Some(end) = buf.windows(2).position(|w| w == [0xFF, 0xD9]) {
                    // `end` is the index of the 0xFF; include both bytes by adding 2
                    frame = buf[start..end + 2].to_vec();
                    break;
                }
            }

            // Keep the buffer from growing unbounded:
            // once it's too big without a complete frame, drop the front
            if buf.len() > 5_000_000 {
                // drop everything except last 1 MB
                buf.drain(0..(buf.len() - 1_000_000));
            }
        }

        let dynamic_image = flow_like_types::image::load_from_memory(&frame)
            .map_err(|e| flow_like_types::anyhow!("Failed to load image from bytes: {}", e))?;

        let node_image = NodeImage::new(context, dynamic_image).await;

        context.set_pin_value("image", json!(node_image)).await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use flow_like_types::reqwest;

    use flow_like_types::{Bytes, tokio};
    use futures::StreamExt;

    #[tokio::test]
    async fn test_grab_mjpg_frame() {
        let url = "http://82.127.206.236/axis-cgi/mjpg/video.cgi";

        let resp = reqwest::get(url).await.unwrap();
        let mut stream = resp.bytes_stream();

        let mut buf: Vec<u8> = Vec::new();

        while let Some(chunk_res) = stream.next().await {
            let chunk: Bytes = chunk_res.unwrap();
            buf.extend_from_slice(&chunk);

            // Look for JPEG Start Of Image marker: 0xFF 0xD8
            if let Some(start) = buf.windows(2).position(|w| w == [0xFF, 0xD8]) {
                // Look for JPEG End Of Image marker: 0xFF 0xD9
                if let Some(end) = buf.windows(2).position(|w| w == [0xFF, 0xD9]) {
                    // `end` is the index of the 0xFF; include both bytes by adding 2
                    let frame = buf[start..end + 2].to_vec();

                    // Save out the first frame to a file
                    std::fs::write("frame.jpg", &frame).unwrap();
                    println!("Saved frame.jpg ({} bytes)", frame.len());
                    break;
                }
            }

            // Keep the buffer from growing unbounded:
            // once it's too big without a complete frame, drop the front
            if buf.len() > 5_000_000 {
                // drop everything except last 1 MB
                buf.drain(0..(buf.len() - 1_000_000));
            }
        }
    }
}
