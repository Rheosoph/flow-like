use crate::client::{checked, validate_url};
use crate::{Client, Error, Method, RequestOptions, Result, SignedFile, Value, json};
use bytes::Bytes;

impl Client {
    pub async fn list_files(
        &self,
        app_id: &str,
        prefix: &str,
        user_scope: bool,
        refresh: bool,
    ) -> Result<Value> {
        let mut path = vec!["apps", app_id, "data"];
        if user_scope {
            path.push("user");
        }
        path.push("list");
        self.request(
            Method::POST,
            &path,
            &RequestOptions::new()
                .query("refresh", refresh)
                .json(json!({"prefix":prefix})),
        )
        .await
    }

    pub async fn delete_files(
        &self,
        app_id: &str,
        prefixes: &[String],
        user_scope: bool,
    ) -> Result<()> {
        let mut path = vec!["apps", app_id, "data"];
        if user_scope {
            path.push("user");
        }
        self.request(
            Method::DELETE,
            &path,
            &RequestOptions::new().json(json!({"prefixes":prefixes})),
        )
        .await
    }

    pub async fn sign_uploads(
        &self,
        app_id: &str,
        prefixes: &[String],
        sizes: &[u64],
        user_scope: bool,
    ) -> Result<Vec<SignedFile>> {
        if prefixes.len() != sizes.len() {
            return Err(Error::Configuration(
                "Every upload needs its exact size".into(),
            ));
        }
        let mut path = vec!["apps", app_id, "data"];
        if user_scope {
            path.push("user");
        }
        self.request(
            Method::PUT,
            &path,
            &RequestOptions::new().json(json!({"prefixes":prefixes,"sizes":sizes})),
        )
        .await
    }

    pub async fn sign_downloads(
        &self,
        app_id: &str,
        prefixes: &[String],
        user_scope: bool,
    ) -> Result<Vec<SignedFile>> {
        let mut path = vec!["apps", app_id, "data"];
        if user_scope {
            path.push("user");
        }
        path.push("download");
        self.request(
            Method::POST,
            &path,
            &RequestOptions::new().json(json!({"prefixes":prefixes})),
        )
        .await
    }

    /// Returns provider credentials and a scoped storage path, rather than a transfer URL.
    pub async fn presign_data_access(
        &self,
        app_id: &str,
        user_scope: bool,
        options: &RequestOptions,
    ) -> Result<Value> {
        let mut path = vec!["apps", app_id, "data"];
        if user_scope {
            path.push("user");
        }
        path.push("presign");
        self.request(Method::POST, &path, options).await
    }

    /// Uploads through a signed grant. Platform credentials and redirect following are disabled.
    pub async fn upload_signed(
        &self,
        grant: &SignedFile,
        bytes: Bytes,
        content_type: &str,
    ) -> Result<()> {
        let url = signed_url(grant)?;
        let method = grant.method.as_deref().unwrap_or("PUT");
        let request = match method {
            "PUT" if grant.fields.is_empty() => self
                .http
                .put(url)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(bytes),
            "POST" => {
                let mut form = reqwest::multipart::Form::new();
                for (key, value) in &grant.fields {
                    form = form.text(key.clone(), value.clone());
                }
                let part = reqwest::multipart::Part::bytes(bytes.to_vec())
                    .file_name(
                        grant
                            .prefix
                            .rsplit('/')
                            .next()
                            .unwrap_or("upload")
                            .to_owned(),
                    )
                    .mime_str(content_type)?;
                self.http.post(url).multipart(form.part("file", part))
            }
            _ => {
                return Err(Error::Configuration(
                    "Unsupported signed upload method".into(),
                ));
            }
        };
        async {
            checked(request.send().await?).await?;
            Ok(())
        }
        .await
        .map_err(redact_transfer_error)
    }

    pub async fn download_signed(&self, grant: &SignedFile) -> Result<Bytes> {
        async {
            let response = self.http.get(signed_url(grant)?).send().await?;
            Ok(checked(response).await?.bytes().await?)
        }
        .await
        .map_err(redact_transfer_error)
    }

    pub async fn upload_file(
        &self,
        app_id: &str,
        prefix: &str,
        bytes: Bytes,
        content_type: &str,
        user_scope: bool,
    ) -> Result<SignedFile> {
        let mut grants = self
            .sign_uploads(app_id, &[prefix.into()], &[bytes.len() as u64], user_scope)
            .await?;
        if grants.len() != 1 || grants[0].prefix != prefix {
            return Err(Error::Configuration(
                "Upload grant does not match the requested file".into(),
            ));
        }
        let grant = grants.remove(0);
        self.upload_signed(&grant, bytes, content_type).await?;
        Ok(grant)
    }

    pub async fn download_file(
        &self,
        app_id: &str,
        prefix: &str,
        user_scope: bool,
    ) -> Result<Bytes> {
        let grants = self
            .sign_downloads(app_id, &[prefix.into()], user_scope)
            .await?;
        if grants.len() != 1 || grants[0].prefix != prefix {
            return Err(Error::Configuration(
                "Download grant does not match the requested file".into(),
            ));
        }
        self.download_signed(&grants[0]).await
    }
}

fn redact_transfer_error(error: Error) -> Error {
    match error {
        Error::Transport(error) => Error::Transport(error.without_url()),
        other => other,
    }
}

fn signed_url(grant: &SignedFile) -> Result<reqwest::Url> {
    if let Some(error) = &grant.error {
        return Err(Error::Configuration(format!("File grant failed: {error}")));
    }
    let url = reqwest::Url::parse(
        grant
            .url
            .as_deref()
            .ok_or_else(|| Error::Configuration("File grant has no URL".into()))?,
    )
    .map_err(|_| Error::Configuration("Invalid signed URL".into()))?;
    validate_url(&url)?;
    Ok(url)
}
