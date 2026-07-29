pub mod types;
pub mod naming;
pub mod identity;
pub mod overlay;
pub mod providers;
pub mod registry;
pub mod store;
pub mod query;
pub mod api_client;
pub mod manager;

// reexpose SatManager
pub use manager::SatManager;