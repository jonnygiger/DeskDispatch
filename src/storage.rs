use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::Duration;

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

pub fn encode_hex(bytes: &[u8]) -> String {
    let mut buf = vec![0u8; bytes.len() * 2];
    for (i, &b) in bytes.iter().enumerate() {
        buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
    }
    String::from_utf8(buf).unwrap_or_default()
}

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_base64(bytes: &[u8]) -> String {
    let cap = bytes.len().saturating_add(2) / 3 * 4;
    let mut buf = Vec::with_capacity(cap);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).map_or(0, |&b| b as u32);
        let b2 = chunk.get(2).map_or(0, |&b| b as u32);
        let triple = (b0 << 16) | (b1 << 8) | b2;

        buf.push(BASE64_CHARS[((triple >> 18) & 0x3f) as usize]);
        buf.push(BASE64_CHARS[((triple >> 12) & 0x3f) as usize]);

        if chunk.len() > 1 {
            buf.push(BASE64_CHARS[((triple >> 6) & 0x3f) as usize]);
        } else {
            buf.push(b'=');
        }

        if chunk.len() > 2 {
            buf.push(BASE64_CHARS[(triple & 0x3f) as usize]);
        } else {
            buf.push(b'=');
        }
    }
    String::from_utf8(buf).unwrap_or_default()
}

#[derive(serde::Serialize)]
struct PolicyDocument<'a> {
    expiration: &'a str,
    conditions: Vec<PolicyCondition<'a>>,
}

enum PolicyCondition<'a> {
    Eq(&'static str, &'a str),
    ContentLengthRange(u64, u64),
    StartingWith(&'static str, &'static str),
}

impl serde::Serialize for PolicyCondition<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PolicyCondition::Eq(key, val) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(key, val)?;
                map.end()
            }
            PolicyCondition::ContentLengthRange(min, max) => {
                ("content-length-range", *min, *max).serialize(serializer)
            }
            PolicyCondition::StartingWith(field, prefix) => {
                ("starting-with", *field, *prefix).serialize(serializer)
            }
        }
    }
}

pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let trimmed = input.trim_end_matches('=');
    let padding_len = input.len() - trimmed.len();
    if padding_len > 2 {
        return None;
    }
    if trimmed.is_empty() {
        return Some(Vec::new());
    }

    fn decode_char(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = trimmed.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }

    let mut decoded = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let mut buf = [0u8; 4];
        let mut count = 0;
        for &b in chunk {
            buf[count] = decode_char(b)?;
            count += 1;
        }

        if count >= 2 {
            decoded.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if count >= 3 {
            decoded.push(((buf[1] & 0x0f) << 4) | (buf[2] >> 2));
        }
        if count >= 4 {
            decoded.push(((buf[2] & 0x03) << 6) | buf[3]);
        }
    }

    Some(decoded)
}

/// Zero-dependency HMAC-SHA256 calculation according to RFC 2104 / RFC 4231 using `sha2::Sha256`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_ipad = [0x36u8; 64];
    let mut k_opad = [0x5cu8; 64];

    if key.len() > 64 {
        let hashed_key = Sha256::digest(key);
        for (i, &b) in hashed_key.iter().enumerate() {
            k_ipad[i] ^= b;
            k_opad[i] ^= b;
        }
    } else {
        for (i, &b) in key.iter().enumerate() {
            k_ipad[i] ^= b;
            k_opad[i] ^= b;
        }
    }

    let mut inner_hasher = Sha256::new();
    inner_hasher.update(k_ipad);
    inner_hasher.update(data);
    let inner_hash = inner_hasher.finalize();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(k_opad);
    outer_hasher.update(inner_hash);
    outer_hasher.finalize().into()
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

#[derive(Debug)]
pub enum StorageError {
    PresigningConfig(aws_sdk_s3::presigning::PresigningConfigError),
    GetObject(Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>>),
    PutObject(Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>>),
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
            StorageError::GetObject(e) => Some(e.as_ref()),
            StorageError::PutObject(e) => Some(e.as_ref()),
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
        StorageError::GetObject(Box::new(err))
    }
}

impl From<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>>
    for StorageError
{
    fn from(
        err: aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>,
    ) -> Self {
        StorageError::PutObject(Box::new(err))
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
    public_endpoint_url: Option<String>,
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
        self.access_key = Some(access_key.into());
        self.secret_key = Some(secret_key.into());
        self.region = Some(region.into());
        self.endpoint_url = endpoint_url.map(|e| e.into());
        self
    }

    pub fn with_public_endpoint(mut self, public_endpoint_url: Option<impl Into<String>>) -> Self {
        self.public_endpoint_url = public_endpoint_url.map(|e| e.into());
        self
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn client(&self) -> &Client {
        &self.s3_client
    }

    /// Fetches object bytes from S3 / RustFS.
    #[tracing::instrument(skip(self))]
    pub async fn get_object(&self, object_key: &str) -> Result<Vec<u8>, StorageError> {
        let res = self
            .s3_client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key)
            .send()
            .await?;
        let data =
            res.body.collect().await.map_err(|e| {
                StorageError::PostPolicy(format!("Failed to read object body: {}", e))
            })?;
        Ok(data.to_vec())
    }

    /// Generates a presigned GET URL for an object key in S3 / RustFS with the given expiration duration.
    #[tracing::instrument(skip(self))]
    pub async fn generate_presigned_get_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let url =
            get_presigned_get_url(&self.s3_client, &self.bucket, object_key, expires_in).await?;
        Ok(rewrite_url_with_public_endpoint(
            url,
            self.endpoint_url.as_deref(),
            self.public_endpoint_url.as_deref(),
        ))
    }

    /// Generates a presigned PUT URL for worker node file uploads in S3 / RustFS with the given expiration duration.
    #[tracing::instrument(skip(self))]
    pub async fn generate_presigned_put_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let url =
            get_presigned_put_url(&self.s3_client, &self.bucket, object_key, expires_in).await?;
        Ok(rewrite_url_with_public_endpoint(
            url,
            self.endpoint_url.as_deref(),
            self.public_endpoint_url.as_deref(),
        ))
    }

    /// Generates a presigned POST policy for browser-direct file uploads in S3 / RustFS.
    #[tracing::instrument(skip(self))]
    pub async fn generate_presigned_post(
        &self,
        object_key: &str,
        expires_in: Duration,
        max_content_length: u64,
        success_action_redirect: Option<&str>,
    ) -> Result<PresignedPost, StorageError> {
        let access_key = self.access_key.as_deref().unwrap_or("rustfsadmin");
        let secret_key = self.secret_key.as_deref().unwrap_or("rustfsadminpassword");
        let region = self.region.as_deref().unwrap_or("us-east-1");
        let endpoint_url = self
            .public_endpoint_url
            .as_deref()
            .or(self.endpoint_url.as_deref());

        generate_presigned_post_policy(
            &self.bucket,
            object_key,
            expires_in,
            max_content_length,
            access_key,
            secret_key,
            region,
            endpoint_url,
            success_action_redirect,
        )
    }
}

fn rewrite_url_with_public_endpoint(
    url: String,
    internal_endpoint: Option<&str>,
    public_endpoint: Option<&str>,
) -> String {
    let (Some(internal_ep), Some(public_ep)) = (internal_endpoint, public_endpoint) else {
        return url;
    };
    let clean_internal = internal_ep.trim_end_matches('/');
    let clean_public = public_ep.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix(clean_internal) {
        format!("{}{}", clean_public, rest)
    } else {
        url
    }
}

/// Standalone helper function to generate a presigned GET URL using an S3 Client, bucket, key, and expiration duration.
#[tracing::instrument(skip(s3_client))]
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
    let duration = chrono::TimeDelta::from_std(expires_in)
        .map_err(|e| StorageError::PostPolicy(format!("Invalid expiration duration: {}", e)))?;
    let expiration_dt = now + duration;

    let date_str = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let expiration_iso = expiration_dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let credential_scope = format!("{}/{}/s3/aws4_request", date_str, region);
    let credential = format!("{}/{}", access_key, credential_scope);

    let mut conditions = Vec::with_capacity(7);
    conditions.push(PolicyCondition::Eq("bucket", bucket));
    conditions.push(PolicyCondition::Eq("key", object_key));
    conditions.push(PolicyCondition::Eq("x-amz-algorithm", "AWS4-HMAC-SHA256"));
    conditions.push(PolicyCondition::Eq("x-amz-credential", &credential));
    conditions.push(PolicyCondition::Eq("x-amz-date", &amz_date));
    conditions.push(PolicyCondition::ContentLengthRange(0, max_content_length));

    if let Some(redirect) = success_action_redirect {
        conditions.push(PolicyCondition::Eq("success_action_redirect", redirect));
    } else {
        conditions.push(PolicyCondition::StartingWith(
            "$success_action_redirect",
            "",
        ));
    }

    let policy_doc = PolicyDocument {
        expiration: &expiration_iso,
        conditions,
    };

    let policy_str = serde_json::to_string(&policy_doc)
        .map_err(|e| StorageError::PostPolicy(format!("Failed to serialize policy JSON: {}", e)))?;

    let policy_base64 = encode_base64(policy_str.as_bytes());

    // SigV4 Key Calculation without heap String allocations
    let k_secret_bytes = if secret_key.len() <= 56 {
        let mut buf = [0u8; 64];
        buf[..4].copy_from_slice(b"AWS4");
        buf[4..4 + secret_key.len()].copy_from_slice(secret_key.as_bytes());
        hmac_sha256(&buf[..4 + secret_key.len()], date_str.as_bytes())
    } else {
        let mut vec = Vec::with_capacity(4 + secret_key.len());
        vec.extend_from_slice(b"AWS4");
        vec.extend_from_slice(secret_key.as_bytes());
        hmac_sha256(&vec, date_str.as_bytes())
    };
    let k_date = k_secret_bytes;
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");

    let signature_bytes = hmac_sha256(&k_signing, policy_base64.as_bytes());
    let signature_hex = encode_hex(&signature_bytes);

    let mut ordered_fields = Vec::with_capacity(7);
    ordered_fields.push(("key".to_string(), object_key.to_string()));
    ordered_fields.push((
        "x-amz-algorithm".to_string(),
        "AWS4-HMAC-SHA256".to_string(),
    ));
    ordered_fields.push(("x-amz-credential".to_string(), credential));
    ordered_fields.push(("x-amz-date".to_string(), amz_date));
    if let Some(redirect) = success_action_redirect {
        ordered_fields.push(("success_action_redirect".to_string(), redirect.to_string()));
    }
    ordered_fields.push(("policy".to_string(), policy_base64));
    ordered_fields.push(("x-amz-signature".to_string(), signature_hex));

    let url = match endpoint_url {
        Some(endpoint) => {
            let clean_endpoint = endpoint.trim_end_matches('/');
            format!("{}/{}", clean_endpoint, bucket)
        }
        None => format!("https://{}.s3.{}.amazonaws.com", bucket, region),
    };

    Ok(PresignedPost {
        url,
        ordered_fields,
    })
}

/// Standalone helper function to generate a presigned PUT URL using an S3 Client, bucket, key, and expiration duration.
#[tracing::instrument(skip(s3_client))]
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_and_decode_base64() {
        let input = b"Hello, deskdispatch!";
        let encoded = encode_base64(input);
        assert_eq!(encoded, "SGVsbG8sIGRlc2tkaXNwYXRjaCE=");
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, input);

        // Empty byte test
        assert_eq!(encode_base64(b""), "");
        assert_eq!(decode_base64("").unwrap(), b"");

        // Invalid base64 inputs validation tests
        assert!(decode_base64("A").is_none());
        assert!(decode_base64("AAAAA").is_none());
        assert!(decode_base64("====").is_none());
    }

    #[test]
    fn test_encode_hex() {
        let input = b"\x00\x0f\x10\xff";
        assert_eq!(encode_hex(input), "000f10ff");
    }

    #[test]
    fn test_hmac_sha256_rfc4231_vectors() {
        // RFC 4231 Test Case 1
        let key1 = [0x0bu8; 20];
        let data1 = b"Hi There";
        let expected1 = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
        assert_eq!(encode_hex(&hmac_sha256(&key1, data1)), expected1);

        // RFC 4231 Test Case 2 ("Jefe")
        let key2 = b"Jefe";
        let data2 = b"what do ya want for nothing?";
        let expected2 = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(encode_hex(&hmac_sha256(key2, data2)), expected2);

        // RFC 4231 Test Case 6 (Key > 64 bytes)
        let key6 = [0xaa_u8; 131];
        let data6 = b"Test Using Larger Than Block-Size Key - Hash Key First";
        let expected6 = "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54";
        assert_eq!(encode_hex(&hmac_sha256(&key6, data6)), expected6);
    }

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
            Some("http://localhost:3000/bitmaps/commit?name=button"),
        )
        .unwrap();

        assert_eq!(presigned_post.url, "http://localhost:9000/test-bucket");
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

        assert!(presigned_post.get_field("x-amz-date").is_some());
        assert!(presigned_post.get_field("policy").is_some());
        assert!(presigned_post.get_field("x-amz-signature").is_some());

        let policy_base64 = presigned_post.get_field("policy").unwrap();
        let decoded_bytes = decode_base64(policy_base64).unwrap();
        let policy_json: serde_json::Value = serde_json::from_slice(&decoded_bytes).unwrap();

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
            .generate_presigned_post(
                "bitmaps/upload_1.png",
                Duration::from_secs(300),
                5_000_000,
                Some("http://s3.local:9000/commit"),
            )
            .await
            .unwrap();

        assert_eq!(post.url, "http://s3.local:9000/deskdispatch-bucket");
        assert_eq!(post.get_field("key").unwrap(), "bitmaps/upload_1.png");
        assert!(
            post.get_field("x-amz-credential")
                .unwrap()
                .starts_with("my_key/")
        );
    }

    #[test]
    fn test_rewrite_url_with_public_endpoint() {
        // 1. Matching internal endpoint with trailing slashes handling
        let url = "http://minio.internal:9000/bucket/key.png?sig=123".to_string();
        let rewritten = rewrite_url_with_public_endpoint(
            url.clone(),
            Some("http://minio.internal:9000/"),
            Some("https://s3.public.com/"),
        );
        assert_eq!(rewritten, "https://s3.public.com/bucket/key.png?sig=123");

        // 2. Missing internal or public endpoint
        assert_eq!(
            rewrite_url_with_public_endpoint(url.clone(), None, Some("https://s3.public.com")),
            url
        );
        assert_eq!(
            rewrite_url_with_public_endpoint(url.clone(), Some("http://minio.internal:9000"), None),
            url
        );

        // 3. Non-matching internal endpoint URL
        let non_matching_url = "http://other-host.internal:9000/bucket/key.png".to_string();
        assert_eq!(
            rewrite_url_with_public_endpoint(
                non_matching_url.clone(),
                Some("http://minio.internal:9000"),
                Some("https://s3.public.com")
            ),
            non_matching_url
        );
    }

    #[tokio::test]
    async fn test_storage_service_public_endpoint_rewriting() {
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
            .endpoint_url("http://minio.internal:9000")
            .force_path_style(true)
            .build();

        let s3_client = Client::from_conf(s3_config);
        let storage = StorageService::new(s3_client, "test-bucket")
            .with_credentials(
                "key",
                "secret",
                "us-east-1",
                Some("http://minio.internal:9000"),
            )
            .with_public_endpoint(Some("https://s3.public.com"));

        // GET URL rewriting
        let get_url = storage
            .generate_presigned_get_url("item.png", Duration::from_secs(300))
            .await
            .unwrap();
        assert!(
            get_url.starts_with("https://s3.public.com/test-bucket/item.png"),
            "Expected GET URL to start with public endpoint, got: {}",
            get_url
        );

        // PUT URL rewriting
        let put_url = storage
            .generate_presigned_put_url("item.png", Duration::from_secs(300))
            .await
            .unwrap();
        assert!(
            put_url.starts_with("https://s3.public.com/test-bucket/item.png"),
            "Expected PUT URL to start with public endpoint, got: {}",
            put_url
        );
    }
}
