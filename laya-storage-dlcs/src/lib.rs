use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use aws_config::meta::credentials::CredentialsProviderChain;
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::provider::error::CredentialsError;
use chrono::Timelike;
use hyper::Uri;
use laya_storage::{FileOrStream, StorageError, StorageObject, StorageProvider};
use mediatype::MediaTypeBuf;
use opendal::Operator;
use opendal::layers::{MimeGuessLayer, RetryLayer, TimeoutLayer};
use opendal::services::{Fs, S3};
use reqsign::AwsCredentialLoad;
use reqwest::Client;
use tokio::sync::{OnceCell, RwLock};
use tracing::info;

pub struct DlcsStorageProvider {
    local_path: PathBuf,
    credentials: RwLock<Option<Credentials>>,
    credential_provider_chain: OnceCell<CredentialsProviderChain>,
}

impl DlcsStorageProvider {
    pub fn new(path: PathBuf) -> Self {
        Self {
            local_path: path,
            credentials: RwLock::default(),
            credential_provider_chain: OnceCell::new(),
        }
    }
}

fn convert_opendal_error(value: opendal::Error) -> StorageError {
    match value.kind() {
        opendal::ErrorKind::NotFound => StorageError::NotFound,
        _ => StorageError::Internal(Box::new(value)),
    }
}

impl StorageProvider for DlcsStorageProvider {
    fn open<'a, 'fut>(
        &'a self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<StorageObject, StorageError>> + Send + 'fut>>
    where
        'a: 'fut,
    {
        Box::pin(open(self, id.to_string()))
    }

    fn healthcheck(&self) -> Pin<Box<dyn Future<Output = Result<(), StorageError>> + Send>> {
        Box::pin(futures::future::ready(Ok(())))
    }
}

impl DlcsStorageProvider {
    async fn get_aws_credentials(&self) -> Result<Credentials, CredentialsError> {
        if let Some(credentials) = &*self.credentials.read().await {
            if credentials.expiry().is_some_and(|expiry| SystemTime::now() < expiry) || credentials.expiry().is_none() {
                info!("reusing cached credentials");
                return Ok(credentials.clone());
            } else {
                info!("cached credentials expired at {:?}", credentials.expiry());
            }
        }

        let credential_provider = self
            .credential_provider_chain
            .get_or_init(async || CredentialsProviderChain::default_provider().await)
            .await;

        let credentials = credential_provider.provide_credentials().await?;

        let mut current_creds = self.credentials.write().await;
        current_creds.replace(credentials.clone());

        Ok(credentials)
    }
}

fn map_creds(sdk_credentials: Credentials) -> reqsign::AwsCredential {
    reqsign::AwsCredential {
        access_key_id: sdk_credentials.access_key_id().into(),
        secret_access_key: sdk_credentials.secret_access_key().into(),
        session_token: sdk_credentials.session_token().map(|value| value.to_string()),
        expires_in: sdk_credentials.expiry().map(|time| time.into()),
    }
}

pub struct FixedCredentialLoader(reqsign::AwsCredential);

impl AwsCredentialLoad for FixedCredentialLoader {
    fn load_credential<'a, 'fut>(
        &'a self,
        _client: Client,
    ) -> ::core::pin::Pin<Box<dyn Future<Output = anyhow::Result<Option<reqsign::AwsCredential>>> + Send + 'fut>>
    where
        'a: 'fut,
    {
        Box::pin(futures::future::ready(Ok(Some(self.0.clone()))))
    }
}

#[tracing::instrument(skip(storage), err)]
async fn open(storage: &DlcsStorageProvider, path: String) -> Result<StorageObject, StorageError> {
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
                        .customized_credential_load(Box::new(FixedCredentialLoader(
                            storage
                                .get_aws_credentials()
                                .await
                                .map(map_creds)
                                .map_err(|_| StorageError::AccessDenied)?,
                        ))),
                )
                .map_err(convert_opendal_error)?
                .layer(MimeGuessLayer::default())
                .layer(
                    TimeoutLayer::new()
                        .with_io_timeout(Duration::from_secs(1))
                        .with_timeout(Duration::from_secs(5)),
                )
                .layer(RetryLayer::new())
                .finish(),
                bucket_key.to_string(),
            )
        }
        _ => (
            Operator::new(Fs::default().root(storage.local_path.to_str().expect("invalid root path provided")))
                .map_err(convert_opendal_error)?
                .layer(MimeGuessLayer::default())
                .layer(RetryLayer::new())
                .finish(),
            path.to_string(),
        ),
    };

    let stat = operator.stat(&path).await.map_err(convert_opendal_error)?;
    let last_modified = stat
        .last_modified()
        .and_then(|date| date.with_nanosecond(0))
        .map(SystemTime::from);

    let media_type = stat
        .content_type()
        .and_then(|value| value.parse::<MediaTypeBuf>().ok())
        .ok_or(StorageError::UnknownFormat)?;

    let reader = operator
        .reader_with(&path)
        .concurrent(8)
        .await
        .map_err(convert_opendal_error)?
        .into_futures_async_read(..)
        .await
        .map_err(convert_opendal_error)?;

    Ok(StorageObject {
        name: Some(path),
        content: FileOrStream::Stream(Box::new(reader)),
        last_modified,
        media_type,
    })
}
