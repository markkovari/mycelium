//! Skill component loader. Reads a `SkillManifest`, returns the raw wasm
//! bytes either from a local disk cache or by fetching from the declared
//! source. Verifies SHA-256 when the manifest pins one.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

use crate::manifest::{SkillManifest, SkillSource};

#[derive(Clone)]
pub struct Loader {
    cache_dir: PathBuf,
}

impl Loader {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let cache_dir = cache_dir.into();
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create skill cache dir {}", cache_dir.display()))?;
        Ok(Self { cache_dir })
    }

    pub async fn fetch(
        &self,
        manifest: &SkillManifest,
        js: &async_nats::jetstream::Context,
    ) -> Result<Vec<u8>> {
        let cache_key = cache_key_for(manifest);
        let cache_path = self.cache_dir.join(&cache_key);

        if cache_path.exists() {
            let bytes = tokio::fs::read(&cache_path)
                .await
                .with_context(|| format!("read cached {}", cache_path.display()))?;
            if hash_ok(manifest, &bytes) {
                tracing::debug!(skill = %manifest.name, "cache hit");
                return Ok(bytes);
            }
            tracing::warn!(skill = %manifest.name, "cache hash mismatch; refetching");
            let _ = tokio::fs::remove_file(&cache_path).await;
        }

        let bytes = match &manifest.source {
            SkillSource::Oci { image, .. } => fetch_oci(image).await?,
            SkillSource::NatsObjectStore { bucket, key, .. } => {
                fetch_nats_object(js, bucket, key).await?
            }
            SkillSource::File { path, .. } => fetch_local(Path::new(path)).await?,
        };

        if !hash_ok(manifest, &bytes) {
            return Err(anyhow!(
                "sha256 mismatch for skill {} {}",
                manifest.name,
                manifest.version
            ));
        }

        if let Some(parent) = cache_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&cache_path, &bytes)
            .await
            .with_context(|| format!("write cache {}", cache_path.display()))?;
        Ok(bytes)
    }
}

fn cache_key_for(manifest: &SkillManifest) -> String {
    // Stable per (name, version, source). Avoid embedding the OCI image
    // string verbatim — slashes confuse the filename.
    let mut hasher = Sha256::new();
    hasher.update(manifest.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(manifest.version.as_bytes());
    hasher.update(b"\0");
    match &manifest.source {
        SkillSource::Oci { image, .. } => {
            hasher.update(b"oci\0");
            hasher.update(image.as_bytes());
        }
        SkillSource::NatsObjectStore { bucket, key, .. } => {
            hasher.update(b"obj\0");
            hasher.update(bucket.as_bytes());
            hasher.update(b"\0");
            hasher.update(key.as_bytes());
        }
        SkillSource::File { path, .. } => {
            hasher.update(b"file\0");
            hasher.update(path.as_bytes());
        }
    }
    format!("{}.wasm", hex::encode(hasher.finalize()))
}

fn hash_ok(manifest: &SkillManifest, bytes: &[u8]) -> bool {
    let expected: Option<&str> = match &manifest.source {
        SkillSource::Oci { sha256, .. } => sha256.as_deref(),
        SkillSource::NatsObjectStore { sha256, .. } => Some(sha256.as_str()),
        SkillSource::File { sha256, .. } => sha256.as_deref(),
    };
    let Some(expected) = expected else {
        // Manifest opted out of pinning. OCI tag immutability is already a
        // weak guarantee; we accept the artifact.
        return true;
    };
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    actual.eq_ignore_ascii_case(expected)
}

async fn fetch_oci(image: &str) -> Result<Vec<u8>> {
    use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};

    let reference: Reference = image
        .parse()
        .with_context(|| format!("parse OCI reference {image}"))?;
    let client = Client::new(ClientConfig::default());
    // Anonymous pull. If callers need creds, they can set the docker config
    // and we'd extend this later — out of scope for now.
    let image = client
        .pull(
            &reference,
            &RegistryAuth::Anonymous,
            vec!["application/wasm", "application/vnd.wasmcloud.component.v1+wasm"],
        )
        .await
        .with_context(|| format!("pull {image}"))?;
    let layer = image
        .layers
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("OCI image {reference} has no layers"))?;
    Ok(layer.data)
}

async fn fetch_nats_object(
    js: &async_nats::jetstream::Context,
    bucket: &str,
    key: &str,
) -> Result<Vec<u8>> {
    let store = js
        .get_object_store(bucket)
        .await
        .with_context(|| format!("open NATS Object Store {bucket}"))?;
    let mut obj = store
        .get(key)
        .await
        .with_context(|| format!("get {bucket}/{key}"))?;
    let mut buf = Vec::new();
    tokio::io::copy(&mut obj, &mut buf)
        .await
        .with_context(|| format!("read object {bucket}/{key}"))?;
    Ok(buf)
}

async fn fetch_local(path: &Path) -> Result<Vec<u8>> {
    tokio::fs::read(path)
        .await
        .with_context(|| format!("read local skill {}", path.display()))
}
