use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum StorageError {
    PresigningConfig(aws_sdk_s3::presigning::PresigningConfigError),
    GetObject(aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::PresigningConfig(e) => {
                write!(f, "Failed to build presigning configuration: {}", e)
            }
            StorageError::GetObject(e) => {
                write!(f, "Failed to generate presigned GET request: {}", e)
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::PresigningConfig(e) => Some(e),
            StorageError::GetObject(e) => Some(e),
        }
    }
}

impl From<aws_sdk_s3::presigning::PresigningConfigError> for StorageError {
    fn from(err: aws_sdk_s3::presigning::PresigningConfigError) -> Self {
        StorageError::PresigningConfig(err)
    }
}

impl From<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>>
    for StorageError
{
    fn from(
        err: aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>,
    ) -> Self {
        StorageError::GetObject(err)
    }
}

/// Storage service wrapper for S3 / RustFS operations.
#[derive(Clone, Debug)]
pub struct StorageService {
    s3_client: Client,
    bucket: String,
}

impl StorageService {
    pub fn new(s3_client: Client, bucket: impl Into<String>) -> Self {
        Self {
            s3_client,
            bucket: bucket.into(),
        }
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn client(&self) -> &Client {
        &self.s3_client
    }

    /// Generates a presigned GET URL for an object key in S3 / RustFS with the given expiration duration.
    pub async fn generate_presigned_get_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        get_presigned_get_url(&self.s3_client, &self.bucket, object_key, expires_in).await
    }
}

/// Standalone helper function to generate a presigned GET URL using an S3 Client, bucket, key, and expiration duration.
pub async fn get_presigned_get_url(
    s3_client: &Client,
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
) -> Result<String, StorageError> {
    let presigning_config = PresigningConfig::expires_in(expires_in)?;

    let presigned_req = s3_client
        .get_object()
        .bucket(bucket)
        .key(object_key)
        .presigned(presigning_config)
        .await?;

    Ok(presigned_req.uri().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generate_presigned_get_url() {
        let credentials = aws_sdk_s3::config::Credentials::new(
            "test_access_key",
            "test_secret_key",
            None,
            None,
            "static",
        );
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .credentials_provider(credentials)
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .endpoint_url("http://localhost:9000")
            .force_path_style(true)
            .build();

        let s3_client = Client::from_conf(s3_config);
        let storage = StorageService::new(s3_client, "test-bucket");

        assert_eq!(storage.bucket(), "test-bucket");

        let expires_in = Duration::from_secs(900);
        let object_key = "screenshots/step_123.png";

        let url = storage
            .generate_presigned_get_url(object_key, expires_in)
            .await
            .unwrap();

        assert!(url.contains("http://localhost:9000/test-bucket/screenshots/step_123.png"));
        assert!(url.contains("X-Amz-Expires=900"));
        assert!(url.contains("X-Amz-Signature="));
    }
}
