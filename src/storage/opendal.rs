use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use aws_config::meta::credentials::CredentialsProviderChain;
use aws_credential_types::provider::ProvideCredentials;
use chrono::Timelike;
use hyper::Uri;
use opendal::Operator;
use opendal::layers::{LoggingLayer, TracingLayer};
use opendal::services::{Fs, S3};
use reqsign::AwsCredentialLoad;
use reqwest::Client;
use tokio::sync::OnceCell;
use tracing::info;

use super::{FileOrStream, StorageError, StorageObject, StorageProvider};

pub struct OpenDalStorageProvider {
    path: PathBuf,
}

impl OpenDalStorageProvider {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl From<opendal::Error> for StorageError {
    fn from(value: opendal::Error) -> Self {
        match value.kind() {
            opendal::ErrorKind::NotFound => StorageError::NotFound,
            _ => StorageError::Internal(value),
        }
    }
}

impl StorageProvider for OpenDalStorageProvider {
    fn open(
        &self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<StorageObject, StorageError>> + Send + 'static>> {
        Box::pin(open(self.path.clone(), id.to_string()))
    }
}

#[derive(Default)]
pub struct AwsCredentialProvider {
    chain: OnceCell<CredentialsProviderChain>,
}

#[async_trait::async_trait]
impl AwsCredentialLoad for AwsCredentialProvider {
    async fn load_credential(
        &self,
        _: Client,
    ) -> Result<Option<reqsign::AwsCredential>, anyhow::Error> {
        let provider = self
            .chain
            .get_or_init(|| async { CredentialsProviderChain::default_provider().await })
            .await;

        let credentials = provider.provide_credentials().await?;

        Ok(Some(reqsign::AwsCredential {
            access_key_id: credentials.access_key_id().into(),
            secret_access_key: credentials.secret_access_key().into(),
            session_token: credentials.session_token().map(|value| value.to_string()),
            expires_in: credentials.expiry().map(|time| time.into()),
        }))
    }
}

#[tracing::instrument]
async fn open(local_root: PathBuf, path: String) -> Result<StorageObject, StorageError> {
    let (operator, path) = match path.parse::<Uri>() {
        Ok(uri) if uri.scheme_str() == Some("s3") => {
            let region = uri
                .host()
                .ok_or(StorageError::Other("invalid S3 URI specification".into()))?;
            let (_, bucket_and_path) = uri
                .path()
                .split_once('/')
                .ok_or(StorageError::Other("invalid S3 URI specification".into()))?;
            let (bucket, bucket_key) = bucket_and_path
                .split_once('/')
                .ok_or(StorageError::Other("invalid S3 bucket/key specification".into()))?;

            info!(
                region = region,
                bucket = bucket,
                bucket_key = bucket_key,
                "recognised image path as S3 object"
            );

            (
                Operator::new(
                    S3::default()
                        .disable_ec2_metadata()
                        .disable_config_load()
                        .region(region)
                        .enable_virtual_host_style()
                        .bucket(bucket)
                        .customized_credential_load(Box::new(AwsCredentialProvider::default())),
                )?
                .layer(TracingLayer)
                .layer(LoggingLayer::default())
                .finish(),
                bucket_key.to_string(),
            )
        }
        _ => (
            Operator::new(
                Fs::default().root(local_root.to_str().expect("invalid root path provided")),
            )?
            .layer(TracingLayer)
            .finish(),
            path.to_string(),
        ),
    };

    let stat = operator.stat(&path).await?;
    let reader = operator
        .reader(&path)
        .await?
        .into_futures_async_read(..)
        .await?;

    Ok(StorageObject {
        name: Some(path),
        content: FileOrStream::Stream(Box::new(reader)),
        last_modified: stat
            .last_modified()
            .map(|utc| utc.with_nanosecond(0).unwrap().into()),
    })
}
