//! UI-independent host storage and OpenSSH operations.
pub mod auth;
pub mod config;
pub mod ssh;
pub use auth::{Credentials, Prompt, askpass_helper, run};
pub use config::{Authentication, Host, HostStore};
pub use ssh::{Operation, command};
