//! Wasm component loader with OCI digest-aware caching.
//!
//! Cache key derivation:
//!   * `sha256` in manifest → use it directly. Stable across image-tag rotations.
//!   * OCI source without `sha256` → resolve the registry manifest digest at
//!     load time (cheap HEAD) and key on `(name, version, "oci", digest)`. This
//!     fixes the prior bug where `:dev` rebuilds kept serving stale bytes.
//!   * Nats object store + file sources without sha256 → key on the location
//!     string; refetch only when the location string changes (operator's
//!     responsibility for now).

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use async_nats::jetstream::Context as JsContext;
use sha2::{Digest, Sha256};

use crate::manifest::{ComponentManifest, ComponentSource};

#[derive(Clone, Debug)]
pub struct LoaderConfig {
    pub cache_dir: PathBuf,
}

#[derive(Clone)]
pub struct Loader {
    config: LoaderConfig,
}

impl Loader {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let cache_dir = cache_dir.into();
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create wasm cache dir {}", cache_dir.display()))?;
        Ok(Self {
            config: LoaderConfig { cache_dir },
        })
    }

    pub async fn fetch(
        &self,
        manifest: &ComponentManifest,
        js: &JsContext,
    ) -> Result<Vec<u8>> {
        let cache_key = self.cache_key_for(manifest).await?;
        let cache_path = self.config.cache_dir.join(&cache_key);

        if cache_path.exists() {
            let bytes = tokio::fs::read(&cache_path)
                .await
                .with_context(|| format!("read cached {}", cache_path.display()))?;
            if hash_ok(manifest, &bytes) {
                tracing::debug!(component = %manifest.name, "cache hit");
                return Ok(bytes);
            }
            tracing::warn!(component = %manifest.name, "cache hash mismatch; refetching");
            let _ = tokio::fs::remove_file(&cache_path).await;
        }

        let bytes = match &manifest.source {
            ComponentSource::Oci { image, .. } => fetch_oci(image).await?,
            ComponentSource::NatsObjectStore { bucket, key, .. } => {
                fetch_nats_object(js, bucket, key).await?
            }
            ComponentSource::File { path, .. } => fetch_local(std::path::Path::new(path)).await?,
        };

        if !hash_ok(manifest, &bytes) {
            return Err(anyhow!(
                "sha256 mismatch for component {} {}",
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

    async fn cache_key_for(&self, manifest: &ComponentManifest) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(manifest.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(manifest.version.as_bytes());
        hasher.update(b"\0");
        match &manifest.source {
            ComponentSource::Oci { image, sha256 } => {
                hasher.update(b"oci\0");
                if let Some(sha) = sha256.as_deref().filter(|s| !s.is_empty()) {
                    hasher.update(b"pinned-sha\0");
                    hasher.update(sha.as_bytes());
                } else {
                    // Resolve the registry manifest digest. Cheap (HEAD-ish)
                    // and gives us a stable cache key that changes when the
                    // remote bytes change, even under mutable tags like :dev.
                    let digest = resolve_oci_digest(image).await?;
                    hasher.update(b"digest\0");
                    hasher.update(digest.as_bytes());
                }
            }
            ComponentSource::NatsObjectStore { bucket, key, sha256 } => {
                hasher.update(b"obj\0");
                hasher.update(bucket.as_bytes());
                hasher.update(b"\0");
                hasher.update(key.as_bytes());
                hasher.update(b"\0");
                hasher.update(sha256.as_bytes());
            }
            ComponentSource::File { path, sha256 } => {
                hasher.update(b"file\0");
                hasher.update(path.as_bytes());
                if let Some(sha) = sha256.as_deref() {
                    hasher.update(b"\0");
                    hasher.update(sha.as_bytes());
                }
            }
        }
        Ok(format!("{}.wasm", hex::encode(hasher.finalize())))
    }
}

fn hash_ok(manifest: &ComponentManifest, bytes: &[u8]) -> bool {
    let expected: Option<&str> = match &manifest.source {
        ComponentSource::Oci { sha256, .. } => sha256.as_deref(),
        ComponentSource::NatsObjectStore { sha256, .. } => Some(sha256.as_str()),
        ComponentSource::File { sha256, .. } => sha256.as_deref(),
    };
    let Some(expected) = expected.filter(|s| !s.is_empty()) else {
        // Manifest didn't pin. Trust the OCI digest (already used as cache key)
        // or the operator (for unpinned File/NatsObjectStore sources).
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
    let image_pull = client
        .pull(
            &reference,
            &RegistryAuth::Anonymous,
            vec!["application/wasm", "application/vnd.wasmcloud.component.v1+wasm"],
        )
        .await
        .with_context(|| format!("pull {image}"))?;
    let layer = image_pull
        .layers
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("OCI image {reference} has no layers"))?;
    Ok(layer.data)
}

async fn resolve_oci_digest(image: &str) -> Result<String> {
    use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};

    let reference: Reference = image
        .parse()
        .with_context(|| format!("parse OCI reference {image}"))?;
    let client = Client::new(ClientConfig::default());
    let digest = client
        .fetch_manifest_digest(&reference, &RegistryAuth::Anonymous)
        .await
        .with_context(|| format!("fetch manifest digest for {image}"))?;
    Ok(digest)
}

async fn fetch_nats_object(
    js: &JsContext,
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

async fn fetch_local(path: &std::path::Path) -> Result<Vec<u8>> {
    tokio::fs::read(path)
        .await
        .with_context(|| format!("read local wasm {}", path.display()))
}
