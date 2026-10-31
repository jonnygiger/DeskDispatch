use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, RequestChecksumCalculation};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

pub fn encode_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

pub fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(input).ok()
}

/// HMAC-SHA256 following RFC 2104 / RFC 4231.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

pub fn url_encode_component(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PresignedPost {
    pub url: String,
    pub ordered_fields: Vec<(String, String)>,
}

impl PresignedPost {
    pub fn get_field(&self, name: &str) -> Option<&str> {
        self.ordered_fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageObject {
    pub key: String,
    pub last_modified: Option<DateTime<Utc>>,
    pub size: u64,
}

#[derive(Debug)]
pub enum StorageError {
    PostPolicy(String),
    NotFound(String),
    Request(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::PostPolicy(s) => write!(f, "Failed to generate presigned policy: {}", s),
            StorageError::NotFound(s) => write!(f, "Storage object not found: {}", s),
            StorageError::Request(s) => write!(f, "Storage request error: {}", s),
        }
    }
}

impl std::error::Error for StorageError {}

fn build_client(
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint: Option<&str>,
) -> Client {
    let credentials = Credentials::new(access_key, secret_key, None, None, "deskdispatch");
    let mut builder = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.to_owned()))
        .credentials_provider(credentials)
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .force_path_style(endpoint.is_some());
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Client::from_conf(builder.build())
}

/// Storage service wrapper for S3 / RustFS operations.
#[derive(Clone, Debug)]
pub struct StorageService {
    client: Client,
    public_client: Client,
    bucket: String,
    access_key: String,
    secret_key: String,
    region: String,
    endpoint_url: Option<String>,
    public_endpoint_url: Option<String>,
}

impl StorageService {
    pub fn new(bucket: impl Into<String>) -> Self {
        let (access_key, secret_key, region) = (
            "rustfsadmin".to_string(),
            "rustfsadminpassword".to_string(),
            "us-east-1".to_string(),
        );
        Self {
            client: build_client(&access_key, &secret_key, &region, None),
            public_client: build_client(&access_key, &secret_key, &region, None),
            bucket: bucket.into(),
            access_key,
            secret_key,
            region,
            endpoint_url: None,
            public_endpoint_url: None,
        }
    }

    pub fn with_credentials(
        mut self,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        region: impl Into<String>,
        endpoint_url: Option<impl Into<String>>,
    ) -> Self {
        self.access_key = access_key.into();
        self.secret_key = secret_key.into();
        self.region = region.into();
        self.endpoint_url = endpoint_url.map(|e| e.into());
        self.rebuild_clients();
        self
    }

    pub fn with_public_endpoint(mut self, public_endpoint_url: Option<impl Into<String>>) -> Self {
        self.public_endpoint_url = public_endpoint_url.map(|e| e.into());
        self.rebuild_clients();
        self
    }

    fn rebuild_clients(&mut self) {
        self.client = build_client(
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        );
        self.public_client = build_client(
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.public_endpoint_url
                .as_deref()
                .or(self.endpoint_url.as_deref()),
        );
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_object(&self, object_key: &str) -> Result<Vec<u8>, StorageError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key)
            .send()
            .await
            .map_err(|e| match e.as_service_error() {
                Some(err) if err.is_no_such_key() => {
                    StorageError::NotFound(object_key.to_string())
                }
                _ => StorageError::Request(e.to_string()),
            })?;
        let bytes = output
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Request(e.to_string()))?;
        Ok(bytes.into_bytes().to_vec())
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_object_range(
        &self,
        object_key: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, StorageError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key)
            .range(format!("bytes={}-{}", start, end))
            .send()
            .await
            .map_err(|e| match e.as_service_error() {
                Some(err) if err.is_no_such_key() => {
                    StorageError::NotFound(object_key.to_string())
                }
                _ => StorageError::Request(e.to_string()),
            })?;
        let bytes = output
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Request(e.to_string()))?;
        Ok(bytes.into_bytes().to_vec())
    }

    #[tracing::instrument(skip(self, bytes))]
    pub async fn put_object(
        &self,
        object_key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(object_key)
            .content_type(content_type)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map(|_| ())
            .map_err(|e| StorageError::Request(e.to_string()))
    }

    #[tracing::instrument(skip(self))]
    pub async fn delete_object(&self, object_key: &str) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(object_key)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| StorageError::Request(e.to_string()))
    }

    #[tracing::instrument(skip(self))]
    pub async fn head_bucket(&self) -> Result<bool, StorageError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(true),
            Err(e) => match e.raw_response().map(|r| r.status().as_u16()) {
                Some(404) => Ok(false),
                _ => Err(StorageError::Request(e.to_string())),
            },
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn create_bucket(&self) -> Result<(), StorageError> {
        self.client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| StorageError::Request(e.to_string()))
    }

    #[tracing::instrument(skip(self))]
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, StorageError> {
        let mut objects = Vec::new();
        let mut continuation_token = None;
        loop {
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);
            if let Some(token) = &continuation_token {
                request = request.continuation_token(token);
            }
            let output = request
                .send()
                .await
                .map_err(|e| StorageError::Request(e.to_string()))?;

            objects.extend(output.contents().iter().filter_map(|object| {
                let key = object.key()?.to_string();
                let last_modified = object
                    .last_modified()
                    .and_then(|dt| Utc.timestamp_opt(dt.secs(), dt.subsec_nanos()).single());
                Some(StorageObject {
                    key,
                    last_modified,
                    size: object.size().unwrap_or_default().max(0) as u64,
                })
            }));

            if output.is_truncated().unwrap_or(false) {
                continuation_token = output.next_continuation_token().map(str::to_owned);
            } else {
                break;
            }
        }
        Ok(objects)
    }

    pub async fn generate_presigned_get_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let config = PresigningConfig::expires_in(expires_in)
            .map_err(|e| StorageError::Request(e.to_string()))?;
        self.public_client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key)
            .presigned(config)
            .await
            .map(|request| request.uri().to_string())
            .map_err(|e| StorageError::Request(e.to_string()))
    }

    pub async fn generate_presigned_put_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let config = PresigningConfig::expires_in(expires_in)
            .map_err(|e| StorageError::Request(e.to_string()))?;
        self.public_client
            .put_object()
            .bucket(&self.bucket)
            .key(object_key)
            .presigned(config)
            .await
            .map(|request| request.uri().to_string())
            .map_err(|e| StorageError::Request(e.to_string()))
    }

    pub fn generate_presigned_post(
        &self,
        object_key: &str,
        expires_in: Duration,
        max_content_length: u64,
        success_action_redirect: Option<&str>,
    ) -> Result<PresignedPost, StorageError> {
        generate_presigned_post_policy(
            &self.bucket,
            object_key,
            expires_in,
            max_content_length,
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.public_endpoint_url
                .as_deref()
                .or(self.endpoint_url.as_deref()),
            success_action_redirect,
        )
    }
}

fn derive_signing_key(secret_key: &str, date_str: &str, region: &str) -> [u8; 32] {
    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date_str.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    hmac_sha256(&k_service, b"aws4_request")
}

#[tracing::instrument(skip(secret_key))]
#[allow(clippy::too_many_arguments)]
pub fn generate_presigned_post_policy(
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    max_content_length: u64,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
    success_action_redirect: Option<&str>,
) -> Result<PresignedPost, StorageError> {
    let now = Utc::now();
    let expiration = now
        + chrono::TimeDelta::from_std(expires_in)
            .map_err(|e| StorageError::PostPolicy(e.to_string()))?;
    let date_str = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let credential = format!("{access_key}/{date_str}/{region}/s3/aws4_request");

    let conditions = serde_json::json!([
        {"bucket": bucket},
        {"key": object_key},
        {"x-amz-algorithm": "AWS4-HMAC-SHA256"},
        {"x-amz-credential": credential},
        {"x-amz-date": amz_date},
        ["content-length-range", 0, max_content_length],
        ["starts-with", "$success_action_redirect", success_action_redirect.unwrap_or("")]
    ]);
    let policy = serde_json::json!({
        "expiration": expiration.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "conditions": conditions,
    });
    let policy_base64 = encode_base64(
        serde_json::to_string(&policy)
            .map_err(|e| StorageError::PostPolicy(e.to_string()))?
            .as_bytes(),
    );
    let signature = encode_hex(&hmac_sha256(
        &derive_signing_key(secret_key, &date_str, region),
        policy_base64.as_bytes(),
    ));

    let mut ordered_fields = vec![
        ("key".to_string(), object_key.to_string()),
        (
            "x-amz-algorithm".to_string(),
            "AWS4-HMAC-SHA256".to_string(),
        ),
        ("x-amz-credential".to_string(), credential),
        ("x-amz-date".to_string(), amz_date),
    ];
    if let Some(redirect) = success_action_redirect {
        ordered_fields.push(("success_action_redirect".to_string(), redirect.to_string()));
    }
    ordered_fields.push(("policy".to_string(), policy_base64));
    ordered_fields.push(("x-amz-signature".to_string(), signature));

    let url = match endpoint_url {
        Some(endpoint) => format!("{}/{}", endpoint.trim_end_matches('/'), bucket),
        None => format!("https://{}.s3.{}.amazonaws.com", bucket, region),
    };

    Ok(PresignedPost {
        url,
        ordered_fields,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_and_decode_base64() {
        let input = b"Hello, deskdispatch!";
        let encoded = encode_base64(input);
        assert_eq!(encoded, "SGVsbG8sIGRlc2tkaXNwYXRjaCE=");
        assert_eq!(decode_base64(&encoded).unwrap(), input);
        assert_eq!(encode_base64(b""), "");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("A").is_none());
        assert!(decode_base64("AAAAA").is_none());
        assert!(decode_base64("====").is_none());
    }

    #[test]
    fn test_encode_hex_and_hmac_sha256_rfc4231_vectors() {
        assert_eq!(encode_hex(b"\x00\x0f\x10\xff"), "000f10ff");
        let key1 = [0x0bu8; 20];
        let expected1 = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
        assert_eq!(encode_hex(&hmac_sha256(&key1, b"Hi There")), expected1);

        let expected2 = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(
            encode_hex(&hmac_sha256(
                b"Jefe",
                b"what do ya want for nothing?"
            )),
            expected2
        );
    }

    fn test_storage(public_endpoint: &str) -> StorageService {
        StorageService::new("test-bucket")
            .with_credentials(
                "test_access_key",
                "test_secret_key",
                "us-east-1",
                Some("http://rustfs:9100"),
            )
            .with_public_endpoint(Some(public_endpoint))
    }

    #[tokio::test]
    async fn test_generate_presigned_get_url() {
        let storage = test_storage("http://localhost:9100");
        let url = storage
            .generate_presigned_get_url("screenshots/step_123.png", Duration::from_secs(900))
            .await
            .unwrap();

        assert!(url.contains("http://localhost:9100/test-bucket/screenshots/step_123.png"));
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Credential=test_access_key%2F"));
        assert!(url.contains("X-Amz-Expires=900"));
        assert!(url.contains("X-Amz-SignedHeaders=host"));
        assert!(url.contains("X-Amz-Signature="));
    }

    #[tokio::test]
    async fn test_generate_presigned_put_url() {
        let storage = test_storage("http://localhost:9100");
        let url = storage
            .generate_presigned_put_url("uploads/worker_run_456.png", Duration::from_secs(600))
            .await
            .unwrap();

        assert!(url.contains("http://localhost:9100/test-bucket/uploads/worker_run_456.png"));
        assert!(url.contains("X-Amz-Expires=600"));
        assert!(url.contains("X-Amz-Signature="));
    }

    #[test]
    fn test_generate_presigned_post_policy() {
        let presigned_post = generate_presigned_post_policy(
            "test-bucket",
            "bitmaps/button.png",
            Duration::from_secs(900),
            10_485_760,
            "test_access_key",
            "test_secret_key",
            "us-east-1",
            Some("http://localhost:9100"),
            Some("http://localhost:3000/bitmaps/commit?name=button"),
        )
        .unwrap();

        assert_eq!(presigned_post.url, "http://localhost:9100/test-bucket");
        assert_eq!(
            presigned_post.get_field("key").unwrap(),
            "bitmaps/button.png"
        );
        assert_eq!(
            presigned_post.get_field("x-amz-algorithm").unwrap(),
            "AWS4-HMAC-SHA256"
        );
        let credential = presigned_post.get_field("x-amz-credential").unwrap();
        assert!(credential.starts_with("test_access_key/"));
        assert!(credential.ends_with("/us-east-1/s3/aws4_request"));
        assert!(presigned_post.get_field("policy").is_some());
        assert!(presigned_post.get_field("x-amz-signature").is_some());

        let policy_json: serde_json::Value = serde_json::from_slice(
            &decode_base64(presigned_post.get_field("policy").unwrap()).unwrap(),
        )
        .unwrap();
        assert!(policy_json.get("expiration").is_some());
        let conditions = policy_json.get("conditions").unwrap().as_array().unwrap();
        assert!(
            conditions
                .iter()
                .any(|c| c.get("bucket") == Some(&serde_json::json!("test-bucket")))
        );
        assert!(
            conditions
                .iter()
                .any(|c| c.get("key") == Some(&serde_json::json!("bitmaps/button.png")))
        );
    }
}
