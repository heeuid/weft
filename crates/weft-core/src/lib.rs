//! weft without its window: the terminal grid and its parser, ptys, hosts
//! and the operations they serve (this PC directly, a remote one through
//! the agent), searching, the command records, and passwords in memory.
//! Nothing here draws; the app (the root package) and weft-draw do.

pub mod editfind;
pub mod find;
pub mod host;
pub mod pty;
pub mod records;
pub mod secret;
pub mod term;
