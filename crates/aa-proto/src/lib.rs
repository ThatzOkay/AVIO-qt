// protobuf `oneof`s routinely generate enums with wildly different variant sizes; that's
// expected for wire-format messages, not something to box away.
#![allow(clippy::large_enum_variant)]

include!(concat!(env!("OUT_DIR"), "/mod_tree.rs"));
