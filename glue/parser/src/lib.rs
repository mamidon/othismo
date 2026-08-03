//! Glue parser: tokens in, lossless syntax tree plus diagnostics out.
//!
//! Nothing here yet — the tree representation is the next thing to design.
//! Two properties it has to have, whatever shape it takes:
//!
//! * **Lossless.** Every byte of the source is reachable from the tree,
//!   trivia included, so the same tree can serve formatting later.
//! * **Total.** Parsing never fails. Malformed input produces a tree with
//!   error nodes and a list of diagnostics, because the editor will spend most
//!   of its time looking at half-typed programs.
