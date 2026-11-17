//! Collective (logical node) head: member registry, invite mint/redeem, job scheduling.
//!
//! Admin control plane stays local; members are physical servers joined via invite tokens.
//! Optional invite-bounded mesh (Phase B) attaches without exposing admin on the overlay.

mod head;
mod scheduler;

pub use head::{CollectiveHead, CollectiveStatus, MeshJoinRequest, MintInviteRequest};
pub use scheduler::pick_server;
