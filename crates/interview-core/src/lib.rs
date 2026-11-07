pub mod background;
pub mod background_settings;
pub mod background_versions;
pub mod recovery;
pub mod cloud_asr;
pub mod index;
pub mod model;
pub mod import_control;
pub mod document_meta;
pub mod facts;
pub mod conflicts;
pub mod structured;
pub mod credentials;
pub mod request_config;
pub use request_config::RequestConfig;
pub mod context_budget;
pub mod request_budget;
pub mod session_memory;
pub mod compaction;
pub mod memory_store;
pub mod model_capabilities;
pub use model_capabilities::*;
pub mod providers;
pub use providers::*;
pub mod vision;
pub use vision::*;
pub mod connection_probe;
pub use connection_probe::*;
pub use credentials::*;
pub use index::*;
pub use model::*;
pub use import_control::*;
pub use document_meta::*;
pub use facts::*;
pub use conflicts::*;
pub use structured::*;
pub type Result<T> = std::result::Result<T, String>;

pub mod preferences;
pub use preferences::{InterviewPreferences, ConversationScenario};

pub mod source_policy;
pub mod toolkit_policy;

pub mod assertions;

pub mod background_dependencies;

pub mod preference_sources;

pub mod alias_sources;
pub mod preparation;
