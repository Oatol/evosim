//! evosim-core —— 模拟内核（与 JS 版逐位对齐）。
//!
//! 零窗口依赖，可以在 `cargo test` 里直接跑，也可以被 CLI / 渲染层 / 界面层复用。

pub mod anatomy;
pub mod blueprint;
pub mod brain;
pub mod cell_type;
pub mod config;
pub mod factory;
pub mod fields;
pub mod grid;
pub mod jsmath;
pub mod mutation;
pub mod organism;
pub mod rng;
pub mod save;
pub mod sensors;
pub mod species;
pub mod world;
pub mod worldgen;

pub use config::Config;
pub use rng::Rng;
pub use world::World;

/// 初始化 rayon 线程池。
///
/// **按物理核数**（逻辑核数的一半）而不是逻辑核数：实测 1500 个体的稳态下，
/// 8 线程 0.415 ms/tick，但用满全部逻辑核反而退化到 0.483 —— 超线程在
/// 这种"每 tick 一小段并行"的负载上只会增加调度与缓存竞争。
pub fn init_threads() {
    let logical = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(4);
    let physical = (logical / 2).max(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(physical)
        .build_global();
}
