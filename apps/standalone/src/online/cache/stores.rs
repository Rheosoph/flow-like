//! File stores implement object_store 0.13 and Lance stores 0.14. Each release
//! gets the same read-through store over the one shared [`CacheControl`].

macro_rules! read_cache {
    ($module:ident, $($object_store:ident)::+, { $($extensions:tt)* }) => {
        pub(in crate::online) mod $module {
            use crate::online::cache::*;
            use $($object_store)::+::{
                self as object_store, Attribute, Attributes, CopyOptions, GetOptions, GetResult,
                GetResultPayload, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
                ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload, PutResult,
                RenameOptions, UploadPart, path::Path as ObjectPath,
            };
            use async_trait::async_trait;
            use bytes::BytesMut;
            use flow_like_types::async_stream::try_stream;
            use futures_util::{StreamExt, stream::BoxStream};
            use std::{fmt, sync::Arc};

            impl From<CacheFailure> for object_store::Error {
                fn from(failure: CacheFailure) -> Self {
                    match failure {
                        CacheFailure::Generic(source) => Self::Generic {
                            store: "StandaloneReadCache",
                            source,
                        },
                        CacheFailure::OutsideScope(path) => Self::PermissionDenied {
                            path,
                            source: OUTSIDE_SCOPE.into(),
                        },
                    }
                }
            }
            pub(in crate::online::cache) fn cache_error(
                error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
            ) -> object_store::Error {
                crate::online::cache::cache_error(error).into()
            }
            fn offline() -> object_store::Error {
                cache_error(OfflineCacheMiss)
            }

            impl object_store::client::HttpConnector for OfflineConnector {
                fn connect(
                    &self,
                    options: &object_store::ClientOptions,
                ) -> object_store::Result<object_store::client::HttpClient> {
                    use object_store::client::{HttpClient, ReqwestConnector};
                    Ok(HttpClient::new(OfflineHttpService(
                        ReqwestConnector::default().connect(options)?,
                    )))
                }
            }
            #[derive(Debug)]
            pub(in crate::online::cache) struct OfflineHttpService(
                pub(in crate::online::cache) object_store::client::HttpClient,
            );
            #[async_trait]
            impl object_store::client::HttpService for OfflineHttpService {
                async fn call(
                    &self,
                    request: object_store::client::HttpRequest,
                ) -> std::result::Result<
                    object_store::client::HttpResponse,
                    object_store::client::HttpError,
                > {
                    let read = matches!(request.method().as_str(), "GET" | "HEAD");
                    let response = self.0.execute(request).await?;
                    if read && response.status().is_server_error() {
                        return Err(object_store::client::HttpError::new(
                            object_store::client::HttpErrorKind::Request,
                            CloudUnavailable,
                        ));
                    }
                    Ok(response)
                }
            }

            pub(in crate::online::cache) fn cached_meta(
                meta: &ObjectMeta,
                attributes: &Attributes,
            ) -> CachedMeta {
                CachedMeta {
                    path: meta.location.to_string(),
                    modified: meta.last_modified.to_rfc3339(),
                    size: meta.size,
                    etag: meta.e_tag.clone(),
                    version: meta.version.clone(),
                    attributes: attributes
                        .iter()
                        .filter_map(|(key, value)| {
                            let key = match key {
                                Attribute::ContentDisposition => "disposition".into(),
                                Attribute::ContentEncoding => "encoding".into(),
                                Attribute::ContentLanguage => "language".into(),
                                Attribute::ContentType => "type".into(),
                                Attribute::CacheControl => "control".into(),
                                Attribute::StorageClass => "class".into(),
                                Attribute::Metadata(name) => format!("metadata:{name}"),
                                _ => return None,
                            };
                            Some((key, value.as_ref().to_owned()))
                        })
                        .collect(),
                }
            }
            fn object_meta(stored: &CachedMeta) -> object_store::Result<ObjectMeta> {
                Ok(ObjectMeta {
                    location: ObjectPath::parse(&stored.path)?,
                    last_modified: stored.modified.parse().map_err(cache_error)?,
                    size: stored.size,
                    e_tag: stored.etag.clone(),
                    version: stored.version.clone(),
                })
            }
            fn object_attributes(stored: &CachedMeta) -> Attributes {
                stored
                    .attributes
                    .iter()
                    .filter_map(|(key, value)| {
                        let key = match key.as_str() {
                            "disposition" => Attribute::ContentDisposition,
                            "encoding" => Attribute::ContentEncoding,
                            "language" => Attribute::ContentLanguage,
                            "type" => Attribute::ContentType,
                            "control" => Attribute::CacheControl,
                            "class" => Attribute::StorageClass,
                            key => Attribute::Metadata(
                                key.strip_prefix("metadata:")?.to_owned().into(),
                            ),
                        };
                        Some((key, object_store::AttributeValue::from(value.clone())))
                    })
                    .collect()
            }

            pub(in crate::online::cache) fn delimiter_snapshot(
                result: &ListResult,
            ) -> Option<Vec<u8>> {
                use std::io::Write;
                let mut json = ListingJson(Vec::new());
                json.write_all(b"[[").ok()?;
                for (index, prefix) in result.common_prefixes.iter().enumerate() {
                    if index != 0 {
                        json.write_all(b",").ok()?;
                    }
                    serde_json::to_writer(&mut json, prefix.as_ref()).ok()?;
                }
                json.write_all(b"],[").ok()?;
                for (index, meta) in result.objects.iter().enumerate() {
                    if index != 0 {
                        json.write_all(b",").ok()?;
                    }
                    // Bound each clone before constructing its cache representation.
                    if meta.location.as_ref().len() > LIST_BYTES
                        || meta
                            .e_tag
                            .as_ref()
                            .is_some_and(|value| value.len() > LIST_BYTES)
                        || meta
                            .version
                            .as_ref()
                            .is_some_and(|value| value.len() > LIST_BYTES)
                    {
                        return None;
                    }
                    serde_json::to_writer(&mut json, &cached_meta(meta, &Attributes::default()))
                        .ok()?;
                }
                json.write_all(b"]]").ok()?;
                Some(json.0)
            }

            #[derive(Debug, Clone)]
            pub(in crate::online) struct ReadCache {
                pub(in crate::online::cache) inner: Arc<dyn ObjectStore>,
                pub(in crate::online::cache) control: Arc<CacheControl>,
            }
            impl fmt::Display for ReadCache {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("StandaloneCloudReadCache")
                }
            }
            impl ReadCache {
                pub(in crate::online) fn new(
                    inner: Arc<dyn ObjectStore>,
                    control: Arc<CacheControl>,
                ) -> Arc<Self> {
                    Arc::new(Self { inner, control })
                }
                fn cached(
                    &self,
                    path: &ObjectPath,
                    options: &GetOptions,
                ) -> object_store::Result<Option<GetResult>> {
                    let Some(data) = self.control.read(&format!("meta:{path}"))? else {
                        return Ok(None);
                    };
                    let stored: CachedMeta = serde_json::from_slice(&data).map_err(cache_error)?;
                    let meta = object_meta(&stored)?;
                    if options
                        .version
                        .as_ref()
                        .is_some_and(|version| meta.version.as_ref() != Some(version))
                    {
                        return Ok(None);
                    }
                    options.check_preconditions(&meta)?;
                    let range = if options.head {
                        0..0
                    } else {
                        options
                            .range
                            .as_ref()
                            .map(|range| range.as_range(meta.size))
                            .transpose()
                            .map_err(cache_error)?
                            .unwrap_or(0..meta.size)
                    };
                    let generation = stored.generation()?;
                    if !self.control.covers(path.as_ref(), &generation, &range)? {
                        return Ok(None);
                    }
                    let control = self.control.clone();
                    let path = path.to_string();
                    let mut position = range.start;
                    let end = range.end;
                    let stream = try_stream! {
                        while position < end {
                            let bytes = control.read_block(&path,&generation,position,end)?.ok_or_else(offline)?;
                            position += bytes.len() as u64;
                            yield bytes;
                        }
                    };
                    Ok(Some(GetResult {
                        payload: GetResultPayload::Stream(Box::pin(stream)),
                        meta,
                        range,
                        attributes: object_attributes(&stored),
                        $($extensions)*
                    }))
                }
                fn wrap_result(
                    &self,
                    result: GetResult,
                    epoch: i64,
                    head: bool,
                    cache_current: bool,
                ) -> object_store::Result<GetResult> {
                    let meta = result.meta.clone();
                    self.control.scope(&meta.location, false)?;
                    let attributes = result.attributes.clone();
                    let range = result.range.clone();
                    // An explicit historical version must not replace the current object
                    // that an unversioned offline read will use.
                    let generation = if cache_current {
                        self.control
                            .observe(&cached_meta(&meta, &attributes), epoch)?
                    } else {
                        None
                    };
                    let control = self.control.clone();
                    let path = meta.location.to_string();
                    let mut position = range.start;
                    let end = range.end;
                    let mut source = result.into_stream();
                    let stream = try_stream! {
                        let mut pending = BytesMut::new();
                        let mut block_start = position;
                        while let Some(result) = source.next().await {
                            let mut bytes = match result {
                                Ok(bytes) => bytes,
                                Err(error) => { control.handle_error(&error); Err(error)?; unreachable!() }
                            };
                            control.authorized()?;
                            if !head && bytes.len() as u64 > end.saturating_sub(position) { Err(cache_error("Cloud object returned an invalid byte range"))?; }
                            while !bytes.is_empty() {
                                let chunk = bytes.split_to(bytes.len().min(BLOCK_BYTES-pending.len()));
                                if !head && generation.is_some() {
                                    pending.extend_from_slice(&chunk);
                                    if pending.len() == BLOCK_BYTES {
                                        control.block(&path,generation.as_deref().unwrap(),block_start,&pending,epoch)?;
                                        pending.clear();
                                        block_start = position + chunk.len() as u64;
                                    }
                                }
                                position += chunk.len() as u64;
                                yield chunk;
                            }
                        }
                        if !head && position != end { Err(cache_error("Cloud object ended before its declared range"))?; }
                        if !pending.is_empty() { control.block(&path,generation.as_deref().unwrap(),block_start,&pending,epoch)?; }
                        control.cloud_succeeded();
                    };
                    Ok(GetResult {
                        payload: GetResultPayload::Stream(Box::pin(stream)),
                        meta,
                        range,
                        attributes,
                        $($extensions)*
                    })
                }
                fn list_key(
                    &self,
                    prefix: Option<&ObjectPath>,
                    delimiter: bool,
                ) -> object_store::Result<String> {
                    let prefix = prefix.ok_or_else(|| {
                        cache_error("Device listing requires an authorized project or user directory")
                    })?;
                    self.control.scope(prefix, true)?;
                    Ok(format!("list:{}:{prefix}", u8::from(delimiter)))
                }
                fn cached_delimiter(&self, key: &str) -> object_store::Result<ListResult> {
                    let bytes = self.control.read(key)?.ok_or_else(offline)?;
                    let (prefixes, objects): (Vec<String>, Vec<CachedMeta>) =
                        serde_json::from_slice(&bytes).map_err(cache_error)?;
                    let result = ListResult {
                        common_prefixes: prefixes
                            .iter()
                            .map(ObjectPath::parse)
                            .collect::<std::result::Result<_, _>>()?,
                        objects: objects
                            .iter()
                            .map(object_meta)
                            .collect::<object_store::Result<_>>()?,
                        $($extensions)*
                    };
                    for prefix in &result.common_prefixes {
                        self.control.scope(prefix, true)?;
                    }
                    for meta in &result.objects {
                        self.control.scope(&meta.location, false)?;
                    }
                    Ok(result)
                }
            }

            #[async_trait]
            impl ObjectStore for ReadCache {
                async fn get_opts(
                    &self,
                    path: &ObjectPath,
                    options: GetOptions,
                ) -> object_store::Result<GetResult> {
                    self.control.scope(path, false)?;
                    if (immutable_lance_data(path) || immutable_model_artifact(path))
                        && self.control.immutable_trusted(path)?
                    {
                        if let Some(result) = self.cached(path, &options)? {
                            return Ok(result);
                        }
                    }
                    if !self.control.try_cloud_read()? {
                        return self.cached(path, &options)?.ok_or_else(offline);
                    }
                    let epoch = self.control.epoch()?;
                    let result = tokio::time::timeout(
                        CLOUD_PROBE_TIMEOUT,
                        self.inner.get_opts(path, options.clone()),
                    )
                    .await
                    .unwrap_or_else(|_| Err(cache_error(CloudUnavailable)));
                    match result {
                        Ok(result) => {
                            if result.meta.location != *path {
                                return Err(cache_error(
                                    "Cloud object response changed its requested path",
                                ));
                            }
                            if options
                                .version
                                .as_ref()
                                .is_some_and(|version| result.meta.version.as_ref() != Some(version))
                            {
                                return Err(cache_error(
                                    "Cloud object response changed its requested version",
                                ));
                            }
                            self.control.cloud_succeeded();
                            self.wrap_result(result, epoch, options.head, options.version.is_none())
                        }
                        Err(error) => {
                            self.control.handle_error(&error);
                            if is_offline(&error) {
                                return self.cached(path, &options)?.ok_or_else(offline);
                            }
                            if options.version.is_none()
                                && matches!(
                                    error,
                                    object_store::Error::NotFound { .. }
                                        | object_store::Error::PermissionDenied { .. }
                                        | object_store::Error::Unauthenticated { .. }
                                )
                            {
                                self.control.invalidate(&[path])?;
                            }
                            Err(error)
                        }
                    }
                }
                async fn put_opts(
                    &self,
                    path: &ObjectPath,
                    payload: PutPayload,
                    options: PutOptions,
                ) -> object_store::Result<PutResult> {
                    self.control.scope(path, false)?;
                    let operation = self.control.begin_write(&[path])?;
                    let result = self.inner.put_opts(path, payload, options).await;
                    self.control.finish_write(&operation, &[path], &result)?;
                    result
                }
                async fn put_multipart_opts(
                    &self,
                    path: &ObjectPath,
                    options: PutMultipartOptions,
                ) -> object_store::Result<Box<dyn MultipartUpload>> {
                    self.control.scope(path, false)?;
                    let upload = self
                        .inner
                        .put_multipart_opts(path, options)
                        .await
                        .map_err(|error| {
                            self.control.handle_error(&error);
                            error
                        })?;
                    self.control.cloud_succeeded();
                    Ok(Box::new(CachedUpload {
                        inner: upload,
                        control: self.control.clone(),
                        path: path.clone(),
                    }))
                }
                fn delete_stream(
                    &self,
                    locations: BoxStream<'static, object_store::Result<ObjectPath>>,
                ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
                    let store = self.clone();
                    Box::pin(locations.then(move |path| {
                        let store = store.clone();
                        async move {
                            let path = path?;
                            store.control.scope(&path, false)?;
                            let operation = store.control.begin_write(&[&path])?;
                            let result = store.inner.delete(&path).await;
                            store.control.finish_write(&operation, &[&path], &result)?;
                            result?;
                            Ok(path)
                        }
                    }))
                }
                fn list(
                    &self,
                    prefix: Option<&ObjectPath>,
                ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
                    let store = self.clone();
                    let prefix = prefix.cloned();
                    Box::pin(try_stream! {
                        let key=store.list_key(prefix.as_ref(),false)?;
                        if !store.control.try_cloud_read()? {
                            let bytes=store.control.read(&key)?.ok_or_else(offline)?;
                            let stored: Vec<CachedMeta>=serde_json::from_slice(&bytes).map_err(cache_error)?;
                            for meta in stored { store.control.scope(&meta.path,false)?; yield object_meta(&meta)?; }
                            return;
                        }
                        let epoch=store.control.epoch()?;
                        let mut source=store.inner.list(prefix.as_ref());
                        let mut complete=true;
                        let mut seen=false;
                        let mut data=Vec::<CachedMeta>::new();
                        let mut size=0;
                        while let Some(result)=tokio::time::timeout(CLOUD_PROBE_TIMEOUT,source.next()).await.unwrap_or_else(|_|Some(Err(cache_error(CloudUnavailable)))) {
                            let meta=match result {
                                Ok(meta)=>meta,
                                Err(error)=>{
                                    store.control.handle_error(&error);
                                    if is_offline(&error) && !seen {
                                        let bytes=store.control.read(&key)?.ok_or_else(offline)?;
                                        let stored: Vec<CachedMeta>=serde_json::from_slice(&bytes).map_err(cache_error)?;
                                        for meta in stored { store.control.scope(&meta.path,false)?; yield object_meta(&meta)?; }
                                        return;
                                    }
                                    if !is_offline(&error) { store.control.invalidate::<ObjectPath>(&[])?; }
                                    Err(error)?;
                                    unreachable!()
                                }
                            };
                            store.control.scope(&meta.location,false)?;
                            seen=true;
                            if complete {
                                let stored=cached_meta(&meta,&Attributes::default());
                                size+=serde_json::to_vec(&stored).map_err(cache_error)?.len()+1;
                                if size <= LIST_BYTES { data.push(stored); } else { complete=false; data.clear(); }
                            }
                            yield meta;
                        }
                        if complete { store.control.listing(&key,&serde_json::to_vec(&data).map_err(cache_error)?,epoch)?; }
                        store.control.cloud_succeeded();
                    })
                }
                async fn list_with_delimiter(
                    &self,
                    prefix: Option<&ObjectPath>,
                ) -> object_store::Result<ListResult> {
                    let key = self.list_key(prefix, true)?;
                    if !self.control.try_cloud_read()? {
                        return self.cached_delimiter(&key);
                    }
                    let epoch = self.control.epoch()?;
                    let result =
                        tokio::time::timeout(CLOUD_PROBE_TIMEOUT, self.inner.list_with_delimiter(prefix))
                            .await
                            .unwrap_or_else(|_| Err(cache_error(CloudUnavailable)));
                    match result {
                        Ok(result) => {
                            for meta in &result.objects {
                                self.control.scope(&meta.location, false)?;
                            }
                            for prefix in &result.common_prefixes {
                                self.control.scope(prefix, true)?;
                            }
                            if let Some(snapshot) = delimiter_snapshot(&result) {
                                self.control.listing(&key, &snapshot, epoch)?;
                            }
                            self.control.cloud_succeeded();
                            Ok(result)
                        }
                        Err(error) => {
                            self.control.handle_error(&error);
                            if !is_offline(&error) {
                                self.control.invalidate::<ObjectPath>(&[])?;
                                return Err(error);
                            }
                            self.cached_delimiter(&key)
                        }
                    }
                }
                async fn copy_opts(
                    &self,
                    from: &ObjectPath,
                    to: &ObjectPath,
                    options: CopyOptions,
                ) -> object_store::Result<()> {
                    self.control.scope(from, false)?;
                    self.control.scope(to, false)?;
                    let operation = self.control.begin_write(&[to])?;
                    let result = self.inner.copy_opts(from, to, options).await;
                    self.control.finish_write(&operation, &[to], &result)?;
                    result
                }
                async fn rename_opts(
                    &self,
                    from: &ObjectPath,
                    to: &ObjectPath,
                    options: RenameOptions,
                ) -> object_store::Result<()> {
                    self.control.scope(from, false)?;
                    self.control.scope(to, false)?;
                    let operation = self.control.begin_write(&[from, to])?;
                    let result = self.inner.rename_opts(from, to, options).await;
                    self.control
                        .finish_write(&operation, &[from, to], &result)?;
                    result
                }
            }
            #[derive(Debug)]
            struct CachedUpload {
                inner: Box<dyn MultipartUpload>,
                control: Arc<CacheControl>,
                path: ObjectPath,
            }
            #[async_trait]
            impl MultipartUpload for CachedUpload {
                fn put_part(&mut self, data: PutPayload) -> UploadPart {
                    if let Err(error) = self.control.authorized() {
                        let error = object_store::Error::from(error);
                        return Box::pin(async move { Err(error) });
                    }
                    let result = self.inner.put_part(data);
                    let control = self.control.clone();
                    Box::pin(async move {
                        let result = result.await;
                        if let Err(error) = &result {
                            control.handle_error(error);
                        } else {
                            control.cloud_succeeded();
                        }
                        result
                    })
                }
                async fn complete(&mut self) -> object_store::Result<PutResult> {
                    self.control.scope(&self.path, false)?;
                    let operation = self.control.begin_write(&[&self.path])?;
                    let result = self.inner.complete().await;
                    self.control
                        .finish_write(&operation, &[&self.path], &result)?;
                    result
                }
                async fn abort(&mut self) -> object_store::Result<()> {
                    self.inner.abort().await
                }
            }
        }
    };
}

read_cache!(file_cache, flow_like_storage::object_store, {});
read_cache!(lance_cache, flow_like_storage::lance_object_store, {
    extensions: Default::default(),
});
