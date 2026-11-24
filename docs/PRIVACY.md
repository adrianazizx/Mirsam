# Privacy

Mirsam is a desktop app that works on your computer. It has **no accounts, no telemetry, no
analytics and no ads**, and its makers receive nothing about you or your documents.

## Your documents

The files you open, edit, sign or OCR stay on your computer. Mirsam does all of this locally,
OCR included. It keeps its settings, recent files and recovery copies of unsaved work in its own
folder on your computer.

## When Mirsam connects to the internet

Only when you ask it to:

| You choose to… | Mirsam contacts | What it sends |
|---|---|---|
| Download OCR models or the OCR runtime | `huggingface.co`, `github.com` and `raw.githubusercontent.com`, which host those files | An ordinary download request. Nothing from your documents. |
| Add a trusted time-stamp to a certificate signature (off unless you turn it on) | The time-stamping server named in the signing dialog (suggested: DigiCert's, which you can change) | A hash of the signature, a fixed-length fingerprint from which the document cannot be read. |
| Open a web link in a document | Your web browser opens it, after Mirsam shows you the address | Whatever that website receives from any browser |
| Report a crash on GitHub | GitHub, in your browser, with a pre-filled issue you can edit before you post it | Only what you post |

As with any download, the servers above see your IP address and the time of the request, under
their own privacy policies.

On Windows, if the system's WebView2 component is missing (it comes with Windows 11 and most
Windows 10 computers), the setup program downloads it from Microsoft.

## Crash reports

Crash reports are off unless you agree to them, and they stay on your computer. A report holds
no document contents, file names or paths. Mirsam never sends one: you can copy it, save it as a
file, or open a pre-filled GitHub issue, and you can delete it.

## Changes

This statement lives in Mirsam's repository, so every change to it is public in its history.
Questions: [open an issue](https://github.com/adrianazizx/Mirsam/issues).
