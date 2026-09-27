//! 升级兼容矩阵测试台（计划 051）。
//!
//! 矩阵单元在编译期由 `anchors.json` 与 `expectations.json` 展开为具名测试；每个单元由
//! [`run_cell`] 持有一个 `uc_testkit::Scenario`，经连接测试宿主驱动各版本的公开 Engine 操作。
//! 进程调度、分组与超时由 nextest 负责，本 crate 不重试。

mod catalog;
mod cell;
mod device;
mod dimensions;
mod fixture;
mod host;
mod interop;
mod rendezvous;

pub use cell::run_cell;
