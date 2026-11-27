mod client;
pub mod saved;

pub use client::{Config, Incoming, Link, Me, Offline, config, dial, drop_msg, watch};
use client::{Handle, room_of};
