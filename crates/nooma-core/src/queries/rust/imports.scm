; `use` declarations, recorded as the path as written.
;
; The whole argument is captured rather than its leaves: `use std::io::{Read,
; Write}` is one dependency on `std::io`, and splitting it into two would
; overstate what the file depends on.

(use_declaration argument: (_) @module)

; `extern crate` still appears in older code and in macro-heavy crates.
(extern_crate_declaration name: (identifier) @module)
