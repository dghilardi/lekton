#[cfg(feature = "ssr")]
pub mod analyzer;
#[cfg(feature = "ssr")]
pub mod cached_embedding;
#[cfg(feature = "ssr")]
pub mod chat;
#[cfg(feature = "ssr")]
pub mod embedding;
#[cfg(feature = "ssr")]
pub mod eval;
#[cfg(feature = "ssr")]
pub mod hyde;
#[cfg(feature = "ssr")]
pub mod provider;
#[cfg(feature = "ssr")]
pub mod query_rewriter;
#[cfg(feature = "ssr")]
pub mod reindex;
#[cfg(feature = "ssr")]
pub mod reranker;
#[cfg(feature = "ssr")]
pub mod rrf;
#[cfg(feature = "ssr")]
pub mod service;
#[cfg(feature = "ssr")]
pub mod splitter;
#[cfg(feature = "ssr")]
mod splitter_blocks;
#[cfg(feature = "ssr")]
mod splitter_sections;
#[cfg(feature = "ssr")]
mod splitter_table;
#[cfg(feature = "ssr")]
pub mod vectorstore;

#[cfg(feature = "ssr")]
pub mod client;
#[cfg(feature = "ssr")]
pub use client::build_oai_client;
#[cfg(feature = "ssr")]
pub use provider::LlmProvider;
