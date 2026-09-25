//! 升级兼容矩阵：每个单元一个具名测试，由 nextest 按名称过滤、分片与设定期限。
//! 运行入口：`bash scripts/testing/run-test-group.sh upgrade-matrix [--smoke] [nextest 参数]`。

include!(concat!(env!("OUT_DIR"), "/cells.rs"));
