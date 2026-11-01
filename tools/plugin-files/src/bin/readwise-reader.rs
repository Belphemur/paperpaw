//! The plugin `readwise-reader` as a native plugin file (see `../../README.md`).

#![no_std]
#![no_main]

paperpaw_sdk::plugin!(paperpaw_plugin_readwise_reader::ReadwiseReader);
