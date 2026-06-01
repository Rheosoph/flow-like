use std::sync::{Arc, Weak};

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::NodeLogic,
        variable::Variable,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog_core::{BoundingBox, NodeImage};
use flow_like_catalog_media::image::annotate::draw_boxes::{DrawBoxesNode, draw_bboxes};
use flow_like_types::{
    Cacheable, Value,
    image::{DynamicImage, RgbaImage},
    json::json,
    sync::{Mutex, RwLock},
};

fn internal_node_with_logic(logic: Arc<dyn NodeLogic>) -> Arc<InternalNode> {
    let node = logic.get_node();
    let mut pins = AHashMap::new();
    let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

    for pin in node.pins.values() {
        let internal_pin = Arc::new(InternalPin::new(pin, false));
        name_cache
            .entry(pin.name.clone())
            .or_default()
            .push(internal_pin.clone());
        pins.insert(pin.id.clone(), internal_pin);
    }

    let internal = Arc::new(InternalNode::new(node, pins, logic, name_cache));

    for pin in internal.pins.values() {
        pin.init_node(Arc::downgrade(&internal));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }

    internal
}

async fn test_context(current: Arc<InternalNode>) -> ExecutionContext {
    let mut node_map = AHashMap::new();
    node_map.insert(current.node_id().to_string(), current.clone());

    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::<String, Variable>::new()));
    let cache = Arc::new(RwLock::new(AHashMap::<String, Arc<dyn Cacheable>>::new()));
    let run: Weak<Mutex<Run>> = Weak::new();

    ExecutionContext::new(
        Arc::new(node_map),
        &run,
        &state,
        &current,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
    )
    .await
}

#[test]
fn draw_boxes_tolerates_edge_and_invalid_boxes() {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        32,
        32,
        flow_like_types::image::Rgba([255, 255, 255, 255]),
    ));
    let boxes = vec![
        BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 12.0,
            y2: 12.0,
            score: 0.95,
            class_idx: 1,
            class_name: Some("person".to_string()),
        },
        BoundingBox {
            x1: 28.0,
            y1: 28.0,
            x2: 2.0,
            y2: 2.0,
            score: 0.5,
            class_idx: -1,
            class_name: None,
        },
        BoundingBox {
            x1: -100.0,
            y1: -100.0,
            x2: 1000.0,
            y2: 1000.0,
            score: 0.8,
            class_idx: 3,
            class_name: None,
        },
        BoundingBox {
            x1: f32::NAN,
            y1: 1.0,
            x2: 8.0,
            y2: 8.0,
            score: 0.2,
            class_idx: 4,
            class_name: None,
        },
    ];

    let output = draw_bboxes(image, &boxes).expect("draw_boxes should tolerate edge-case boxes");

    assert_eq!(output.width(), 32);
    assert_eq!(output.height(), 32);
}

#[tokio::test]
async fn draw_boxes_accepts_core_node_image_from_upstream_nodes() {
    let logic = Arc::new(DrawBoxesNode::new());
    let internal = internal_node_with_logic(logic.clone());
    let mut context = test_context(internal).await;

    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        96,
        96,
        flow_like_types::image::Rgba([255, 255, 255, 255]),
    ));
    let node_image = NodeImage::new(&mut context, image).await;
    let boxes = vec![BoundingBox {
        x1: 10.0,
        y1: 20.0,
        x2: 70.0,
        y2: 80.0,
        score: 0.93,
        class_idx: 1,
        class_name: Some("person".to_string()),
    }];

    context
        .set_pin_value("image_in", json!(node_image))
        .await
        .unwrap();
    context.set_pin_value("bboxes", json!(boxes)).await.unwrap();
    context
        .set_pin_value("use_ref", json!(false))
        .await
        .unwrap();

    logic
        .run(&mut context)
        .await
        .expect("draw_boxes should not fail with a NodeImageWrapper downcast error");

    let output_value: Value = context
        .node
        .get_pin_by_name("image_out")
        .await
        .unwrap()
        .get_raw_value()
        .await
        .expect("image_out should be set");
    let output_image: NodeImage = flow_like_types::json::from_value(output_value).unwrap();
    let output_ref = output_image.get_image(&mut context).await.unwrap();
    let output_guard = output_ref.lock().await;

    assert_eq!(output_guard.width(), 96);
    assert_eq!(output_guard.height(), 96);
}
