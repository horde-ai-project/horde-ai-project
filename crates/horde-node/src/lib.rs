//! Physical-server agent: passphrase keystore, resource caps, stub GPU executor.
//!
//! Phase A runs without a real GPU by default (`StubExecutor`). CUDA backends can
//! be wired later behind feature flags; CI stays green without accelerators.

mod executor;
mod keystore;
mod server;

pub use executor::{GpuExecutor, StubExecutor};
pub use keystore::{EncryptedKeystore, IdentityKeys, KeystoreError};
pub use server::{PhysicalServer, ServerSnapshot};
