//! The plugin `format-docx` as a native plugin file (see `../../README.md`).

#![no_std]
#![no_main]

paperpaw_sdk::plugin!(paperpaw_plugin_format_docx::DocxFormat);
