use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::Duration;

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
const HEX_CHARS_UPPER: &[u8; 16] = b"0123456789ABCDEF";

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

pub fn url_encode_component(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push('%');
                result.push(HEX_CHARS_UPPER[(b >> 4) as usize] as char);
                result.push(HEX_CHARS_UPPER[(b & 0x0f) as usize] as char);
            }
        }
    }
    result
}

/// Derives the 4-tier AWS SigV4 HMAC-SHA256 signing key.
fn derive_signing_key(secret_key: &str, date_str: &str, region: &str) -> [u8; 32] {
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
    hmac_sha256(&k_service, b"aws4_request")
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
    Http(reqwest::Error),
    PostPolicy(String),
    NotFound(String),
    Request(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Http(e) => write!(f, "HTTP storage error: {}", e),
            StorageError::PostPolicy(s) => write!(f, "Failed to generate presigned policy: {}", s),
            StorageError::NotFound(s) => write!(f, "Storage object not found: {}", s),
            StorageError::Request(s) => write!(f, "Storage request error: {}", s),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for StorageError {
    fn from(err: reqwest::Error) -> Self {
        StorageError::Http(err)
    }
}

/// Standalone helper to generate SigV4 presigned URLs for GET, PUT, DELETE, or HEAD requests.
#[allow(clippy::too_many_arguments)]
pub fn generate_presigned_url(
    method: &str,
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
) -> Result<String, StorageError> {
    generate_presigned_url_with_query(
        method,
        bucket,
        object_key,
        expires_in,
        access_key,
        secret_key,
        region,
        endpoint_url,
        &[],
    )
}

/// Helper to generate SigV4 presigned URLs including optional extra query parameters.
#[allow(clippy::too_many_arguments)]
pub fn generate_presigned_url_with_query(
    method: &str,
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
    extra_params: &[(&str, &str)],
) -> Result<String, StorageError> {
    let now = Utc::now();
    let date_str = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let expires_secs = expires_in.as_secs().to_string();

    let credential_scope = format!("{}/{}/s3/aws4_request", date_str, region);
    let credential = format!("{}/{}", access_key, credential_scope);

    let clean_key = object_key.trim_start_matches('/');

    let (host, canonical_uri, base_url) = match endpoint_url {
        Some(endpoint) => {
            let clean_endpoint = endpoint.trim_end_matches('/');
            let parsed_url = url::Url::parse(clean_endpoint).map_err(|e| {
                StorageError::Request(format!("Invalid endpoint URL {}: {}", clean_endpoint, e))
            })?;
            let host = parsed_url
                .host_str()
                .ok_or_else(|| StorageError::Request("Missing host in endpoint".to_string()))?;
            let port_part = parsed_url
                .port()
                .map(|p| format!(":{}", p))
                .unwrap_or_default();
            let full_host = format!("{}{}", host, port_part);
            let uri = if clean_key.is_empty() {
                format!("/{}", bucket)
            } else {
                format!("/{}/{}", bucket, clean_key)
            };
            let base = format!("{}/{}", clean_endpoint, bucket);
            (full_host, uri, base)
        }
        None => {
            let host = format!("{}.s3.{}.amazonaws.com", bucket, region);
            let uri = if clean_key.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", clean_key)
            };
            let base = format!("https://{}", host);
            (host, uri, base)
        }
    };

    let mut query_params = vec![
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential", credential),
        ("X-Amz-Date", amz_date.clone()),
        ("X-Amz-Expires", expires_secs),
        ("X-Amz-SignedHeaders", "host".to_string()),
    ];

    for &(k, v) in extra_params {
        query_params.push((k, v.to_string()));
    }

    let mut encoded_params: Vec<(String, String)> = query_params
        .into_iter()
        .map(|(k, v)| (url_encode_component(k), url_encode_component(&v)))
        .collect();
    encoded_params.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut canonical_query_string = String::with_capacity(
        encoded_params
            .iter()
            .map(|(k, v)| k.len() + 1 + v.len() + 1)
            .sum::<usize>()
            .saturating_sub(1),
    );
    for (idx, (k, v)) in encoded_params.iter().enumerate() {
        if idx > 0 {
            canonical_query_string.push('&');
        }
        canonical_query_string.push_str(k);
        canonical_query_string.push('=');
        canonical_query_string.push_str(v);
    }

    let canonical_headers = format!("host:{}\n", host);
    let signed_headers = "host";
    let payload_hash = "UNSIGNED-PAYLOAD";

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.to_uppercase(),
        canonical_uri,
        canonical_query_string,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    let canonical_request_hash = encode_hex(&Sha256::digest(canonical_request.as_bytes()));

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date, credential_scope, canonical_request_hash
    );

    let k_signing = derive_signing_key(secret_key, &date_str, region);
    let signature_bytes = hmac_sha256(&k_signing, string_to_sign.as_bytes());
    let signature_hex = encode_hex(&signature_bytes);

    let full_url = if clean_key.is_empty() {
        format!(
            "{}?{}&X-Amz-Signature={}",
            base_url, canonical_query_string, signature_hex
        )
    } else {
        format!(
            "{}/{}?{}&X-Amz-Signature={}",
            base_url, clean_key, canonical_query_string, signature_hex
        )
    };

    Ok(full_url)
}

pub fn get_presigned_get_url_standalone(
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
) -> Result<String, StorageError> {
    generate_presigned_url(
        "GET",
        bucket,
        object_key,
        expires_in,
        access_key,
        secret_key,
        region,
        endpoint_url,
    )
}

pub fn get_presigned_put_url_standalone(
    bucket: &str,
    object_key: &str,
    expires_in: Duration,
    access_key: &str,
    secret_key: &str,
    region: &str,
    endpoint_url: Option<&str>,
) -> Result<String, StorageError> {
    generate_presigned_url(
        "PUT",
        bucket,
        object_key,
        expires_in,
        access_key,
        secret_key,
        region,
        endpoint_url,
    )
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

    let k_signing = derive_signing_key(secret_key, &date_str, region);
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

#[cfg(test)]
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

/// Storage service wrapper for S3 / RustFS operations.
#[derive(Clone, Debug)]
pub struct StorageService {
    http_client: reqwest::Client,
    bucket: String,
    access_key: String,
    secret_key: String,
    region: String,
    endpoint_url: Option<String>,
    public_endpoint_url: Option<String>,
}

impl StorageService {
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            bucket: bucket.into(),
            access_key: "rustfsadmin".to_string(),
            secret_key: "rustfsadminpassword".to_string(),
            region: "us-east-1".to_string(),
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
        self
    }

    pub fn with_public_endpoint(mut self, public_endpoint_url: Option<impl Into<String>>) -> Self {
        self.public_endpoint_url = public_endpoint_url.map(|e| e.into());
        self
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Fetches object bytes from S3 / RustFS.
    #[tracing::instrument(skip(self))]
    pub async fn get_object(&self, object_key: &str) -> Result<Vec<u8>, StorageError> {
        let presigned_url = generate_presigned_url(
            "GET",
            &self.bucket,
            object_key,
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self.http_client.get(&presigned_url).send().await?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound(object_key.to_string()));
        }
        if !status.is_success() {
            return Err(StorageError::Request(format!(
                "GET {} returned HTTP {}",
                object_key, status
            )));
        }

        let bytes = res.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Fetches a byte range of an object from S3 / RustFS.
    #[tracing::instrument(skip(self))]
    pub async fn get_object_range(
        &self,
        object_key: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, StorageError> {
        let presigned_url = generate_presigned_url(
            "GET",
            &self.bucket,
            object_key,
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self
            .http_client
            .get(&presigned_url)
            .header("Range", format!("bytes={}-{}", start, end))
            .send()
            .await?;

        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound(object_key.to_string()));
        }
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(StorageError::Request(format!(
                "GET range {} returned HTTP {}",
                object_key, status
            )));
        }

        let bytes = res.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Uploads object bytes to S3 / RustFS.
    #[tracing::instrument(skip(self, bytes))]
    pub async fn put_object(
        &self,
        object_key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        let presigned_url = generate_presigned_url(
            "PUT",
            &self.bucket,
            object_key,
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self
            .http_client
            .put(&presigned_url)
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(StorageError::Request(format!(
                "PUT {} returned HTTP {}",
                object_key,
                res.status()
            )));
        }

        Ok(())
    }

    /// Deletes an object from S3 / RustFS.
    #[tracing::instrument(skip(self))]
    pub async fn delete_object(&self, object_key: &str) -> Result<(), StorageError> {
        let presigned_url = generate_presigned_url(
            "DELETE",
            &self.bucket,
            object_key,
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self.http_client.delete(&presigned_url).send().await?;
        let status = res.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(StorageError::Request(format!(
                "DELETE {} returned HTTP {}",
                object_key, status
            )))
        }
    }

    /// Checks if bucket exists.
    #[tracing::instrument(skip(self))]
    pub async fn head_bucket(&self) -> Result<bool, StorageError> {
        let presigned_url = generate_presigned_url(
            "GET",
            &self.bucket,
            "",
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self.http_client.get(&presigned_url).send().await?;
        Ok(res.status().is_success())
    }

    /// Creates bucket if it does not exist.
    #[tracing::instrument(skip(self))]
    pub async fn create_bucket(&self) -> Result<(), StorageError> {
        let presigned_url = generate_presigned_url(
            "PUT",
            &self.bucket,
            "",
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
        )?;

        let res = self.http_client.put(&presigned_url).send().await?;
        if res.status().is_success() || res.status() == reqwest::StatusCode::CONFLICT {
            Ok(())
        } else {
            Err(StorageError::Request(format!(
                "Create bucket {} returned HTTP {}",
                self.bucket,
                res.status()
            )))
        }
    }

    /// Lists objects in the bucket matching prefix.
    #[tracing::instrument(skip(self))]
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, StorageError> {
        let presigned_url = generate_presigned_url_with_query(
            "GET",
            &self.bucket,
            "",
            Duration::from_secs(900),
            &self.access_key,
            &self.secret_key,
            &self.region,
            self.endpoint_url.as_deref(),
            &[("list-type", "2"), ("prefix", prefix)],
        )?;

        let res = self.http_client.get(&presigned_url).send().await?;
        if !res.status().is_success() {
            return Ok(Vec::new());
        }

        let body_text = res.text().await?;
        let objects = parse_list_objects_v2_xml(&body_text);
        Ok(objects)
    }

    /// Generates a presigned GET URL for an object key in S3 / RustFS with the given expiration duration.
    #[tracing::instrument(skip(self))]
    pub async fn generate_presigned_get_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let endpoint_url = self
            .public_endpoint_url
            .as_deref()
            .or(self.endpoint_url.as_deref());
        generate_presigned_url(
            "GET",
            &self.bucket,
            object_key,
            expires_in,
            &self.access_key,
            &self.secret_key,
            &self.region,
            endpoint_url,
        )
    }

    /// Generates a presigned PUT URL for worker node file uploads in S3 / RustFS with the given expiration duration.
    #[tracing::instrument(skip(self))]
    pub async fn generate_presigned_put_url(
        &self,
        object_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        let endpoint_url = self
            .public_endpoint_url
            .as_deref()
            .or(self.endpoint_url.as_deref());
        generate_presigned_url(
            "PUT",
            &self.bucket,
            object_key,
            expires_in,
            &self.access_key,
            &self.secret_key,
            &self.region,
            endpoint_url,
        )
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
        let endpoint_url = self
            .public_endpoint_url
            .as_deref()
            .or(self.endpoint_url.as_deref());

        generate_presigned_post_policy(
            &self.bucket,
            object_key,
            expires_in,
            max_content_length,
            &self.access_key,
            &self.secret_key,
            &self.region,
            endpoint_url,
            success_action_redirect,
        )
    }
}

pub fn parse_list_objects_v2_xml(xml: &str) -> Vec<StorageObject> {
    let mut objects = Vec::new();
    let mut search = xml;

    while let Some(start_idx) = search.find("<Contents>") {
        let rest = &search[start_idx + "<Contents>".len()..];
        let end_idx = match rest.find("</Contents>") {
            Some(idx) => idx,
            None => break,
        };
        let block = &rest[..end_idx];
        search = &rest[end_idx + "</Contents>".len()..];

        let Some(key_str) = extract_xml_tag_slice(block, "Key") else {
            continue;
        };
        if key_str.is_empty() {
            continue;
        }

        let size = extract_xml_tag_slice(block, "Size")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let last_modified = extract_xml_tag_slice(block, "LastModified").and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        objects.push(StorageObject {
            key: key_str.to_string(),
            last_modified,
            size,
        });
    }

    objects
}

fn extract_xml_tag_slice<'a>(xml_block: &'a str, tag_name: &str) -> Option<&'a str> {
    let bytes = xml_block.as_bytes();
    let tag_bytes = tag_name.as_bytes();

    let mut start = 0;
    while start < bytes.len() {
        let rel_open = xml_block[start..].find('<')?;
        let open_idx = start + rel_open;
        let rest = &xml_block[open_idx + 1..];
        if rest.as_bytes().starts_with(tag_bytes)
            && rest.as_bytes().get(tag_bytes.len()) == Some(&b'>')
        {
            let val_start = open_idx + 1 + tag_bytes.len() + 1;
            let close_rest = &xml_block[val_start..];
            for close_idx in 0..close_rest.len() {
                let s = close_rest.as_bytes();
                if s[close_idx..].starts_with(b"</")
                    && s[close_idx + 2..].starts_with(tag_bytes)
                    && s.get(close_idx + 2 + tag_bytes.len()) == Some(&b'>')
                {
                    return Some(&close_rest[..close_idx]);
                }
            }
            return None;
        }
        start = open_idx + 1;
    }
    None
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
        let storage = StorageService::new("test-bucket")
            .with_credentials(
                "test_access_key",
                "test_secret_key",
                "us-east-1",
                Some("http://rustfs:9000"),
            )
            .with_public_endpoint(Some("http://localhost:9000"));

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

        // Verify the signature calculation used the public endpoint host (localhost:9000)
        let direct_url = generate_presigned_url(
            "GET",
            "test-bucket",
            object_key,
            expires_in,
            "test_access_key",
            "test_secret_key",
            "us-east-1",
            Some("http://localhost:9000"),
        )
        .unwrap();

        let extract_sig = |u: &str| {
            u.split("X-Amz-Signature=")
                .nth(1)
                .unwrap_or_default()
                .to_string()
        };

        assert_eq!(extract_sig(&url), extract_sig(&direct_url));
    }

    #[tokio::test]
    async fn test_generate_presigned_put_url() {
        let storage = StorageService::new("test-bucket")
            .with_credentials(
                "test_access_key",
                "test_secret_key",
                "us-east-1",
                Some("http://rustfs:9000"),
            )
            .with_public_endpoint(Some("http://localhost:9000"));

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

        // Verify the signature calculation used the public endpoint host (localhost:9000)
        let direct_url = generate_presigned_url(
            "PUT",
            "test-bucket",
            object_key,
            expires_in,
            "test_access_key",
            "test_secret_key",
            "us-east-1",
            Some("http://localhost:9000"),
        )
        .unwrap();

        let extract_sig = |u: &str| {
            u.split("X-Amz-Signature=")
                .nth(1)
                .unwrap_or_default()
                .to_string()
        };

        assert_eq!(extract_sig(&url), extract_sig(&direct_url));
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

    #[test]
    fn test_parse_list_objects_v2_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
            <Name>test-bucket</Name>
            <Prefix>tmp/</Prefix>
            <Contents>
                <Key>tmp/item1.png</Key>
                <LastModified>2025-01-01T12:00:00.000Z</LastModified>
                <Size>12345</Size>
            </Contents>
            <Contents>
                <Key>tmp/item2.png</Key>
                <LastModified>2025-01-02T15:30:00.000Z</LastModified>
                <Size>67890</Size>
            </Contents>
        </ListBucketResult>"#;

        let objects = parse_list_objects_v2_xml(xml);
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].key, "tmp/item1.png");
        assert_eq!(objects[0].size, 12345);
        assert_eq!(objects[1].key, "tmp/item2.png");
        assert_eq!(objects[1].size, 67890);
    }

    #[test]
    fn test_rewrite_url_with_public_endpoint() {
        let url = "http://minio.internal:9000/bucket/key.png?sig=123".to_string();
        let rewritten = rewrite_url_with_public_endpoint(
            url.clone(),
            Some("http://minio.internal:9000/"),
            Some("https://s3.public.com/"),
        );
        assert_eq!(rewritten, "https://s3.public.com/bucket/key.png?sig=123");
    }
}
