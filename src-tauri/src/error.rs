use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Every failure the frontend can observe.
///
/// Serialised as `{ kind, message }` so the UI can branch on `kind`
/// (notably `toolMissing`, which routes to the setup screen) while still
/// having something human-readable to show.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("`{tool}` was not found on your PATH. Install ffmpeg, then restart Whittle.")]
    ToolMissing { tool: &'static str },

    #[error("{tool} exited with {code}: {stderr}")]
    ToolFailed {
        tool: &'static str,
        code: String,
        stderr: String,
    },

    #[error("could not understand {tool} output: {detail}")]
    Unparsable {
        tool: &'static str,
        detail: String,
    },

    #[error("{0}")]
    Io(String),

    #[error("{0}")]
    Invalid(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::ToolMissing { .. } => "toolMissing",
            AppError::ToolFailed { .. } => "toolFailed",
            AppError::Unparsable { .. } => "unparsable",
            AppError::Io(_) => "io",
            AppError::Invalid(_) => "invalid",
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::Invalid(msg.into())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}
