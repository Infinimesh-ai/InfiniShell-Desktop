use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PaletteMode {
    Command,
    Navigation,
    LaunchConfig,
    #[serde(alias = "ZapDrive")]
    InfiniShellDrive,
    Files,
    Conversations,
}
