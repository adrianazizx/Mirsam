use serde::Serialize;

/// The error shape every command returns to the UI. `code` is a stable
/// identifier the UI maps to a localized message; `detail` is for logs only
/// and is never shown to the user as-is.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub detail: String,
}

impl CommandError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }

    pub fn unknown_document(id: u32) -> Self {
        Self::new("unknown_document", format!("no open document with id {id}"))
    }
}

impl From<safha_core::Error> for CommandError {
    fn from(err: safha_core::Error) -> Self {
        Self::new(engine_error_code(&err), err.to_string())
    }
}

impl From<safha_core::render::RenderError> for CommandError {
    fn from(err: safha_core::render::RenderError) -> Self {
        use safha_core::render::RenderError as E;
        match err {
            // Expected during scrolling; the UI drops these silently.
            E::Cancelled => Self::new("cancelled", err.to_string()),
            E::UnknownDocument(_) => Self::new("unknown_document", err.to_string()),
            E::Engine(engine) => Self::new(engine_error_code(&engine), engine.to_string()),
        }
    }
}

fn engine_error_code(err: &safha_core::Error) -> &'static str {
    use safha_core::Error as E;
    match err {
        E::PdfiumNotFound { .. } | E::PdfiumLoad { .. } => "engine_unavailable",
        E::PasswordRequired => "password_required",
        E::InvalidPdf => "invalid_pdf",
        E::PageOutOfRange { .. } => "page_out_of_range",
        E::InvalidRenderRequest(_) => "invalid_render_request",
        E::EditUnsupported(_) => "edit_unsupported",
        E::InvalidEdit(_) => "invalid_edit",
        E::Io(_) => "io_error",
        E::Pdfium(_) => "engine_error",
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
