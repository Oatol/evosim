//! evosim-core —— 模拟内核（与 JS 版逐位对齐）。
//!
//! 零窗口依赖，可以在 `cargo test` 里直接跑，也可以被 CLI / 渲染层 / 界面层复用。

pub mod anatomy;
pub mod brain;
pub mod cell_type;
pub mod config;
pub mod factory;
pub mod grid;
pub mod jsmath;
pub mod mutation;
pub mod organism;
pub mod rng;
pub mod sensors;
pub mod species;
pub mod world;
pub mod worldgen;

pub use config::Config;
pub use rng::Rng;
pub use world::World;
