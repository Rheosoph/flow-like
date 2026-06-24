use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct PackageHlsVodNode;

impl PackageHlsVodNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PackageHlsVodNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_package_hls_vod",
            "Package HLS VOD",
            "Write an HLS media playlist plus MPEG-TS or fMP4 segments",
            "Streaming",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "playlist", "Playlist", "Target .m3u8 FlowPath");
        node.add_input_pin(
            "target_duration_seconds",
            "Target Duration",
            "Target segment duration in seconds",
            VariableType::Float,
        )
        .set_default_value(Some(json!(6.0)));
        node.add_input_pin(
            "segment_format",
            "Segment Format",
            "Segment container format",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["mpeg_ts".to_owned(), "fmp4".to_owned()])
                .build(),
        )
        .set_default_value(Some(json!("mpeg_ts")));
        node.add_input_pin(
            "segment_track_id",
            "Segment Track",
            "Track used for segment boundaries; 0 chooses first video/audio",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "copy_all_tracks",
            "Copy All Tracks",
            "Include every stream in each segment",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "segment_prefix",
            "Segment Prefix",
            "Optional segment object-key prefix",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "init_segment_name",
            "Init Segment",
            "Optional fMP4 init segment name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "uri_prefix",
            "URI Prefix",
            "Optional URI prefix written into playlist",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_flow_path_output(
            &mut node,
            "playlist_out",
            "Playlist",
            "Written playlist FlowPath",
        );
        node.add_output_pin(
            "segments",
            "Segments",
            "Written segment FlowPaths",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "init_segment",
            "Init Segment",
            "Written fMP4 init segment FlowPath when present",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "segment_count",
            "Segment Count",
            "Number of media segments",
            VariableType::Integer,
        );
        node.add_output_pin(
            "bytes_written",
            "Bytes Written",
            "Total bytes written",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let playlist: FlowPath = context.evaluate_pin("playlist").await?;
        let target_duration_seconds: f64 = context.evaluate_pin("target_duration_seconds").await?;
        let segment_format: String = context.evaluate_pin("segment_format").await?;
        let segment_track_id: i64 = context.evaluate_pin("segment_track_id").await?;
        let copy_all_tracks: bool = context.evaluate_pin("copy_all_tracks").await?;
        let segment_prefix: String = context.evaluate_pin("segment_prefix").await?;
        let init_segment_name: String = context.evaluate_pin("init_segment_name").await?;
        let uri_prefix: String = context.evaluate_pin("uri_prefix").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (playlist_store, playlist_location) = flow_path_object(context, &playlist).await?;

        let mut job = video_utils_rs::ObjectHlsVodJob::new()
            .with_target_duration(target_duration_seconds)
            .copy_all_tracks(copy_all_tracks);
        if let Some(track_id) = optional_track_id(segment_track_id)? {
            job = job.with_segment_track(track_id);
        }
        job = match segment_format.trim().to_ascii_lowercase().as_str() {
            "mpeg_ts" | "ts" => {
                job.with_segment_format(video_utils_rs::HlsSegmentContainer::MpegTs)
            }
            "fmp4" | "mp4" => job.with_segment_format(video_utils_rs::HlsSegmentContainer::Mp4),
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported HLS segment format: {}",
                    other
                ));
            }
        };
        if let Some(segment_prefix) = clean_optional(segment_prefix) {
            job = job.with_segment_prefix(segment_prefix);
        }
        if let Some(init_segment_name) = clean_optional(init_segment_name) {
            job = job.with_init_segment_name(init_segment_name);
        }
        if let Some(uri_prefix) = clean_optional(uri_prefix) {
            job = job.with_uri_prefix(uri_prefix);
        }

        let report = video_utils_rs::package_object_hls_vod_between_stores(
            source_store.as_ref(),
            &source_location,
            playlist_store.as_ref(),
            &playlist_location,
            &job,
        )
        .await?;
        let segments = report
            .segments
            .iter()
            .map(|path| flow_path_from_object_path(path, &playlist))
            .collect::<Vec<_>>();
        let init_segment = report
            .init_segment
            .as_ref()
            .map(|path| flow_path_from_object_path(path, &playlist));

        context
            .set_pin_value("playlist_out", json!(playlist))
            .await?;
        context.set_pin_value("segments", json!(segments)).await?;
        context
            .set_pin_value("init_segment", json!(init_segment))
            .await?;
        context
            .set_pin_value("segment_count", json!(report.segment_count as i64))
            .await?;
        context
            .set_pin_value("bytes_written", json!(report.bytes_written as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
