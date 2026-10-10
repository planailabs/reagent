//! A task's view of reagent's own source (built into the binary).

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;

use crate::app::App;

#[derive(Clone)]
pub struct SourceTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Ls {
    /// A folder of the repository (default: the top), like crates/tools/src.
    #[serde(default)]
    pub path: String,
    /// Every file under it, not only its entries.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Read {
    /// A file of the repository, like crates/tools/src/app.rs or DESIGN.md.
    pub path: String,
    /// The first line (default 1).
    pub from: Option<usize>,
    /// The last line (default: 400 lines on).
    pub to: Option<usize>,
}

#[tool_router(server_handler)]
impl SourceTools {
    #[tool(description = "List reagent's own source (as this running build has it): a folder's entries, or every file under it. DESIGN.md says how it's built; crates/ holds the Rust, webui/ the web UI.")]
    async fn reagent_source_ls(&self, Parameters(a): Parameters<Ls>) -> Result<String, String> {
        crate::source::ls(&a.path, a.recursive)
    }

    #[tool(description = "Read a file of reagent's own source (as this running build has it), numbered lines, 400 at a time.")]
    async fn reagent_source_read(&self, Parameters(a): Parameters<Read>) -> Result<String, String> {
        crate::source::read(&a.path, a.from, a.to)
    }
}
