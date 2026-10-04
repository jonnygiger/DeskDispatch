use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length valid");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PresignedPost {
    pub url: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug)]
pub enum StorageError {
    PresigningConfig(aws_sdk_s3::presigning::PresigningConfigError),
    GetObject(aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>),
    PutObject(aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>),
    PostPolicy(String),
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
            StorageError::PutObject(e) => {
                write!(f, "Failed to generate presigned PUT request: {}", e)
            }
            StorageError::PostPolicy(e) => {
                write!(f, "Failed to generate presigned POST policy: {}", e)
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::PresigningConfig(e) => Some(e),
            StorageError::GetObject(e) => Some(e),
            StorageError::PutObject(e) => Some(e),
            StorageError::PostPolicy(_) => None,
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

impl From<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>>
    for StorageError
{
    fn from(
        err: aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>,
    ) -> Self {
        StorageError::PutObject(err)
    }
}

/// Storage service wrapper for S3 / RustFS operations.
#[derive(Clone, Debug)]
pub struct StorageService {
    s3_client: Client,
    bucket: String,
    access_key: Option<String>,
    secret_key: Option<String>,
    region: Option<String>,
    endpoint_url: Option<String>,
}

impl StorageService {
    pub fn new(s3_client: Client, bucket: impl Into<String>) -> Self {
        Self {
            s3_client,
            bucket: bucket.into(),
            access_key: None,
            secret_key: None,
            region: None,
            endpoint_url: None,
        }
    }

    pub fn with_credentials(
        mut self,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        region: impl Into<String>,
        endpoint_url: Option<impl Into<String>>,
    ) -> Self {
        self.access_key = Some(access_key.into());
        self.secret_key = Some(secret_key.into());
        self.region = Some(region.into());
        self.endpoint_url = endpoint_url.map(|e| e.into());
        self
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

    /// Generates a presigned PUT URL for worker node file uploads in S3 / RustFS with the given expiration duration.
    pub async fn generate_presigned_put_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        get_presigned_put_url(&self.s3_client, &self.bucket, object_key, expires_in).await
    }

    /// Generates a presigned POST policy for browser-direct file uploads in S3 / RustFS.
    pub async fn generate_presigned_post(
        &self,
        object_key: &str,
        expires_in: Duration,
        max_content_length: u64,
    ) -> Result<PresignedPost, StorageError> {
        let access_key = self.access_key.as_deref().unwrap_or("rustfsadmin");
        let secret_key = self.secret_key.as_deref().unwrap_or("rustfsadminpassword");
        let region = self.region.as_deref().unwrap_or("us-east-1");
        let endpoint_url = self.endpoint_url.as_deref().or(Some("http://localhost:9000"));

        generate_presigned_post_policy(
            &self.bucket,
            object_key,
            expires_in,
            max_content_length,
            access_key,
            secret_key,
            region,
            endpoint_url,
        )
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

/// Standalone helper function to generate a presigned POST policy and SigV4 form fields for browser-direct upload.
pub fn generate_presigned_post_policy(
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    max_content_length: u64,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
) -> Result<PresignedPost, StorageError> {
    let now = Utc::now();
    let duration = chrono::TimeDelta::from_std(expires_in)
        .map_err(|e| StorageError::PostPolicy(format!("Invalid expiration duration: {}", e)))?;
    let expiration_dt = now + duration;

    let date_str = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let expiration_iso = expiration_dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let credential_scope = format!("{}/{}/s3/aws4_request", date_str, region);
    let credential = format!("{}/{}", access_key, credential_scope);

    let policy_json = serde_json::json!({
        "expiration": expiration_iso,
        "conditions": [
            { "bucket": bucket },
            { "key": object_key },
            { "x-amz-algorithm": "AWS4-HMAC-SHA256" },
            { "x-amz-credential": credential },
            { "x-amz-date": amz_date },
            [ "starting-with", "$success_action_redirect", "" ],
            [ "content-length-range", 0, max_content_length ]
        ]
    });

    let policy_str = policy_json.to_string();
    let policy_base64 = base64::engine::general_purpose::STANDARD.encode(policy_str.as_bytes());

    // SigV4 Key Calculation
    let k_secret = format!("AWS4{}", secret_key);
    let k_date = hmac_sha256(k_secret.as_bytes(), date_str.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");

    let signature_bytes = hmac_sha256(&k_signing, policy_base64.as_bytes());
    let signature_hex = hex::encode(signature_bytes);

    let mut fields = HashMap::new();
    fields.insert("key".to_string(), object_key.to_string());
    fields.insert("x-amz-algorithm".to_string(), "AWS4-HMAC-SHA256".to_string());
    fields.insert("x-amz-credential".to_string(), credential);
    fields.insert("x-amz-date".to_string(), amz_date);
    fields.insert("policy".to_string(), policy_base64);
    fields.insert("x-amz-signature".to_string(), signature_hex);

    let url = match endpoint_url {
        Some(endpoint) => {
            let clean_endpoint = endpoint.trim_end_matches('/');
            format!("{}/{}", clean_endpoint, bucket)
        }
        None => format!("https://{}.s3.{}.amazonaws.com", bucket, region),
    };

    Ok(PresignedPost { url, fields })
}

/// Standalone helper function to generate a presigned PUT URL using an S3 Client, bucket, key, and expiration duration.
pub async fn get_presigned_put_url(
    s3_client: &Client,
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
) -> Result<String, StorageError> {
    let presigning_config = PresigningConfig::expires_in(expires_in)?;

    let presigned_req = s3_client
        .put_object()
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

    #[tokio::test]
    async fn test_generate_presigned_put_url() {
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

        let expires_in = Duration::from_secs(600);
        let object_key = "uploads/worker_run_456.png";

        let url = storage
            .generate_presigned_put_url(object_key, expires_in)
            .await
            .unwrap();

        assert!(url.contains("http://localhost:9000/test-bucket/uploads/worker_run_456.png"));
        assert!(url.contains("X-Amz-Expires=600"));
        assert!(url.contains("X-Amz-Signature="));
    }

    #[test]
    fn test_generate_presigned_post_policy() {
        let bucket = "test-bucket";
        let object_key = "bitmaps/button.png";
        let expires_in = Duration::from_secs(900);
        let max_content_length = 10_485_760; // 10MB
        let access_key = "test_access_key";
        let secret_key = "test_secret_key";
        let region = "us-east-1";
        let endpoint_url = Some("http://localhost:9000");

        let presigned_post = generate_presigned_post_policy(
            bucket,
            object_key,
            expires_in,
            max_content_length,
            access_key,
            secret_key,
            region,
            endpoint_url,
        )
        .unwrap();

        assert_eq!(presigned_post.url, "http://localhost:9000/test-bucket");
        assert_eq!(
            presigned_post.fields.get("key").unwrap(),
            "bitmaps/button.png"
        );
        assert_eq!(
            presigned_post.fields.get("x-amz-algorithm").unwrap(),
            "AWS4-HMAC-SHA256"
        );

        let credential = presigned_post.fields.get("x-amz-credential").unwrap();
        assert!(credential.starts_with("test_access_key/"));
        assert!(credential.ends_with("/us-east-1/s3/aws4_request"));

        assert!(presigned_post.fields.contains_key("x-amz-date"));
        assert!(presigned_post.fields.contains_key("policy"));
        assert!(presigned_post.fields.contains_key("x-amz-signature"));

        let policy_base64 = presigned_post.fields.get("policy").unwrap();
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(policy_base64)
            .unwrap();
        let policy_json: serde_json::Value = serde_json::from_slice(&decoded_bytes).unwrap();

        assert!(policy_json.get("expiration").is_some());
        let conditions = policy_json.get("conditions").unwrap().as_array().unwrap();

        assert!(conditions
            .iter()
            .any(|c| c.get("bucket") == Some(&serde_json::json!("test-bucket"))));
        assert!(conditions
            .iter()
            .any(|c| c.get("key") == Some(&serde_json::json!("bitmaps/button.png"))));
    }

    #[tokio::test]
    async fn test_storage_service_generate_presigned_post() {
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
        let storage = StorageService::new(s3_client, "deskdispatch-bucket").with_credentials(
            "my_key",
            "my_secret",
            "us-west-2",
            Some("http://s3.local:9000"),
        );

        let post = storage
            .generate_presigned_post("bitmaps/upload_1.png", Duration::from_secs(300), 5_000_000)
            .await
            .unwrap();

        assert_eq!(post.url, "http://s3.local:9000/deskdispatch-bucket");
        assert_eq!(post.fields.get("key").unwrap(), "bitmaps/upload_1.png");
        assert!(post
            .fields
            .get("x-amz-credential")
            .unwrap()
            .starts_with("my_key/"));
    }
}
