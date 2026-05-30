//! # chafa-syms-rs
//!
//! A pure-Rust port of the **symbol-rendering core** of [chafa](https://hpjansson.org/chafa/):
//! turning a raster image into a grid of terminal character cells, where each cell picks the
//! Unicode symbol + foreground/background colors that best reconstruct that cell's pixels.
//!
//! The port targets *core numerical parity* with chafa 1.19.0: given an identical input pixel
//! grid, the chosen symbol and colors per cell match chafa bit-for-bit (sRGB color space only).
//!
//! See `devdocs/PLAN.md` for the full design and the C-source map.

pub mod color;
pub mod geometry;
pub mod palette;
pub mod select;
pub mod symbol;
pub mod symbol_map;
pub mod work_cell;

pub use color::{color_diff, Color, ColorPair, COLOR_PAIR_BG, COLOR_PAIR_FG};
pub use select::{render_cells, CanvasMode, CellOut, RenderConfig};
pub use symbol::{Symbol, SymbolTags, WideSymbol};
pub use symbol_map::{Candidate, Selector, SymbolMap};
pub use work_cell::WorkCell;
