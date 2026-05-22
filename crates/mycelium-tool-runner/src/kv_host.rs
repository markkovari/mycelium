//! wasi:keyvalue/store host backed by NATS JetStream KV.
//!
//! Defines `NatsBucket`, calls `wasmtime::component::bindgen!` to generate the
//! `Host` / `HostBucket` traits, and exports `add_to_linker` so sandbox.rs can
//! wire the interface into a `Linker<SkillCtx>`.

use anyhow::Result;
use async_nats::jetstream;
use wasmtime::component::Linker;

/// Backing state for a wasi:keyvalue bucket resource.
pub struct NatsBucket {
    pub kv: jetstream::kv::Store,
}

wasmtime::component::bindgen!({
    path: "wit",
    world: "kv-host",
    async: true,
    with: {
        "wasi:keyvalue/store/bucket": NatsBucket,
    },
});

/// Wire wasi:keyvalue/store into `linker`.  `T` must implement
/// `wasi::keyvalue::store::Host` + `HostBucket` (i.e. `SkillCtx` after
/// sandbox.rs implements those traits).
pub fn add_to_linker<T>(linker: &mut Linker<T>) -> Result<()>
where
    T: wasi::keyvalue::store::Host + wasi::keyvalue::store::HostBucket + Send + 'static,
{
    wasi::keyvalue::store::add_to_linker(linker, |ctx| ctx)
}
