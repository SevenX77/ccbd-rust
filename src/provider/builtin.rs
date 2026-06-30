//! Built-in ah rule resources embedded into the ccbd binary.

pub const MASTER_KERNEL: &str = include_str!("../../assets/builtin/master_kernel.md");
pub const WORKER_KERNEL: &str = include_str!("../../assets/builtin/worker_kernel.md");
pub const DEFAULT_MASTER: &str = include_str!("../../assets/builtin/defaults/master.md");
pub const DEFAULT_WORKER: &str = include_str!("../../assets/builtin/defaults/worker.md");
