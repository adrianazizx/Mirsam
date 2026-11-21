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
        use safha_core::Error as E;
        let code = match &err {
            E::PdfiumNotFound { .. } | E::PdfiumLoad { .. } => "engine_unavailable",
            E::PasswordRequired => "password_required",
            E::InvalidPdf => "invalid_pdf",
            E::PageOutOfRange { .. } => "page_out_of_range",
            E::InvalidRenderRequest(_) => "invalid_render_request",
            E::Io(_) => "io_error",
            E::Pdfium(_) => "engine_error",
        };
        Self::new(code, err.to_string())
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
